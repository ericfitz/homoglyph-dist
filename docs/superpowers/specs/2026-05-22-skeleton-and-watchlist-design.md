# Design: Multi-char skeletonization + watchlist mode

Date: 2026-05-22
Status: Approved
Component: `sqdist` (src/main.rs)

## Summary

Add two features to `sqdist` before the first release:

1. **Full UTS#39 skeletonization** — compute each input's skeleton string and
   measure distance on skeletons, so multi-character confusables (`rn`↔`m`,
   `vv`↔`w`, `cl`↔`d`) are caught. This resolves the README's documented
   "Known limitation".
2. **Watchlist / file mode** — score one CLI string against every line in a
   candidates file (either direction), with optional sorting and truncation.
   Closer to the real workflow of screening Artifactory/registry package names.

Breaking changes are explicitly approved (greenfield, single owner, unreleased).
Optimize for correctness over backward compatibility.

## Feature 1 — Skeletonization

### `skeleton(s: &str) -> String`

A **single, non-recursive** pass: for each `char`, look up its `CONFUSABLES`
skeleton string (existing `skeleton_of`), or fall back to the char itself;
concatenate the results.

This is the UTS#39 skeleton. The confusables table already maps single source
code points to their canonical (possibly multi-char) target form, so one pass
yields the canonical skeleton — we do NOT iterate to a fixed point.

Examples:
- `skeleton("microsoft")` → `"rnicrosoft"` (`m` → `rn`)
- `skeleton("rnicrosoft")` → `"rnicrosoft"` (unchanged) ⇒ equal to above
- `skeleton("pаypal")` (Cyrillic а) → `skeleton("paypal")`

### Scores struct / output fields

Output order (human and JSONL):

| Field | Definition | Status |
|---|---|---|
| `levenshtein` | unweighted Levenshtein on raw strings | unchanged |
| `damerau` | unweighted Damerau on raw strings | unchanged |
| `homoglyph_damerau` | per-char weighted Damerau, single-char confusables, no skeletonization | unchanged |
| `skeleton_damerau` | Damerau on `skeleton(a)` vs `skeleton(b)` | **new** |
| `normalized` | `homoglyph_damerau / max(len_a, len_b)` | unchanged |
| `skeleton_normalized` | `skeleton_damerau / max(skeleton_len_a, skeleton_len_b, 1)` (guard against zero, mirroring `normalized`) | **new** |
| `confusable_only` | `a != b && skeleton(a) == skeleton(b)` | **redefined** (drops equal-length requirement) |

`homoglyph_damerau` and `skeleton_damerau` are both kept: they carry different
signals. Per-char preserves the graded "soft cost" (one homoglyph = 0.1);
skeleton makes confusables free (0.0) and is multi-char aware. Seeing both
distinguishes "a few homoglyph substitutions" from "fully confusable".

Worked example — `rnicrosoft` vs `microsoft`:
- `homoglyph_damerau` ≈ 2.0 (per-char can't align `rn` to `m`)
- `skeleton_damerau` = 0.0, `confusable_only` = true (skeletons identical)

## Feature 2 — Watchlist / file mode

### Flags

- `-f, --file <PATH>` — the multi-line side; one candidate string per line.
  Each non-blank line is scored against the single CLI positional argument.
  Blank/whitespace-only lines skipped; surrounding whitespace trimmed. There is
  no field-splitting (unlike `--stdin`), so there is no "malformed line" case.
  Mutually exclusive with `--stdin`. Requires exactly one positional argument.
- `-r, --reverse` — swap the `a`/`b` roles in output. Default: CLI arg is `a`
  (reference), each file line is `b` (candidate). With `--reverse`: file line is
  `a`, CLI arg is `b` (the "does this candidate impersonate any of my brands?"
  direction).
- `--sort` — buffer all results and emit ordered by the active `--metric`
  distance ascending (most suspicious first).
- `--top <N>` — implies `--sort`; emit only the N closest results.

### Output

Batch modes (`--stdin`, `--file`) always emit **JSONL** (one JSON object per
line, newline-separated, no enclosing array). `--sort`/`--top` emit the same
JSONL, just reordered — never a JSON array. Single-pair mode (`-j`) emits one
JSON object (not JSONL).

`-t` threshold filtering applies as in `--stdin`: only emit pairs whose active
`--metric` distance is ≤ the threshold.

### Invocation examples

```sh
# screen candidates against one known-good name (reference vs candidates)
sqdist --file candidates.txt paypal -t 0.5

# check one new package against a brand watchlist (candidate vs watchlist)
sqdist --file brands.txt newpkg-name --reverse --sort --top 5
```

## Cross-cutting

### `-m, --metric {homoglyph,skeleton}`

Default **`skeleton`**. Selects which distance drives the `-t` threshold and
`--sort` ordering. `--metric homoglyph` selects `homoglyph_damerau`; `skeleton`
selects `skeleton_damerau`. Does not change which fields are emitted (all are
always emitted); only which one is used for filtering/sorting.

### Mode matrix

| Mode | Trigger | Pairing | Output |
|---|---|---|---|
| single pair | two positionals | the two args | human (default) or one JSON (`-j`) |
| stdin batch | `-s/--stdin` | pre-paired lines (tab/comma) | JSONL |
| file/watchlist | `-f/--file` | each line × one positional | JSONL |

Errors: `--stdin` + `--file` together; `--file` with ≠1 positional; `--top`
without a positive integer.

## Refactor for testability

Extract pure functions so `main()` is a thin I/O wrapper:

- `skeleton(&str) -> String`
- `score_pair(a, b, homo_weight) -> Scores` (extended with skeleton fields)
- `metric_value(&Scores, Metric) -> f64` — picks the active distance
- `sort_and_truncate(Vec<(String, String, Scores)>, Metric, top: Option<usize>)`
  — pure ordering/truncation over a vector
- JSONL serialization of one result as a function returning `String`

`main()` handles arg parsing, file/stdin reading, and writing; all logic above
is unit-tested directly in the `#[cfg(test)]` module. No process spawning, no
new dependencies.

## Testing

Unit tests (in `src/main.rs` test module):

- `skeleton()`: `microsoft`→`rnicrosoft`; idempotence `skeleton(skeleton(x)) == skeleton(x)`;
  multi-char (`rn`/`m`, `vv`/`w`); single-char Cyrillic still works.
- `skeleton_damerau` = 0 for `rnicrosoft` vs `microsoft`, while
  `homoglyph_damerau` > 1 for the same pair.
- `confusable_only` true for skeleton-equal pairs of unequal length.
- `metric_value` returns the right field per `Metric`.
- `sort_and_truncate`: ordering ascending by active metric; `--top` truncation;
  stable/deterministic for ties (define: preserve input order on equal scores).
- Existing six tests must still pass (adjusting `confusable_only` expectations
  where the redefinition changes them).

## Out of scope

- No JSON-array output mode.
- No streaming sort (sort buffers; acceptable for candidate-list sizes).
- No change to the `gen_confusables.py` generator (the multi-char target data is
  already present in `confusables_data.rs`).
- No new runtime dependencies.

## Docs to update

- README: rewrite the "Known limitation: multi-character homoglyphs" section
  (now solved); document `--file`, `--reverse`, `--sort`, `--top`, `--metric`,
  the new fields, and clarify JSONL.
- CLAUDE.md: update the "known limitation" framing, the output contract, and the
  mode matrix.
- `sqdist --help` text.
