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

Two source files plus one build-time helper:

- [src/main.rs](src/main.rs) — everything: distance algorithms, the confusable model, arg parsing, single-pair and `--stdin` batch modes, and the test module. The release profile (Cargo.toml) is tuned for a small fast binary (`lto`, `panic = "abort"`, `strip`).
- [src/confusables_data.rs](src/confusables_data.rs) — **auto-generated, do not hand-edit.** A `static CONFUSABLES: &[(u32, &str)]` slice (~6565 entries) sorted by code point, embedded at compile time so the binary needs no runtime data files or network.
- [scripts/gen_confusables.py](scripts/gen_confusables.py) — regenerates `confusables_data.rs` from Unicode UTS #39 `confusables.txt`. Pure stdlib (no dependencies); resolves its paths relative to the repo root, so run it from anywhere.

### The confusable model (the conceptual core)

Two characters are confusable when they share the same **skeleton** under UTS #39. `skeleton_of` does a binary search over the sorted `CONFUSABLES` slice; `confusable(a, b)` compares skeletons (falling back to the char itself when unmapped), which transitively handles confusable chains (Greek omicron, Cyrillic о, and Latin o all skeleton to the same thing, so all three are mutually confusable). `sub_cost` is the single hook where the homoglyph weight enters the otherwise-standard edit-distance DP. Skeleton comparison is **case-sensitive** per UTS #39 (`0`~`O` but not `0`~`o`) — tests encode this, don't "fix" it.

### Known limitation — do not treat as a bug

The skeleton map keys on **single code points only**, so multi-character homoglyphs (`rn`→`m`, `vv`→`w`, `cl`→`d`) are NOT caught and score as full edits. Supporting them requires a greedy multi-char skeletonization pass before the edit-distance step; the natural next extension, intentionally absent. Leetspeak (`3`→`e`) is deliberately excluded because UTS #39 does not consider it visually confusable.

## Regenerating the confusables table

When a new Unicode version ships:

```sh
curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
python3 scripts/gen_confusables.py   # rewrites src/confusables_data.rs
cargo test                   # confirm the embedded table still satisfies the confusable tests
```

`gen_confusables.py` keeps only entries whose **source is a single code point** (it skips multi-codepoint sources). If multi-char support is ever added, this filter is where the parsing changes.

## Output contract

Both human and JSON output expose the same fields, relied on by downstream pipelines — keep the JSON key names stable: `levenshtein`, `damerau`, `homoglyph_damerau`, `normalized` (= `homoglyph_damerau / max(len_a, len_b)`), `confusable_only` (true when strings differ but are identical after skeletonization — the highest-confidence spoof signal). `--stdin` batch mode always emits one JSON object per line; with `-t` it emits only lines at/under the threshold.
