# Design: `--typosquat` profile and `--pypi` / `--normalize` rules

Date: 2026-08-23
Status: Approved
Component: `sqdist` — CLI profile for package-registry typosquat detection,
plus a file-driven string normalizer with a PyPI PEP 503 preset.

Version: **0.4.0** (additive flags and opt-in JSON keys; default `sqdist a b`
JSON and verdict are unchanged).

## Summary

Two composing features for the package-registry use case:

1. **`--typosquat`** — a detection profile: emit the five axes that matter for
   ASCII typos and visual lookalikes, classify each pair into four tags, and
   treat batch/list mode as an **alert feed** (`likely_typosquat` only).
2. **`--pypi` and `--normalize`/`-n`** — identity-gate then score under registry
   name rules. `--pypi` is the PEP 503 preset. `-n <PATH>` loads an ordered JSON
   op list from a file. There is no `--normalize pypi` (that would be a file
   named `pypi`), no inline JSON, and no colon-delimited mini-language.

They compose. npm-style names: `--typosquat`. PyPI: `--typosquat --pypi`.
Custom rules: `--typosquat -n rules.json`. Normalization without the profile is
allowed (`sqdist --pypi foo_bar foo-bar` → same project, not a squat).

No new runtime dependencies. New module for the normalizer; classification
lives next to the existing human verdict.

## Motivation

sqdist already scores the right signals for registry typosquats (`damerau`,
`keyboard_distance`, `confusable_only`, skeleton distances). The defaults fight
that use case:

- Default `--metric` is `skeleton_damerau`; the human verdict is homoglyph-
  oriented, so `lodash` vs `lodahs` prints `[LIKELY BENIGN]`.
- PyPI treats `.`, `_`, and `-` (and case) as the same project (PEP 503). Raw
  Damerau on `requests_toolbelt` vs `requests-toolbelt` is 1 — a false alert
  unless the caller normalizes first.
- Getting the right `--fields` / `--metric` / threshold combination is too many
  flags for the intended pipeline.

This spec adds a profile and a normalizer so the one-liner is the intended
behavior, without changing default JSON.

## Non-goals

- Combosquats / affix stuffing (`lodash` vs `lodash-utils`).
- Named presets other than `--pypi` (`npm`, `crates` later).
- `--normalize pypi`, inline JSON, colon-delimited `map:` / `fold:` syntax,
  `--normalize-file` as a second flag name.
- Subcommands or `--profile`.
- Changing default-mode JSON keys or the spoof/benign verdict when none of
  the new flags are set.
- Skipping internal axis computation (the full panel is still built; emit
  set is what changes).
- Validating PyPI/npm name grammar (`---` becoming `-` is still scored).
- Rules from stdin (`-n -` is a file named `-`).

## CLI

Two independent flags plus one engine flag:

| Flag | Role |
|---|---|
| `--typosquat` | Detection profile (emit set, classification, batch filter, default metric). |
| `--pypi` | Expand the built-in PEP 503 op list, then continue. |
| `-n`, `--normalize <PATH>` | Append ops from that JSON file. Repeatable. |

`--pypi` and `-n` may combine. **Argv order is op order**:
`--pypi -n extra.json` ≠ `-n extra.json --pypi`. Duplicate `--pypi` is an error.

`--normalize pypi` opens a file named `pypi` in the current directory (or the
given relative path). It is **not** a preset.

### Flag interactions

- `--fields` overrides the `--typosquat` five-axis emit set. Classification is
  still computed and, under `--typosquat`, still emitted.
- `--metric` still drives `--sort` / `--top`. Under `--typosquat` the default
  metric is `damerau` (not `skeleton_damerau`).
- `-t` / `--threshold` **cannot** combine with `--typosquat` (exit 2). The
  classification rule *is* the filter; a damerau `-t 1` would drop
  `rnicrosoft` (`damerau=2`, `confusable_only=true`).
- `--len-tolerance` is ignored by `--typosquat` classification; it still
  affects default-mode human spoof/benign verdict only. Not an error.
- Existing mode conflicts (`--list`/`--string`/`--stdin`/positionals) unchanged.

## Data flow (per pair)

1. Keep **originals** for identifier keys (`a`/`b` or `input`/`match`).
2. If `--pypi` and/or `-n`: apply the concatenated op list to both strings
   → `a_norm`, `b_norm`.
3. If originals are equal → `identical`. Panel on originals.
4. Else if a normalizer ran **and** `a_norm == b_norm` → `same_project`.
   Panel still on **originals** (analyst sees `_` vs `-`). Not a squat.
5. Else score the **normalized** strings when a normalizer ran, else the
   originals. Full panel is always computed.
6. If `--typosquat`, classify (see below).
7. Batch/list + `--typosquat`: emit only `likely_typosquat` rows; exit 1 if
   none. Single-pair always prints.

`equal` and the other axes describe the **scored** strings. That is why
`*_normalized` keys exist when a normalizer is on.

## Normalizer

New module `src/normalize.rs`. Pure functions: parse ops, apply to one
`&str`, produce `String`. The CLI loads files and concatenates op lists.

### File format

UTF-8 JSON. Root is an **ordered array of ops**. Each op is a JSON array:

| Op | Form | Meaning |
|---|---|---|
| `lower` | `["lower"]` | Unicode lowercase (`str::to_lowercase`). |
| `map` | `["map", CHARSET, REPL]` | Each char in `CHARSET` is replaced by `REPL` (1:1; **no** run compression). `CHARSET` is a non-empty string (set of chars; order does not matter; duplicate chars are fine). `REPL` is any string, including `""` and `":"`. |
| `collapse` | `["collapse", CHARSET]` | Maximal runs of 2+ of the **same** character, if that character is in `CHARSET`, become a single instance of that character. Mixed runs are not merged: `.-_` is unchanged. `CHARSET` non-empty. |

Unknown op, wrong arity, non-string operands, empty `CHARSET`, non-array
root or op → error (exit 2). Empty root `[]` is valid: identity transform.

Colon in charset or replacement is a normal JSON character (`":"`). No
backslash-colon, no doubled-colon escape, no user-chosen delimiter.

`--pypi` expands to exactly:

```json
[["lower"], ["map", "._-", "-"], ["collapse", "-"]]
```

That is PEP 503 (`re.sub(r"[-_.]+", "-", name).lower()`) decomposed so that:

| Input | After `map "._-" → "-"` | After `collapse "-"` |
|---|---|---|
| `my---package` | `my---package` | `my-package` |
| `my___package` | `my---package` | `my-package` |
| `my.-_package` | `my---package` | `my-package` |
| `requests_toolbelt` | `requests-toolbelt` | `requests-toolbelt` |

`map` alone does **not** make `my---package` and `my-package` equal.
`collapse` on `._-` **before** map does **not** merge `my.-_package` (different
characters). Op order is left to right; `--pypi` is lower → map → collapse.

Example file (custom, includes colon in charset):

```json
[["map", "._-:", "-"], ["collapse", "-"]]
```

The shell only quotes a path:

```sh
sqdist --typosquat --pypi --string lodash --list names.txt
sqdist --typosquat -n rules.json --string lodash --list names.txt
```

### Apply

```rust
pub enum NormOp {
    Lower,
    Map { charset: String, repl: String },
    Collapse { charset: String },
}

pub fn parse_ops(json: &str) -> Result<Vec<NormOp>, String>;
pub fn apply_ops(s: &str, ops: &[NormOp]) -> String;
```

`Map`: for each `char` in `s`, if it occurs in `charset`, append `repl`;
else append the char. `Collapse`: scan chars; for a char in `charset`, emit
it once and skip following identical chars; other chars pass through.

## `--typosquat` classification

Four tags, in this order:

| Tag | When |
|---|---|
| `identical` | originals are equal |
| `same_project` | a normalizer ran, originals differ, normalized forms are equal |
| `likely_typosquat` | `confusable_only` is true **or** `damerau ≤ 1`, **and** `max(scored_len_a, scored_len_b) ≥ 3` |
| `unrelated` | otherwise |

Lengths are character counts of the **scored** strings (normalized when
step 5 applied). `damerau` / `confusable_only` are the panel values on those
strings.

This is **not** the default homoglyph verdict. `lodash` vs `lodahs` is
`likely_typosquat` (one Damerau edit), not “benign typo”. Visual collapse
(`1odash`, `rnicrosoft`) is also `likely_typosquat` even when `damerau > 1`.
Combosquats (`lodash-utils`) stay `unrelated`. Pairs with longer side `< 3`
stay `unrelated` (known gap for 1–2 character package names).

`same_project` only exists when `--pypi` or `-n` ran.

### Emit set

Default under `--typosquat`:

`equal`, `damerau`, `skeleton_damerau`, `confusable_only`, `keyboard_distance`

Canonical `ALL_AXES` order is preserved among those keys. `--fields` replaces
this set entirely (does not union).

### JSON (opt-in keys)

**Default JSON** (no new flags): unchanged. Existing tests that forbid a
`"normalized"` key remain valid; new keys are `a_normalized` /
`input_normalized`, never the retired v0.2 `normalized`.

When `--pypi` or `-n` is on, after the identifier pair:

- single-pair / stdin: `a_normalized`, `b_normalized`
- list: `input_normalized`, `match_normalized`

Always present under those flags, even if they equal the originals.

When `--typosquat` is on, after the selected axes:

- `classification` — one of `identical`, `same_project`, `likely_typosquat`,
  `unrelated` (stable parse key)
- `reason` — short English explanation. **Not** a parse key; wording may
  change. Typical:
  - identical: `The strings are identical.`
  - same_project: `The names differ only by registry normalization (same project).`
  - likely_typosquat + `confusable_only`: `Strings differ but share a confusable skeleton (visual lookalike).`
  - likely_typosquat + Damerau ≤ 1: `1 Damerau edit.` May mention keyboard
    adjacency when `keyboard_distance` is a finite value near 0.
  - unrelated: `No typosquat signal (damerau > 1 and not confusable-only).`

Key order: identifiers → optional `*_normalized` → selected axes in
`ALL_AXES` order → `classification` → `reason`.

Without `--typosquat`, `classification` / `reason` are omitted. Same-project
is still visible as `a != b` and `a_normalized == b_normalized`, and human
mode prints `[SAME PROJECT]`.

### Human (single-pair)

If a normalizer ran and at least one side changed, one line before the axis
table:

```
normalized               foo-bar / foo-bar
```

Then the selected axes (padding as today). Then a blank line and:

```
[IDENTICAL] …
[SAME PROJECT] …
[LIKELY TYPOSQUAT] …
[UNRELATED] …
```

Default mode without `--typosquat` keeps `[IDENTICAL]` / `[LIKELY SPOOF]` /
`[LIKELY BENIGN]`, plus `[SAME PROJECT]` when the identity gate hits.

Batch/list remain JSONL (no human table), as today.

## Exit codes

| Code | When |
|---|---|
| 0 | Success; batch/list `--typosquat` with ≥1 `likely_typosquat` row; single-pair `--typosquat` (no `-t`) |
| 1 | Batch/list `--typosquat` with zero alerts (same idea as today’s `-t` miss) |
| 2 | Usage, I/O, rules-file errors |

Exit 2 includes: `-n` missing path; path not found / not a file / unreadable /
not valid UTF-8 JSON; JSON schema errors above; duplicate `--pypi`;
`--typosquat` with `-t`; existing parse errors.

Single-pair `--typosquat` does not fail the process on `unrelated` (there is
no `-t`).

## Implementation sketch

- `src/normalize.rs` — `NormOp`, `parse_ops`, `apply_ops`, `PYPI_OPS` (or a
  function that returns the `--pypi` list). Unit tests for map vs collapse vs
  the composed preset live here.
- Classification function next to `verdict.rs` (new `typosquat_verdict` or
  extend the module). Reads originals, optional norms, and the panel. Does
  not replace `verdict()` for default mode.
- `src/main.rs` — flags, file load, argv-ordered op concat, per-pair flow,
  JSON extras, `--typosquat` emit defaults, batch filter, help text.
- Version bump to 0.4.0 in `Cargo.toml`. README + help + `CLAUDE.md` panel
  docs: new flags, classification, PEP 503 caveat.

No change to `ALL_AXES` membership. `classification` is not an axis.

## Tests

Same style: `#[cfg(test)]` in the touched modules; `cargo test` is the suite.

**Normalizer:** `lower`; `map` 1:1 (`my___package` → `my---package`);
`collapse` same-char only (`my---package` → `my-package`, `my.-_package`
unchanged); `--pypi` composition on `my---package` / `my___package` /
`my.-_package` / `Friendly.Bard` → `friendly-bard`; colon in charset/REPL
via JSON strings; `[]` identity; schema errors (unknown op, empty charset,
object root).

**CLI parse:** `-n file`; `--normalize file`; `--pypi -n file` vs
`-n file --pypi` order (apply ops in argv order); `-n pypi` is a filename;
`--typosquat -t` errors; `--pypi --pypi` errors.

**Classification:** `lodash`/`lodahs` → `likely_typosquat`; `1odash` →
`likely_typosquat` (`confusable_only`); `rnicrosoft` → `likely_typosquat`
despite `damerau=2`; `google`/`go0gle` → `likely_typosquat`; `ab`/`ac` →
`unrelated`; `lodash`/`lodash-utils` → `unrelated`; `--pypi`
`foo_bar`/`foo-bar` → `same_project`; equal originals → `identical`.

**Output:** `--typosquat` JSON has `classification` + `reason` and the five
axes only (unless `--fields`); `--pypi` JSON has `*_normalized`; list mode
uses `input_normalized`/`match_normalized`; default JSON still has no
`"normalized"` key (the existing substring assertion
`!line.contains("normalized")` is only valid on default JSON — `a_normalized`
contains that substring); batch `--typosquat` omits `unrelated` and
`same_project`.

## Help text (OPTIONS additions)

```
    --typosquat             Package-typosquat profile: five axes, four-way
                            classification, batch emits likely_typosquat only.
                            Default metric damerau. Cannot combine with -t.
    --pypi                  PEP 503 normalize (lower, map ._- → -, collapse -).
                            Identity-gate same-project names; otherwise score
                            normalized strings. Originals stay identifier keys.
-n, --normalize <PATH>      Append normalize ops from a JSON file (ordered
                            array of [op, …]). Repeatable. Not a preset name.
```

## Worked commands

```sh
# npm-style (no hyphen/underscore equivalence)
sqdist --typosquat lodash lodahs
sqdist --typosquat --string lodash --list new-packages.txt

# PyPI
sqdist --typosquat --pypi requests_toolbelt requests-toolbelt
# → classification same_project; batch would omit this row

sqdist --typosquat --pypi --string django --list new-pypi.txt

# custom rules
sqdist --typosquat -n ./rules.json --string foo --list names.txt
```
