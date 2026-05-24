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
skeleton-distance axes (below) make genuine spoofs collapse toward zero
distance while real typos stay near 1.0, so you can alert on spoofs without
false-alarming on honest fat-finger typos.

Typical uses:

- **Brand / domain monitoring** — scan newly-registered domains against a brand
  watchlist (`microsоft.com`?).
- **Supply-chain defense** — check new npm / PyPI / crates package names against
  popular names to catch malicious lookalikes before they're installed.
- **Phishing / fraud filtering** — flag deceptive sender names or URLs.

## Breaking changes in v0.3.0 (upgrading from 0.2.x)

**JSON schema changed.** Downstream parsers must update:

- **Removed keys:** `homoglyph_damerau`, `normalized`, `skeleton_normalized`
- **Added keys:** `equal`, `skeleton_levenshtein`, `uts39_confusable_count`, `uts39_skeleton_delta`
- **`--hogl-weight`/`-w` is removed** — unknown-option error if used
- **`--metric` now takes an axis key** (e.g. `skeleton_damerau`) instead of `homoglyph|skeleton`; default is `skeleton_damerau`

## Axes

`sqdist` computes a panel of 8 independent axes for each pair. The `--help`
AXES block and JSON key order:

```
AXES:
    equal                   the strings are byte-identical (bool)
    levenshtein             min single-char insert/delete/substitute edits
    damerau                 like levenshtein, but an adjacent swap counts as one edit
    skeleton_levenshtein    levenshtein after reducing both to UTS#39 skeletons
    skeleton_damerau        damerau after reducing both to UTS#39 skeletons
                            (~0 when visually identical, incl. multi-char confusables)
    uts39_confusable_count  # of aligned substitutions that are UTS#39-confusable (experimental)
    uts39_skeleton_delta    damerau - skeleton_damerau; edits that vanish under
                            skeletonization (experimental, may change)
    confusable_only         true when the strings differ but share an identical skeleton
```

The homoglyph model uses the **official Unicode UTS #39 confusables data**
(`confusables.txt`, v17.0.0), the same authoritative source attacker tooling
targets. Two characters are treated as confusable when they share the same
*skeleton* (prototype) under UTS #39 — this transitively handles confusable
chains (e.g. Greek omicron → Latin o, Cyrillic о → Latin o, so all three are
mutually confusable).

## Why the skeleton axes matter

A homoglyph spoof and a benign typo can have **identical** Levenshtein/Damerau
distance. The skeleton columns collapse homoglyphs — and also handle multi-char
confusables (`rn`→`m`) that per-character approaches miss:

```
                          lev  damerau  skeleton_lev  skeleton_dam  uts39_delta  confusable_only
paypal vs pаypal           1      1          0             0             1          true   <- homoglyph spoof
rnicrosoft vs microsoft    2      2          0             0             2          true   <- multi-char spoof
google vs gogle            1      1          1             1             0          false  <- benign typo
```

Set a threshold (e.g. `-t 1 -m skeleton_damerau`) to alert only on the spoofs.

## Usage

```
sqdist [OPTIONS] <STRING_A> <STRING_B>      # single pair
sqdist [OPTIONS] --stdin                    # batch: pre-paired lines
sqdist [OPTIONS] --string <S> --list <FILE> # score <S> vs each line

OPTIONS:
    -t, --threshold <F>     Alert when the --metric distance <= F. Single-pair:
                            sets exit code. Batch: filters output; exit 1 if none match.
    -m, --metric <AXIS>     Numeric axis for -t and --sort (default skeleton_damerau)
        --fields <LIST>     Comma-separated axes to show (default: all). See AXES.
        --len-tolerance <F> Max length-difference ratio for a spoof verdict (default 0.25)
    -s, --stdin             Batch: read TAB/comma pairs from stdin, emit JSONL
        --string <S>        (with --list) the single string to compare
        --list <FILE>       (with --string) score <S> against each non-blank line
        --sort              List mode: emit most-suspicious-first (buffers)
        --top <N>           List mode: keep only the N closest (implies --sort)
    -j, --json              Emit JSON (single-pair mode)
    -v, --version           Print version and commit, then exit
    -h, --help              This help
```

### Single pair

```sh
sqdist paypal pаypal          # human-readable
sqdist -j microsoft micrоsoft # JSON
sqdist -t 1 apple аpple && echo "ALERT: likely spoof"
```

### Batch / pipeline (the real security workflow)

Feed candidate↔brand pairs (one per line, tab- or comma-separated). Emits a
JSON line per pair; with `-t` it emits **only alerts** at/under the threshold:

```sh
# scan npm/pypi candidate names against your brand watchlist
generate_pairs | sqdist --stdin -t 1 > alerts.jsonl
```

With `-t`, batch modes exit with code 0 if at least one alert was emitted and 1 if none matched.
Without `-t`, batch modes always exit 0. This makes it easy to use in shell conditionals:

```sh
if generate_pairs | sqdist --stdin -t 1 > alerts.jsonl; then
  echo "Found suspicious matches"
else
  echo "All clear"
fi
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

# alert-only: skeleton_damerau distance at/under the threshold
sqdist --string paypal --list candidates.txt -t 1
```

#### Memory usage on large lists

By default list mode **streams**: each matching row is written to stdout as it
is scored, so memory stays flat (~2 MB) no matter how large the file is — a
million-line scan costs the same as a hundred.

`--sort` and `--top` are the exception. Ranking needs every row in hand before
it can order them, so those flags **buffer all kept rows** in memory first. Each
buffered row costs roughly **~100 bytes of fixed overhead + ~2× the candidate's
length in bytes** — about **190 bytes per row for 16-character names**. Peak
resident memory then scales with the number of rows kept:

| Rows kept (≈16-char names), `--sort` | Peak RSS |
|---|---|
| 10,000 | ~5 MB |
| 100,000 | ~23 MB |
| 1,000,000 | ~190 MB |

Rough rule of thumb when sorting: `peak_MB ≈ 2 (baseline) + rows × (100 + 2 × avg_name_len) / 1e6`.

`-t` cuts this down further even when sorting: a threshold drops non-matching
rows *before* they're buffered, so `--sort -t 1` over a million lines that
alerts on only a handful stays near the ~2 MB baseline. So: stream by default;
add `-t` (and optionally `--top`) when you want a ranked view of a very large
list without holding it all in memory. (Chunking a huge list and scanning each
piece is also fine — results are independent per line.)

## Output axes

| Key | Type | Meaning |
|---|---|---|
| `equal` | bool | the strings are byte-identical |
| `levenshtein` | int | min single-char insert/delete/substitute edits |
| `damerau` | int | like levenshtein, but an adjacent swap counts as one edit |
| `skeleton_levenshtein` | int | levenshtein after reducing both to UTS#39 skeletons |
| `skeleton_damerau` | int | damerau after reducing both to UTS#39 skeletons; ~0 when visually identical including multi-char confusables |
| `uts39_confusable_count` | int | # of aligned substitutions that are UTS#39-confusable **(experimental)** |
| `uts39_skeleton_delta` | int | damerau − skeleton_damerau; edits that vanish under skeletonization **(experimental)** |
| `confusable_only` | bool | true when strings differ but share an identical skeleton — highest-confidence spoof signal |

Single-pair and batch (stdin) modes use keys `a` and `b`; list mode uses `input` and `match`. Batch and list modes emit JSONL.

### Example: single-pair human output

```sh
sqdist paypal pаypal   # second 'a' is Cyrillic
```

```
equal                    false
levenshtein              1
damerau                  1
skeleton_levenshtein     0
skeleton_damerau         0
uts39_confusable_count   1
uts39_skeleton_delta     1
confusable_only          true

[LIKELY SPOOF] The strings differ by 1 edit, but every differing character is a homoglyph (the strings are visually identical). High likelihood of an attempt to confuse.
```

### Example: single-pair JSON output

```sh
sqdist -j paypal pаypal   # second 'a' is Cyrillic
```

```json
{"a":"paypal","b":"pаypal","equal":false,"levenshtein":1,"damerau":1,"skeleton_levenshtein":0,"skeleton_damerau":0,"uts39_confusable_count":1,"uts39_skeleton_delta":1,"confusable_only":true}
```

### Selecting axes

The `--fields` flag restricts output to a comma-separated list of axis keys. The identifier keys (a/b in single-pair and stdin, input/match in list mode) are always included regardless. Axes are printed in canonical order regardless of the input order. Invalid axis names produce an error listing the valid keys.

Example: show only damerau and confusable_only:

```sh
sqdist --fields damerau,confusable_only GOOGLE GO0GLE
```

Output:
```
damerau              1
confusable_only      true

[LIKELY SPOOF] The strings differ by 1 edit, but every differing character is a homoglyph (the strings are visually identical). High likelihood of an attempt to confuse.
```

The same axes in JSON output:

```sh
sqdist -j --fields damerau,confusable_only GOOGLE GO0GLE
```

Output:
```json
{"a":"GOOGLE","b":"GO0GLE","damerau":1,"confusable_only":true}
```

### Selecting the metric axis

`-m/--metric <AXIS>` selects which numeric axis drives `-t` (threshold filtering and exit code) and `--sort`. Default: `skeleton_damerau`. The two bool axes (`equal`, `confusable_only`) are not valid metric axes and produce an error listing the valid numeric keys.

```sh
sqdist -m skeleton_damerau -t 1 paypal pаypal
sqdist --string microsoft --list candidates.txt --sort -m levenshtein
```

### Interpretation verdict (single-pair)

In single-pair human output (not `-j` JSON, not batch modes), a verdict line appears below the axis table:

The verdict is one of:
- `[IDENTICAL]` — the two strings are identical
- `[LIKELY SPOOF]` — strong signal of a homoglyph attack
- `[LIKELY BENIGN]` — a real typo or legitimate edit

A likely spoof is signaled when:
- All differing characters are homoglyphs (`confusable_only = true`), **OR**
- Homoglyphs account for more than half the Damerau distance within a length tolerance (default `--len-tolerance 0.25`)

This verdict helps distinguish attacks from honest typos and appears only in single-pair human output — it's never emitted in JSON mode or batch modes.

## Multi-character homoglyphs

Some multi-character visual confusables are detected via full UTS#39
skeletonization — each string is reduced to its skeleton (every code point
mapped through the confusables table and concatenated) before measuring
distance. The classic example is the letter **m**, whose UTS#39 skeleton is
`rn`, so a spoof that swaps one for the other collapses to a zero-distance match:

- `rn` ↔ `m` (`rnicrosoft` vs `microsoft`) — **detected**

This surfaces in the `skeleton_damerau` field (~0 for a pure multi-char spoof)
and sets `confusable_only` to `true`, even when the strings differ in length.

### What is *not* caught

sqdist only knows the confusables that **UTS#39 itself defines**. Some
visually-plausible multi-character spoofs are **not** in the Unicode data and
are therefore reported as ordinary edits (`confusable_only` false), e.g.:

- `vv` ≈ `w`
- `cl` ≈ `d`
- `nn` ≈ `m`

UTS#39 keys its multi-character skeletons on a small set of single code points
(among ASCII letters, essentially just `m` → `rn`); it does not provide the
reverse `vv → w` style mappings. Supplementing the official data with a curated
table of these pairs is a candidate future enhancement. Leetspeak substitutions
(`3`→`e`, `4`→`a`) are deliberately excluded — UTS#39 does not consider them
visually confusable.

## Installing

```sh
# Homebrew (macOS) — prebuilt, signed, notarized universal binary
brew install ericfitz/tap/sqdist

# Cargo (any platform with the Rust toolchain) — builds from source
cargo install sqdist
```

You can also grab a signed `.pkg` installer or the universal tarball directly
from the [latest release](https://github.com/ericfitz/homoglyph-dist/releases/latest).

To check your installed version:

```sh
sqdist --version
# sqdist 0.3.0 (d430de5)
```

## Building

```sh
cargo build --release    # -> target/release/sqdist
cargo test               # unit tests in each module's #[cfg(test)] block; currently 56 tests
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
