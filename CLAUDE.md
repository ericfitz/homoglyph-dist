# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`sqdist` — a single-binary Rust CLI that computes three string-distance metrics between two strings for **typosquatting / homoglyph attack detection**: Levenshtein, Damerau-Levenshtein (OSA, adjacent transpositions), and a **homoglyph-weighted Damerau** where confusable-character substitutions cost a fraction of an edit (default 0.1). The point is to separate a visual spoof (`pаypal` with Cyrillic а) from a benign typo (`gogle`) that has identical unweighted distance.

## Commands

```sh
cargo build --release    # -> target/release/sqdist
cargo test               # 6 unit tests in src/main.rs (one per metric + confusable logic)
cargo test <name>        # run a single test, e.g. cargo test digit_letter_confusable
cargo run -- <A> <B>     # run against two strings (note the -- before args)
```

There is no separate lint config; use `cargo clippy` and `cargo fmt --check`.

## Architecture

Two source files plus build-time helpers:

- [src/main.rs](src/main.rs) — everything: distance algorithms, the confusable model, arg parsing, single-pair and `--stdin` batch modes, and the test module. The release profile (Cargo.toml) is tuned for a small fast binary (`lto`, `panic = "abort"`, `strip`).
- [src/confusables_data.rs](src/confusables_data.rs) — **auto-generated, do not hand-edit.** A `static CONFUSABLES: &[(u32, &str)]` slice (~6565 entries) sorted by code point, embedded at compile time so the binary needs no runtime data files or network.
- [build.rs](build.rs) — compile-time git SHA capture (runs `git rev-parse --short HEAD`, exposes `SQDIST_GIT_SHA` env var, falls back to "unknown" for crates.io/git-less builds).
- [scripts/gen_confusables.py](scripts/gen_confusables.py) — regenerates `confusables_data.rs` from Unicode UTS #39 `confusables.txt`. Pure stdlib (no dependencies); resolves its paths relative to the repo root, so run it from anywhere.

### Modes and structure

Three modes, dispatched by a thin `main()`:

| Mode | Trigger | Pairing | JSON keys |
|---|---|---|---|
| single pair | two positionals | the two args | `a`/`b` |
| stdin batch | `--stdin` | pre-paired tab/comma lines | `a`/`b` |
| watchlist | `--string` + `--list` | `--string` × each file line | `input`/`match` |

The logic lives in pure, unit-tested functions — `skeleton`, `score_pair`,
`metric_value`, `sort_and_truncate`, `process_list`, `result_json`, `parse_from(Vec<String>)` arg parser, `verdict`, `parse_fields`, `selected_fields`, and `batch_matched_ok` — plus the `Field` and `Verdict` enums and `Field::value_string` method — with `main()` only doing I/O dispatch.

### The confusable model (the conceptual core)

Two characters are confusable when they share the same **skeleton** under UTS #39. `skeleton_of` does a binary search over the sorted `CONFUSABLES` slice; `confusable(a, b)` compares skeletons (falling back to the char itself when unmapped), which transitively handles confusable chains (Greek omicron, Cyrillic о, and Latin o all skeleton to the same thing, so all three are mutually confusable). `sub_cost` is the single hook where the homoglyph weight enters the otherwise-standard edit-distance DP. Skeleton comparison is **case-sensitive** per UTS #39 (`0`~`O` but not `0`~`o`) — tests encode this, don't "fix" it.

### Multi-character skeletonization (implemented)

`skeleton(s: &str)` builds the full UTS#39 skeleton of a string (each code point
mapped through the confusables table and concatenated), so multi-character
confusables ARE caught: `rn`↔`m`, `vv`↔`w`, `cl`↔`d`. These surface in
`skeleton_damerau` (≈0 for a pure multi-char spoof) and set `confusable_only`
to `true` (defined as `a != b && skeleton(a) == skeleton(b)`, no equal-length
requirement). The per-char `homoglyph_damerau` metric does NOT collapse
multi-char sequences — keeping both lets a caller distinguish a few homoglyph
substitutions from a fully-confusable string. Leetspeak (`3`→`e`) is still
intentionally excluded (UTS #39 does not treat it as visually confusable).

## Regenerating the confusables table

When a new Unicode version ships:

```sh
curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
python3 scripts/gen_confusables.py   # rewrites src/confusables_data.rs
cargo test                   # confirm the embedded table still satisfies the confusable tests
```

`gen_confusables.py` filters the confusables table for embedding. Currently it keeps only entries whose **source is a single code point** (skipping multi-codepoint sources) because `skeleton()` maps each individual code point; multi-codepoint confusables are handled by the full-string skeletonization pass in `skeleton()` itself, which concatenates all mapped code points.

## Output contract

Both human and JSON output expose the same fields, relied on by downstream pipelines — keep the JSON key names stable:

- `levenshtein`, `damerau`, `homoglyph_damerau` — three distance metrics
- `skeleton_damerau` — distance after full-string skeletonization (multi-char confusables caught)
- `normalized` — `homoglyph_damerau / max(len_a, len_b)`
- `skeleton_normalized` — `skeleton_damerau / max(len(skeleton(a)), len(skeleton(b)))`
- `confusable_only` — true when strings differ but are identical after skeletonization (highest-confidence spoof signal)

**Field filtering:** `--fields <comma-list>` filters which score fields are displayed/emitted in all modes. Identifier keys (`a`/`b` or `input`/`match`) are always preserved; fields emit in canonical order. Note that `-t`/`--metric`/`--sort` operate on the full internal scores regardless of `--fields` setting.

**Single-pair human verdict:** Single-pair human output appends an interpretation verdict line: `[IDENTICAL]`, `[LIKELY SPOOF]`, or `[LIKELY BENIGN]` with explanation. This is produced by the pure `verdict(scores, len_a, len_b, len_tolerance)` function and appears only in human mode (NOT in JSON or batch modes). Spoof rule: `confusable_only || (homoglyph_share > 0.5 && max(len) >= 3 && len_diff_ratio <= len_tolerance)`, where `homoglyph_share = (damerau - skeleton_damerau) / damerau`. Tune with `--len-tolerance` (default 0.25, range [0,1]).

**Batch exit codes:** Batch modes (`--stdin`, `--list`) with `-t` exit 1 when zero rows match; 0 otherwise. Without `-t`, always exit 0.

**Version string:** `-v/--version` prints `sqdist <version> (<git-sha>)`, where the git SHA is captured at build time by `build.rs` (falls back to "unknown" for crates.io/git-less builds).

**Modes and JSON keys:**
- Single pair (two positionals) and `--stdin` batch: JSON keys `a`, `b`
- Watchlist (`--string` + `--list`): JSON keys `input`, `match`

**Batch and list output:** always JSONL (one JSON object per line). With `-t` threshold: emits only lines at/under the threshold. `-m/--metric` (default `skeleton`) selects which distance drives `-t` and `--sort`.
