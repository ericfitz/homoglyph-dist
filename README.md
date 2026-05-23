# sqdist

A blazing-fast Rust CLI for measuring string distance with a focus on
**typosquatting** and **homoglyph** attack detection.

It computes three distances between two strings:

| Metric | Catches |
|---|---|
| **Levenshtein** | insertions, deletions, substitutions (`gogle`, `gooogle`) |
| **Damerau-Levenshtein** | the above **+ adjacent transpositions** as a single edit (`googel`) — matches real keyboard typos |
| **Homoglyph-weighted Damerau** | confusable-character substitutions cost a small fraction of an edit, so visually-identical spoofs (`pаypal` with Cyrillic а) float to the top of your alerts |

The homoglyph model uses the **official Unicode UTS #39 confusables data**
(`confusables.txt`, v17.0.0), the same authoritative source attacker tooling
targets. Two characters are treated as confusable when they share the same
*skeleton* (prototype) under UTS #39 — this transitively handles confusable
chains (e.g. Greek omicron → Latin o, Cyrillic о → Latin o, so all three are
mutually confusable).

## Why the homoglyph weight matters

A homoglyph spoof and a benign typo can have **identical** Levenshtein/Damerau
distance. The weighted metric separates them:

```
                      lev  damerau  homoglyph  confusable_only
paypal vs pаypal       1      1        0.1          true     <- SPOOF
google vs gogle        1      1        1.0          false    <- benign typo
```

Set a threshold (e.g. `-t 0.5`) to alert only on the spoofs.

## Usage

```
sqdist [OPTIONS] <STRING_A> <STRING_B>

OPTIONS:
    -w, --homo-weight <F>   Cost of a homoglyph substitution (default 0.1)
    -t, --threshold <F>     Exit 0 if homoglyph distance <= F (alert), else exit 1
    -s, --stdin             Batch mode: read TAB- or comma-separated pairs from
                            stdin, emit one JSON object per line. With -t, only
                            lines at/under the threshold are emitted (alerts).
    -j, --json              Emit JSON (single-pair mode)
    -h, --help              Help
```

### Single pair

```sh
sqdist paypal pаypal          # human-readable
sqdist -j microsoft micrоsoft # JSON
sqdist -t 0.5 apple аpple && echo "ALERT: likely spoof"
```

### Batch / pipeline (the real security workflow)

Feed candidate↔brand pairs (one per line, tab- or comma-separated). Emits a
JSON line per pair; with `-t` it emits **only alerts** at/under the threshold:

```sh
# scan npm/pypi candidate names against your brand watchlist
generate_pairs | sqdist --stdin -t 0.5 > alerts.jsonl
```

Process-spawn overhead is ~1 ms; the distance computation itself is
sub-microsecond for typical identifier-length strings, so batch mode keeps
everything in one process for high throughput.

## Output fields

- `levenshtein` / `damerau` — integer edit counts (unweighted)
- `homoglyph_damerau` — Damerau distance where confusable substitutions cost `--homo-weight`
- `normalized` — `homoglyph_damerau / max(len_a, len_b)`, a 0–1 similarity-ish score for ranking
- `confusable_only` — `true` when the strings differ but are **identical after skeletonization** (a pure homoglyph attack with zero real edits) — your highest-confidence signal

## Known limitation: multi-character homoglyphs

The skeleton map currently keys on **single code points**, so multi-character
visual confusables are **not** caught:

- `rn` → `m` (`rnicrosoft` vs `microsoft`)
- `vv` → `w`
- `cl` → `d`

These score as full edits. UTS #39 does define multi-char mappings; supporting
them requires a skeletonization pass that greedily rewrites multi-char
sequences before the edit-distance step. The data is already parsed out (2103
multi-target entries were noted during build); wiring a multi-char skeleton
pass is the natural next extension. Leetspeak substitutions (`3`→`e`, `4`→`a`)
are deliberately **not** treated as homoglyphs because UTS #39 does not consider
them visually confusable.

## Building

```sh
cargo build --release    # -> target/release/sqdist
cargo test               # 6 unit tests covering each metric + confusable logic
```

The confusables table is embedded at compile time (`src/confusables_data.rs`,
auto-generated from `confusables.txt`), so the binary is fully self-contained —
no runtime data files, no network.

## Regenerating the confusables table

If a new Unicode version ships, regenerate the embedded table:

```sh
curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
python3 gen_confusables.py   # emits src/confusables_data.rs
```
