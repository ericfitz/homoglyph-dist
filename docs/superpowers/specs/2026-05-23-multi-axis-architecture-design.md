# Design: sqdist multi-axis architecture (v0.3.0)

Date: 2026-05-23
Status: Approved
Component: `sqdist` — ground-up refactor of the scoring core into an extensible
multi-axis panel. Splits `src/main.rs` into modules.

## Summary

Replace the single blended scoring model (weighted-homoglyph Damerau + a fixed
`Scores` struct) with an **extensible panel of independent axes**. Each axis is
a self-contained similarity signal computed from a shared per-pair context; the
verdict and output read a uniform axis interface. This is the architectural
foundation for later research-heavy axes (mixed-script count, keyboard distance,
CJK pseudo-homoglyphs, digraph↔char mappings, curated supplemental confusables),
each of which will be its own follow-up spec — **out of scope here.**

This is a **breaking change** to the JSON/output contract (new keys, removed
fields, removed `--hogl-weight`). Pre-1.0, single owner, breaking changes
approved → released as **v0.3.0**.

## Motivation

The v0.2.0 model blends homoglyph-awareness into a weighted Damerau (`hogl`) and
exposes derived 0–1 scores. That conflates distinct signals. The redesign
evaluates each axis **independently and raw**, so the verdict (which already
works by comparing scores) can reason over a full panel, and new signals can be
added without touching the core. It also fixes a conceptual muddle: "homoglyph
involvement" is better expressed as explicit confusable counts than as a
fractional edit weight.

## Core abstraction

An **axis** is one independent similarity signal for a string pair.

```rust
enum AxisValue {
    Int(u64),
    Float(f64),
    Bool(bool),
}

enum Direction {
    HigherMoreSimilar,
    HigherMoreDifferent,
}

trait Axis {
    fn key(&self) -> &'static str;          // stable JSON key / human label
    fn direction(&self) -> Direction;        // how to read the value
    fn compute(&self, ctx: &PairContext) -> AxisValue;
}
```

- A static registry `ALL_AXES: &[&dyn Axis]` lists every axis **in canonical
  order**. This order defines emit order (human rows and JSON keys) and is the
  single source of truth for field names — it **replaces the v0.2.0 `Field`
  enum** entirely.
- `--fields <comma-list>` validates each name against axis keys (error naming
  the offender + listing valid keys, exit 2), and emits the selected axes in
  registry order. Identifier keys (`a`/`b` or `input`/`match`) are always
  emitted and are not axes.
- `Direction` is carried for the verdict / future ranking to reason generically;
  it is not emitted in output.

### PairContext (shared precomputation)

Built once per pair, passed by reference to every `compute()`:

```rust
struct PairContext<'a> {
    a: &'a str,
    b: &'a str,
    ca: Vec<char>,          // a.chars().collect()
    cb: Vec<char>,          // b.chars().collect()
    ska: String,            // skeleton(a)
    skb: String,            // skeleton(b)
    sva: Vec<char>,         // ska.chars().collect()
    svb: Vec<char>,         // skb.chars().collect()
    // alignment traceback for uts39_confusable_count (computed once)
    align: Vec<AlignOp>,    // edit operations a->b (Match/Sub/Ins/Del)
}
```

Axes are pure functions of the context; expensive shared data (skeletons, the
Damerau alignment) is computed once.

## Two-phase panel computation

1. **Base axes** — pure functions of `PairContext`.
2. **Derived axes** — functions of already-computed base `AxisValue`s.

The panel builder runs phase 1 over the base registry, collecting results into a
key→`AxisValue` map (preserving order), then runs phase 2 derived axes which
read that map. Derived inputs are guaranteed present because the base distances
always run. (Implementation note: base and derived axes may be two registries,
or one registry with a `phase()` on the trait — implementer's choice, as long as
canonical order across both is well-defined and stable.)

### The v0.3.0 axis panel

| Key | Type | Direction | Phase | Definition |
|---|---|---|---|---|
| `equal` | Bool | — | base | `a == b` |
| `levenshtein` | Int | HigherMoreDifferent | base | unweighted Levenshtein on `ca`/`cb` |
| `damerau` | Int | HigherMoreDifferent | base | unweighted Damerau (OSA) on `ca`/`cb` |
| `skeleton_levenshtein` | Int | HigherMoreDifferent | base | Levenshtein on `sva`/`svb` |
| `skeleton_damerau` | Int | HigherMoreDifferent | base | Damerau on `sva`/`svb` |
| `uts39_confusable_count` | Int | HigherMoreSimilar | base | substitution positions in the Damerau alignment whose two chars are UTS#39-confusable |
| `uts39_skeleton_delta` | Int | HigherMoreSimilar | derived | `damerau - skeleton_damerau` (edits that vanish under skeletonization) |
| `confusable_only` | Bool | — | derived | `!equal && skeleton_levenshtein == 0` |

Notes:
- `equal` and `confusable_only` are booleans; `Direction` is irrelevant for them
  (use a sensible default; they are not valid `--metric` targets).
- `uts39_confusable_count` and `uts39_skeleton_delta` are intentionally BOTH
  kept for this release — they measure confusable involvement two different ways
  (literal aligned-position count vs. skeleton-collapse delta). They are an
  exploration aid; one may be dropped in a later release once their behavior on
  real inputs is compared. Document both as "experimental, may change".
- `uts39_skeleton_delta` could in principle be negative if skeletonization
  expands length such that `skeleton_damerau > damerau`. Since the type is `u64`,
  define it as `damerau.saturating_sub(skeleton_damerau)` (clamp at 0). The
  verdict's `homoglyph_share` uses this saturating value.

### Removed (breaking)

- `homoglyph_damerau` (the weighted `hogl` metric) — gone.
- `normalized`, `skeleton_normalized` — gone.
- `--hogl-weight` flag — removed.
- The `Field` enum — replaced by the axis registry.

## Alignment traceback (for uts39_confusable_count)

The Damerau DP already computes the cost matrix. To count confusable
*substitutions*, add a traceback that walks the matrix from `(n,m)` to `(0,0)`,
emitting `AlignOp::{Match, Sub(i,j), Ins, Del, Transpose}`. `uts39_confusable_count`
counts `Sub(i,j)` ops where `confusable(ca[i], cb[j])` is true. Computed once and
stored in `PairContext.align`. The traceback is standard; ties broken
deterministically (prefer substitution/match on diagonal, then deletion, then
insertion) so the count is stable.

## Verdict (re-sourced, behavior preserved)

`src/verdict.rs`. Keeps the v0.2.0 categories and rules, reading the new axes:

- `[IDENTICAL]` when the `equal` axis is true → "The strings are identical."
- `homoglyph_share = uts39_skeleton_delta / damerau` (0 when `damerau == 0`).
- `[LIKELY SPOOF]` when `confusable_only` axis is true, OR
  (`homoglyph_share > 0.5` AND `max(len_a, len_b) >= 3` AND
  `len_diff_ratio <= len_tolerance`), where
  `len_diff_ratio = |len_a - len_b| / max(len_a, len_b, 1)`.
- `[LIKELY BENIGN]` otherwise: `damerau <= 2` → "likely a typo" (note minor
  homoglyph involvement when `skeleton_damerau < damerau`); `damerau >= 3` →
  "appear unrelated".
- Sentences use the proper pluralization helper ("1 edit" / "N edits").
- Single-pair human output only (NOT `-j` JSON, NOT batch).
- `--len-tolerance` (default 0.25, validated to `[0,1]`) unchanged.

This is behavior-equivalent to v0.2.0 because `uts39_skeleton_delta` equals the
old `(damerau - skeleton_damerau)` numerator and `confusable_only` has the same
meaning.

## CLI

Unchanged except:
- `--metric <axis-key>`: generalized to accept ANY numeric axis key (Int/Float).
  Bool axes (`equal`, `confusable_only`) are rejected with a clear error listing
  valid numeric axis keys. Default `skeleton_damerau`. Drives `-t` and `--sort`.
- `--hogl-weight` removed (error as unknown flag).
- `--fields` validates against axis keys (see above).

Preserved verbatim: `-t`/threshold semantics (single-pair exit code; batch
filter + `batch_matched_ok` exit codes), `--stdin`, `--string`/`--list`,
`--sort`/`--top` (buffered) vs streaming default, `-j`, `-v/--version`,
`--len-tolerance`, the `input`/`match` vs `a`/`b` keying, JSONL batch output.

## File layout

Split the single `src/main.rs` into modules:

| File | Responsibility |
|---|---|
| `src/distance.rs` | `levenshtein`, `damerau`, `skeleton`, `skeleton_of`, `confusable`, the alignment traceback + `AlignOp` |
| `src/axes.rs` | `Axis` trait, `AxisValue`, `Direction`, `PairContext`, the 8 axis impls, `ALL_AXES` registry, two-phase panel builder, axis-key parsing/selection (`--fields`), metric lookup |
| `src/verdict.rs` | `Verdict` enum + `verdict()` |
| `src/confusables_data.rs` | unchanged (generated) |
| `src/main.rs` | CLI arg parsing, `Opts`, I/O, mode dispatch, output formatting (human + JSONL), orchestration calling the panel + verdict |

Module boundaries make each axis independently testable and keep files focused
as the deferred axes are added later.

## Output format

- **JSON/JSONL**: `{"<k0>":<a>,"<k1>":<b>, <axis>:<value>, ...}` in registry
  order; identifier keys always first; booleans unquoted (`true`/`false`),
  ints unquoted, floats formatted consistently (none in the initial panel, but
  the formatter must handle `Float` for future axes — use a fixed precision,
  e.g. `{:.4}`).
- **Human (single-pair)**: one `{:<24} {value}` row per selected axis (widen the
  label column to fit the longest key `uts39_confusable_count` = 22 chars → pad
  to 24), then a blank line, then the `[CATEGORY] verdict`.
- `--fields` narrows which axes appear in both; identifier keys and the verdict
  always present (verdict only in single-pair human).

## Testing

- Per-axis unit tests: each axis's value on representative pairs + its declared
  direction. Include: `equal` true/false; `levenshtein`/`damerau` classic cases;
  skeleton variants catch `m`/`rn`; `uts39_confusable_count` counts a Cyrillic-а
  substitution and ignores a real edit; `uts39_skeleton_delta` saturates at 0;
  `confusable_only` true for GO0GLE/GOOGLE and rnicrosoft/microsoft, false for
  devflovv/devflow (the documented `vv`/`w` gap regression test carries over).
- Panel builder: two-phase ordering; derived axes read base values; canonical
  emit order matches registry.
- Alignment traceback: substitution count correct for equal- and unequal-length
  pairs; deterministic on ties.
- Verdict: all categories, behavior-equivalent to v0.2.0 (port existing tests).
- `--metric <axis>`: accepts numeric keys, rejects bool keys and unknown keys.
- `--fields`: validates against axis keys, canonical order, dedup.
- Preserved CLI: batch exit codes, streaming vs buffered, `--version`,
  `--len-tolerance` range validation, `input`/`match` keys.

## Migration / docs

- README, CLAUDE.md, and `--help` rewritten for the axis panel: new field list,
  `--metric <axis-key>`, removal of `--hogl-weight`/`normalized`, the two
  experimental `uts39_*` axes flagged as may-change.
- Note the breaking JSON-contract change prominently (downstream parsers must
  update key names).
- Version → 0.3.0 in Cargo.toml + Cargo.lock.

## Out of scope (explicitly deferred to their own specs)

- Mixed-script / Unicode-script-count axis.
- Keyboard-distance (kbd) axis (needs layout-data research).
- CJK pseudo-homoglyph axis.
- Digraph↔char / char↔digraph (math-symbol) confusables.
- Curated supplemental confusable table (`vv`→`w`, `cl`→`d`, `nn`→`m`) and any
  opt-in/opt-out flag governing non-UTS#39 data.
- Any verdict redesign that weighs the new axes (current verdict only uses the
  ported signals).
