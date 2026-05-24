# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`sqdist` — a single-binary Rust CLI that computes a **panel of independent similarity axes** across two strings for **typosquatting / homoglyph attack detection**: Levenshtein, Damerau-Levenshtein (OSA, adjacent transpositions), their UTS#39-skeleton variants, and confusable-involvement signals. The point is to separate a visual spoof (`pаypal` with Cyrillic а) — which collapses to zero skeleton distance — from a benign typo (`gogle`) that has identical unweighted distance but no confusable characters.

## Commands

```sh
cargo build --release    # -> target/release/sqdist
cargo test               # unit tests live in each module's #[cfg(test)] block; cargo test runs all (currently 76)
cargo test <name>        # run a single test, e.g. cargo test confusable_only_axis
cargo run -- <A> <B>     # run against two strings (note the -- before args)
```

There is no separate lint config; use `cargo clippy` and `cargo fmt --check`.

## Architecture

Six source files plus build-time helpers:

- [src/main.rs](src/main.rs) — CLI arg parsing (`Opts`), I/O, the three modes (single-pair / `--stdin` batch / `--string`+`--list` watchlist), output formatting (human table + JSONL), and orchestration calling the panel + verdict. The release profile (Cargo.toml) is tuned for a small fast binary (`lto`, `panic = "abort"`, `strip`).
- [src/distance.rs](src/distance.rs) — edit distances (unweighted integer `levenshtein`, `damerau` OSA), the UTS#39 skeleton model (`skeleton_of`, `skeleton`, `confusable`), and the alignment traceback (`AlignOp` + `align`).
- [src/axes.rs](src/axes.rs) — the `Axis` trait, `AxisValue` (Int/Float/Bool/NA), `Direction`, `Phase`, `PairContext` (per-pair precompute), the 10 axis impls, `ALL_AXES` registry (canonical order = single source of truth for JSON key order and emit order), the two-phase base/derived `build_panel`, and `--fields`/`--metric` parsing and validation (`parse_fields`, `validate_metric`, `metric_value`). `AxisValue::NA` renders as JSON `null` and human `n/a`; its `as_f64()` returns `None`, and `row_metric` falls back to `f64::INFINITY` so NA rows never match a finite `-t` and sort last.
- [src/keyboard.rs](src/keyboard.rs) — embedded stagger-aware US-QWERTY key-coordinate table for `keyboard_distance`; no external dependency.
- [src/verdict.rs](src/verdict.rs) — `Verdict` enum + `verdict()`, reading the panel (single-pair human output only).
- [src/confusables_data.rs](src/confusables_data.rs) — **auto-generated, do not hand-edit.** A `pub static CONFUSABLES: &[(u32, &str)]` slice (~6565 entries) sorted by code point, embedded at compile time so the binary needs no runtime data files or network.
- [build.rs](build.rs) — compile-time git SHA capture (runs `git rev-parse --short HEAD`, exposes `SQDIST_GIT_SHA` env var, falls back to "unknown" for crates.io/git-less builds).
- [scripts/gen_confusables.py](scripts/gen_confusables.py) — regenerates `confusables_data.rs` from Unicode UTS #39 `confusables.txt`. Pure stdlib (no dependencies); resolves its paths relative to the repo root, so run it from anywhere.

### Modes and structure

Three modes, dispatched by a thin `main()`:

| Mode | Trigger | Pairing | JSON keys |
|---|---|---|---|
| single pair | two positionals | the two args | `a`/`b` |
| stdin batch | `--stdin` | pre-paired tab/comma lines | `a`/`b` |
| watchlist | `--string` + `--list` | `--string` × each file line | `input`/`match` |

### The 10-axis panel (canonical order)

The `ALL_AXES` registry in `axes.rs` defines the canonical order — this controls JSON key order and human-output row order. Axes are computed in two phases: base axes are pure functions of a `PairContext`; derived axes read already-computed base axis values.

| Key | Type | Phase | Meaning |
|---|---|---|---|
| `equal` | bool | base | the two strings are byte-identical |
| `levenshtein` | int | base | min single-char insert/delete/substitute edits |
| `damerau` | int | base | like levenshtein, but an adjacent transposition (swap) counts as one edit |
| `skeleton_levenshtein` | int | base | levenshtein after reducing both strings to their UTS#39 skeletons |
| `skeleton_damerau` | int | base | damerau after reducing both to UTS#39 skeletons (~0 when visually identical, incl. multi-char confusables like m↔rn) |
| `uts39_confusable_count` | int | base | # of substitution positions in the Damerau alignment whose two chars are UTS#39-confusable **(EXPERIMENTAL, may change)** |
| `uts39_skeleton_delta` | int | derived | damerau − skeleton_damerau (saturating at 0); edits that vanish under skeletonization **(EXPERIMENTAL, may change)** |
| `confusable_only` | bool | derived | true when the strings differ but share an identical skeleton (highest-confidence spoof signal) |
| `script_restriction` | int | base | UTS#39 restriction level 0–5 of the pair (max of the two strings' levels); higher = more mixed-script and more suspicious. The direct mixed-script spoof signal — e.g. Latin+Cyrillic `pаypal` scores high, while legitimate pure CJK stays low. Direction: higher = more different/suspicious. Uses the `unicode-security` crate (MIT/Apache-2.0); see `[dependencies]` in Cargo.toml. |
| `keyboard_distance` | float\|NA | base | mean physical US-QWERTY key distance over the substituted positions, normalized to [0,1]; near 0 = adjacent-key fat-finger typo, near 1 = far-apart/deliberate. Direction: higher = more different/suspicious. NA (JSON `null`, human `n/a`) when either string contains a non-ASCII character (keyboard distance is undefined there); zero substitutions = 0.0 (accurate). Self-contained QWERTY table (`src/keyboard.rs`); no dependency. |

### The confusable model (the conceptual core)

Two characters are confusable when they share the same **skeleton** under UTS #39. `skeleton_of` (in `distance.rs`) does a binary search over the sorted `CONFUSABLES` slice; `confusable(a, b)` compares skeletons (falling back to the char itself when unmapped), which transitively handles confusable chains (Greek omicron, Cyrillic о, and Latin o all skeleton to the same thing, so all three are mutually confusable). Skeleton comparison is **case-sensitive** per UTS #39 (`0`~`O` but not `0`~`o`) — tests encode this, don't "fix" it.

### Multi-character skeletonization (implemented)

`skeleton(s: &str)` builds the full UTS#39 skeleton of a string (each code point mapped through the confusables table and concatenated), so the multi-character confusables that UTS#39 *defines* ARE caught. Among ASCII letters this is essentially just `m` ↔ `rn` (the skeleton of `m` is `rn`). These surface in `skeleton_damerau` (≈0 for a pure multi-char spoof) and set `confusable_only` to `true` (defined as `a != b && skeleton(a) == skeleton(b)`, no equal-length requirement). IMPORTANT: UTS#39 does NOT define reverse mappings like `vv`→`w`, `cl`→`d`, or `nn`→`m` (their right-hand sides are not source code points in the table), so those spoofs are NOT caught and report `confusable_only` false — a known gap, candidate for a future curated supplemental table. Leetspeak (`3`→`e`) is intentionally excluded (UTS #39 does not treat it as visually confusable).

## Regenerating the confusables table

When a new Unicode version ships:

```sh
curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
python3 scripts/gen_confusables.py   # rewrites src/confusables_data.rs
cargo test                   # confirm the embedded table still satisfies the confusable tests
```

`gen_confusables.py` filters the confusables table for embedding. Currently it keeps only entries whose **source is a single code point** (skipping multi-codepoint sources) because `skeleton()` maps each individual code point; multi-codepoint confusables are handled by the full-string skeletonization pass in `skeleton()` itself, which concatenates all mapped code points.

## Output contract

**v0.3.0 BREAKING CHANGE:** The JSON output schema changed significantly from v0.2.0. Downstream parsers must update key names. Keys removed: `homoglyph_damerau`, `normalized`, `skeleton_normalized`. Keys added: `equal`, `skeleton_levenshtein`, `uts39_confusable_count`, `uts39_skeleton_delta`. The `--hogl-weight`/`-w` flag is gone (now an unknown-option error). `--metric` now takes an axis key (default `skeleton_damerau`) rather than `homoglyph|skeleton`.

Both human and JSON output expose the same fields, relied on by downstream pipelines — keep the JSON key names stable. Canonical key order and meaning: see the 10-axis panel table above. `AxisValue` has an `NA` variant that renders as JSON `null` and human `n/a`; `keyboard_distance` uses it when either string is non-ASCII.

**Field filtering:** `--fields <comma-list>` filters which axes are displayed/emitted in all modes. Field names are validated against the axis keys in `ALL_AXES`; an invalid name produces an error listing the valid keys. Identifier keys (`a`/`b` or `input`/`match`) are always preserved; fields emit in canonical `ALL_AXES` order. Note that `-t`/`--metric`/`--sort` operate on the full internal scores regardless of `--fields` setting.

**`--metric <axis>`:** Selects which numeric axis drives `-t` (threshold) and `--sort`. Default: `skeleton_damerau`. The two bool axes (`equal`, `confusable_only`) are rejected with an error listing the valid numeric axis keys.

**Single-pair human verdict:** Single-pair human output appends an interpretation verdict line: `[IDENTICAL]`, `[LIKELY SPOOF]`, or `[LIKELY BENIGN]` with explanation. This is produced by `verdict()` in `verdict.rs` and appears only in human mode (NOT in JSON or batch modes). Spoof rule: `confusable_only || (homoglyph_share > 0.5 && max(len) >= 3 && len_diff_ratio <= len_tolerance)`, where `homoglyph_share = uts39_skeleton_delta / damerau`. Tune with `--len-tolerance` (default 0.25, range [0,1]).

**Batch exit codes:** Batch modes (`--stdin`, `--list`) with `-t` exit 1 when zero rows match; 0 otherwise. Without `-t`, always exit 0.

**Version string:** `-v/--version` prints `sqdist <version> (<git-sha>)`, where the git SHA is captured at build time by `build.rs` (falls back to "unknown" for crates.io/git-less builds).

**Modes and JSON keys:**
- Single pair (two positionals) and `--stdin` batch: JSON keys `a`, `b`
- Watchlist (`--string` + `--list`): JSON keys `input`, `match`

**Batch and list output:** always JSONL (one JSON object per line). With `-t` threshold: emits only lines at/under the threshold. `-m/--metric` (default `skeleton_damerau`) selects which numeric axis drives `-t` and `--sort`.
