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

`sqdist` computes a panel of 10 independent axes for each pair. The `--help`
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
    script_restriction      UTS#39 restriction level 0-5 (higher = more mixed-script/suspicious)
    keyboard_distance       mean QWERTY key distance over substitutions, 0-1 (n/a if non-ASCII)
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
        --confusables <LIST> Confusable sources for skeletons: uts39,flowcrypt,digraph (default uts39)
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
| `script_restriction` | int | UTS#39 restriction level 0–5 of the pair (max of the two strings' levels); higher = more mixed-script and more suspicious. Direction: higher = more different/suspicious. |
| `keyboard_distance` | float\|null | mean physical US-QWERTY key distance over the substituted positions, normalized to [0,1]; near 0 = adjacent-key fat-finger typo, near 1 = far-apart/deliberate. Direction: higher = more different/suspicious. NA (JSON `null`, human `n/a`) when either string contains a non-ASCII character (keyboard distance is undefined there); zero substitutions = 0.0 (accurate). Self-contained QWERTY table (`src/keyboard.rs`); no dependency. |

`AxisValue` has an `NA` variant that renders as JSON `null` and human `n/a`; `keyboard_distance` uses it when either string is non-ASCII.

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
script_restriction       4
keyboard_distance        n/a

[LIKELY SPOOF] The strings differ by 1 edit, but every differing character is a homoglyph (the strings are visually identical). High likelihood of an attempt to confuse.
```

### Example: single-pair JSON output

```sh
sqdist -j paypal pаypal   # second 'a' is Cyrillic
```

```json
{"a":"paypal","b":"pаypal","equal":false,"levenshtein":1,"damerau":1,"skeleton_levenshtein":0,"skeleton_damerau":0,"uts39_confusable_count":1,"uts39_skeleton_delta":1,"confusable_only":true,"script_restriction":4,"keyboard_distance":null}
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

### What UTS#39 alone does *not* catch — and how to close the gap

UTS#39 keys its multi-character skeletons on a small set of single code points
(among ASCII letters, essentially just `m` → `rn`); it does not provide reverse
`vv → w` style mappings. By default sqdist uses only pure UTS#39, so these three
spoofs are reported as ordinary edits (`confusable_only` false):

- `vv` ≈ `w`
- `cl` ≈ `d`
- `nn` ≈ `m` (note: `rn` ≈ `m` *is* caught — see above)

**These gaps are closable via `--confusables=digraph`** (see Supplemental
confusables below). The digraph source adds exactly those three curated mappings.
Leetspeak substitutions (`3`→`e`, `4`→`a`) are deliberately excluded — UTS#39
does not consider them visually confusable and sqdist follows that policy.

## Supplemental confusables

`--confusables <LIST>` selects which confusable sources are active during
skeleton computation. The value is a comma-separated list; the default is
`uts39`.

| Source | Description | Default |
|---|---|---|
| `uts39` | Unicode UTS#39 confusables data (authoritative, ~6565 entries). Always included. | yes |
| `flowcrypt` | FlowCrypt `idn-homographs-database` single-char supplement — 477 ASCII-base look-alikes anchored to UTS#39 skeletons. MIT-licensed. | no (opt-in) |
| `digraph` | Three curated multi-char mappings: `vv→w`, `cl→d`, `nn→rn`. Closes the three most common UTS#39 multi-char gaps. | no (opt-in) |

An unknown source name is an error (exit 2).

**The default is pure UTS#39 — unchanged from prior releases.** The supplemental
sources are opt-in and less authoritative. In particular, `cl→d` has a higher
false-positive rate (`clear` matches `dear`), so use the `digraph` source with
that caveat in mind. UTS#39 always wins on any collision.

**The skeleton axes (`skeleton_levenshtein`, `skeleton_damerau`,
`confusable_only`) reflect whichever sources are enabled.** Changing
`--confusables` changes the behavior of those three axes.

### Worked example: closing the vv/w gap

```sh
# Default (pure UTS#39): vv is not a known confusable for w
sqdist -j devflovv devflow
# → "confusable_only":false, "skeleton_damerau":2

# With digraph source: vv→w mapping is active, gap closes
sqdist -j --confusables uts39,digraph devflovv devflow
# → "confusable_only":true, "skeleton_damerau":0
```

### FlowCrypt attribution

The `flowcrypt` source is derived from the
[FlowCrypt idn-homographs-database](https://github.com/FlowCrypt/idn-homographs-database),
licensed under the MIT License. The embedded data is pinned to commit `f27b783`
(retrieved 2021-05-26).

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
cargo test               # unit tests in each module's #[cfg(test)] block; currently 88 tests
```

The confusables table is embedded at compile time (`src/confusables_data.rs`,
auto-generated from `confusables.txt`), so **no runtime data files or network
access** are needed for confusables. sqdist has one compiled dependency:
`unicode-security` (MIT/Apache-2.0), used for the `script_restriction` axis.

### Version and data provenance

`-v`/`--version` prints the binary version, build commit, and one `data:` line
per embedded confusable source (regardless of `--confusables`):

```
sqdist 0.3.0 (<sha>)
  data: UTS#39 confusables.txt v17.0.0 (2025-07-22)
  data: FlowCrypt idn-homographs-database @ f27b783 (retrieved 2021-05-26)
```

## Regenerating the confusables table

If a new Unicode version ships, regenerate the embedded UTS#39 table:

```sh
curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
python3 scripts/gen_confusables.py   # emits src/confusables_data.rs
```

### Regenerating the FlowCrypt table

The FlowCrypt single-char supplement is regenerated from the upstream
`idn-homographs-database` JSON. The 19 MB source file is not committed; fetch it
when you need to update:

```sh
# Online (fetches source JSON from GitHub)
python3 scripts/gen_flowcrypt.py   # emits src/flowcrypt_data.rs

# Offline / pinned (provide local copies + provenance metadata)
python3 scripts/gen_flowcrypt.py \
  --homographs path/to/homographs.json \
  --confusables path/to/confusables.txt \
  --source-commit <sha> \
  --source-date <YYYY-MM-DD>
```
