# sqdist

A blazing-fast Rust CLI for measuring string distance with a focus on
**typosquatting** and **homoglyph** attack detection.

## What it's for

`sqdist` detects when one string is trying to *impersonate* another — the two
main families of name-based impersonation attack:

- **Typosquatting** — names one keyboard slip away from a real one (`gogle`,
  `gooogle`, `googel` for `google`), used for malicious lookalike domains and
  package names.
- **Homoglyph spoofing** — characters that *look identical* but are different
  Unicode code points: `pаypal` where the `а` is Cyrillic. Indistinguishable to
  a human, a completely different string to a byte comparison.

The hard part is that a homoglyph spoof and an innocent typo can have the
**exact same** edit distance, so plain Levenshtein can't separate them. The
homoglyph-weighted metric (below) makes genuine spoofs sink toward zero distance
while real typos stay near 1.0, so you can alert on spoofs without false-alarming
on honest fat-finger typos.

Typical uses:

- **Brand / domain monitoring** — scan newly-registered domains against a brand
  watchlist (`microsоft.com`?).
- **Supply-chain defense** — check new npm / PyPI / crates package names against
  popular names to catch malicious lookalikes before they're installed.
- **Phishing / fraud filtering** — flag deceptive sender names or URLs.

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
distance. The weighted metric separates them — and the skeleton column also
collapses multi-char confusables (`rn`→`m`) that the per-character homoglyph
column misses:

```
                          lev  damerau  homoglyph  skeleton  confusable_only
paypal vs pаypal           1      1        0.1        0        true   <- homoglyph spoof
rnicrosoft vs microsoft    2      2        2          0        true   <- multi-char spoof
google vs gogle            1      1        1          1        false  <- benign typo
```

Set a threshold (e.g. `-t 0.5`) to alert only on the spoofs.

## Usage

```
sqdist [OPTIONS] <STRING_A> <STRING_B>      # single pair
sqdist [OPTIONS] --stdin                    # batch: pre-paired lines
sqdist [OPTIONS] --string <S> --list <FILE> # score <S> vs each line

OPTIONS:
    -w, --hogl-weight <F>   Cost of a homoglyph substitution (default 0.1)
    -t, --threshold <F>     Alert (emit / exit 0) when the --metric distance <= F
    -m, --metric <M>        Distance for -t and --sort: homoglyph|skeleton (default skeleton)
    -s, --stdin             Batch: read TAB/comma pairs from stdin, emit JSONL
        --string <S>        (with --list) the single string to compare
        --list <FILE>       (with --string) score <S> against each non-blank line
        --sort              List mode: emit most-suspicious-first (buffers)
        --top <N>           List mode: keep only the N closest (implies --sort)
    -j, --json              Emit JSON (single-pair mode)
    -h, --help              This help
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

### Watchlist mode (one string vs. a file)

Score a single name against every line of a candidates file — closer to how
you'd screen registry/Artifactory package names against a known-good name:

```sh
# emit JSONL (input/match keys), most-suspicious first, top 10
sqdist --string paypal --list candidates.txt --sort --top 10

# alert-only: skeleton distance at/under the threshold
sqdist --string paypal --list candidates.txt -t 0.5
```

## Output fields

- `levenshtein` / `damerau` — integer edit counts (unweighted)
- `homoglyph_damerau` — Damerau distance where confusable substitutions cost `--hogl-weight`
- `normalized` — `homoglyph_damerau / max(len_a, len_b)`, a 0–1 similarity-ish score for ranking
- `skeleton_damerau` — Damerau distance computed on the two strings' full UTS#39 skeletons; ~0 when they are visually identical including multi-char confusables.
- `skeleton_normalized` — `skeleton_damerau / max(skeleton_len_a, skeleton_len_b)`, a 0–1 score for ranking.
- `confusable_only` — `true` when the strings differ but are **identical after skeletonization** (a pure homoglyph attack with zero real edits) — your highest-confidence signal

Single-pair and batch (stdin) modes use keys `a` and `b`; list mode uses `input` and `match`. Batch and list modes emit JSONL.

## Multi-character homoglyphs

Multi-character visual confusables ARE detected via full UTS#39
skeletonization — each string is reduced to its skeleton (every code point
mapped through the confusables table and concatenated) before measuring
distance:

- `rn` → `m` (`rnicrosoft` vs `microsoft`)
- `vv` → `w`
- `cl` → `d`

These surface in the `skeleton_damerau` field (~0 for a pure multi-char spoof)
and set `confusable_only` to `true`, even when the strings differ in length.
The `homoglyph_damerau` field uses per-character weighting and does NOT collapse
multi-char sequences, so comparing the two fields distinguishes "a few homoglyph
substitutions" from "fully visually confusable".

Leetspeak substitutions (`3`→`e`, `4`→`a`) are deliberately NOT treated as
homoglyphs because UTS #39 does not consider them visually confusable.

## Building

```sh
cargo build --release    # -> target/release/sqdist
cargo test               # 21 unit tests covering metrics, skeletonization, confusable logic, arg parsing, and list mode
```

The confusables table is embedded at compile time (`src/confusables_data.rs`,
auto-generated from `confusables.txt`), so the binary is fully self-contained —
no runtime data files, no network.

## Regenerating the confusables table

If a new Unicode version ships, regenerate the embedded table:

```sh
curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
python3 scripts/gen_confusables.py   # emits src/confusables_data.rs
```
