# Design: sqdist `keyboard_distance` axis + `AxisValue::NA` (Phase 3)

Date: 2026-05-24
Status: Approved
Component: `sqdist` — adds a 10th panel axis (`keyboard_distance`) measuring
physical QWERTY key proximity of substituted characters, plus the `AxisValue::NA`
("not applicable") variant the multi-axis architecture flagged as a future need.
Builds on v0.3.0.

## Summary

Add one **base axis**, `keyboard_distance`: the mean physical key distance over
the *substituted* positions in the Damerau alignment, on a stagger-aware
US-QWERTY coordinate grid, normalized to [0,1]. Near 0 = substitutions are
adjacent-key fat-finger typos (benign); near 1 = far-apart keys (deliberate
substitution, more suspicious). It is **undefined for non-ASCII input**, so this
phase also adds `AxisValue::NA`, rendered as JSON `null` / human `n/a`.

Self-contained: a hand-built embedded key-coordinate table (`src/keyboard.rs`),
**no new dependency**. Reuses the existing `ctx.align` traceback. No verdict
change. Folded into the still-unreleased **v0.3.0** (nothing published).

## Motivation

The panel measures *whether* characters differ (edit distances) and *how* they
differ visually (skeletons, confusable count, restriction level). It does not yet
measure *keyboard plausibility*: `gogle`/`google` (a dropped letter) vs.
`gpogle`/`google` (g↔p, opposite ends of the keyboard) vs. `gigle`/`google`
(o↔i, adjacent). A substitution between adjacent keys is a likely fat-finger
typo; a substitution between distant keys is more likely deliberate. This axis
quantifies that as a [0,1] signal, complementing the visual axes.

## The axis

| Key | Type | Direction | Phase | Definition |
|---|---|---|---|---|
| `keyboard_distance` | Float \| NA | HigherMoreDifferent | base | mean Euclidean key-distance over substituted alignment positions, ASCII-folded, normalized to [0,1]; **NA** when either string contains a non-ASCII char |

### Computation

Given `ctx` (the `PairContext`, which already holds `ca`, `cb`, and `align`):

1. **Non-ASCII guard (first).** If `ctx.a` or `ctx.b` contains any non-ASCII
   char (`!c.is_ascii()` for any char), return `AxisValue::NA`. Keyboard distance
   is undefined for non-ASCII; emitting 0.0 would falsely imply key proximity.
2. Otherwise, walk `ctx.align`, collecting the `AlignOp::Sub(i, j)` positions.
   For each, look up the key coordinates of `fold(ca[i])` and `fold(cb[j])` and
   take their Euclidean distance, where `fold(c)` = ASCII-lowercase (case shares a
   physical key) — see "Unmappable chars" for chars absent from the table.
3. **Zero substitutions** (identical strings, or differences that are purely
   insertions/deletions/transpositions) → `AxisValue::Float(0.0)`. This is
   **accurate, not a sentinel**: the mean key displacement over zero substituted
   positions is genuinely 0. NA is reserved strictly for "cannot compute"
   (non-ASCII); 0.0 means "computed, no substitution displacement."
4. Otherwise return `AxisValue::Float(mean_distance / MAX_KEY_DISTANCE)`, clamped
   to [0,1], where `MAX_KEY_DISTANCE` is the maximum pairwise distance between any
   two keys in the table (so the normalized value reaches but never exceeds 1.0).

Direction: HigherMoreDifferent (far-apart keys = more suspicious). It is a numeric
axis (Float), so it is a valid `--metric` target and `numeric_keys()` includes it
automatically. (A given *pair* may yield NA, which `as_f64()` maps to `None` —
see the `AxisValue::NA` section for how `-t`/`--sort` handle that.)

## The keyboard model — `src/keyboard.rs` (new module, no dependency)

A hand-built US-QWERTY coordinate table. Physical key positions are factual (not
copyrightable); `clavier` (MIT) and dnstwist (Apache-2.0) consulted only as
cross-references — the table is our own.

```rust
//! Stagger-aware US-QWERTY key coordinates for the keyboard_distance axis.
//! Physical key positions are factual data. Coordinates are in "key units";
//! row stagger offsets approximate a standard staggered keyboard.

/// (x, y) of a key in key-units. Returns None for chars not on the modelled
/// keyboard (after the caller has ASCII-lowercased).
pub fn key_coord(c: char) -> Option<(f32, f32)>;

/// The maximum Euclidean distance between any two modelled keys — the
/// normalizer so keyboard_distance lands in [0,1]. Computed once.
pub fn max_key_distance() -> f32;
```

- **Rows & stagger** (y = row, x = column + row stagger offset, per the research):
  - number row `1234567890` and reachable unshifted symbols (`-`, `=`): y=0, x-offset 0.0
  - top row `qwertyuiop` (+ `[`, `]`): y=1, x-offset +0.5
  - home row `asdfghjkl` (+ `;`, `'`): y=2, x-offset +0.75
  - bottom row `zxcvbnm` (+ `,`, `.`, `/`): y=3, x-offset +1.25
  - Columns are integer x positions left-to-right within a row; final coordinate
    is `(col + row_offset, row)`.
- **Coverage:** lowercase letters a–z, digits 0–9, and the common unshifted
  punctuation listed above (~47 keys). Uppercase folds to lowercase before lookup
  (Computation step 2). Shifted symbols (`!@#…`) are NOT separately modelled in v1
  — they share their key's position via the unshifted char only if typed as the
  unshifted char; a shifted symbol char itself is "unmappable" (below).
- **Normalizer:** `max_key_distance()` returns the largest pairwise key distance
  (roughly the diagonal corner-to-corner span). Implemented as a function that
  computes it from the table (a small fixed loop) — may be cached via
  `std::sync::OnceLock` or simply recomputed (the table is ~47 keys; recompute is
  cheap). Implementer's choice; it must be deterministic.
- **Unmappable chars:** an ASCII char absent from the table (e.g. a control char,
  or a shifted-symbol codepoint like `!`) — after the non-ASCII guard these are
  rare. Rule: **skip that substitution** from the mean (don't count it). If ALL
  substitutions are unmappable, the mean is over zero usable positions → return
  `Float(0.0)` (same as the zero-substitution case). Documented as a known small
  edge; the axis targets ordinary typeable identifiers.

`key_coord` and `max_key_distance` are pure and independently unit-tested.

## `AxisValue::NA` (the cross-cutting addition)

Add a fourth variant to the existing enum:

```rust
pub enum AxisValue {
    Int(u64),
    Float(f64),
    Bool(bool),
    NA, // "not applicable" — the axis is undefined for this pair
}
```

Behavior at each consumer (all exhaustive `match`es over `AxisValue` live in
`src/axes.rs` — only `to_json` and `as_f64`; `to_human` delegates to `to_json`):

- **`to_json()`** → `"null"` (unquoted JSON null). So the field is always present:
  `"keyboard_distance":null`. The key set stays stable across all rows (every
  record carries all axis keys) — the invariant downstream parsers rely on.
- **`to_human()`** → `"n/a"`. Human output should read `n/a`, not `null`. Today
  `to_human` blindly delegates to `to_json` for every variant; change it to
  special-case `AxisValue::NA => "n/a".to_string()` and delegate to `to_json` for
  Int/Float/Bool. This is the single behavioral change to `to_human`.
- **`as_f64()`** → `None`. NA has no numeric value.
- **`--metric keyboard_distance` + `-t` + `--sort`:** `metric_value(panel, key)`
  returns `None` for an NA row (via `as_f64`), and the existing `row_metric`
  helper in `main.rs` already maps `None` → `f64::INFINITY`. Semantics: an NA row
  sorts to the END (least "keyboard-typo-like") and is NEVER under a `-t`
  threshold. This is correct — a non-ASCII pair is not a keyboard-fat-finger
  match — and requires **NO change** to `row_metric`/`sort_and_truncate`/threshold
  logic. `validate_metric("keyboard_distance")` still accepts it (it is a
  numeric-typed axis; NA is a per-pair runtime outcome, not a type property).

No other `AxisValue` consumer exists, so the variant addition is a localized,
compiler-checked change (the two `match`es force handling).

## Canonical position

Appended **last** in `ALL_AXES`, after `script_restriction`. Existing 9-key order
undisturbed. New order:

```
equal, levenshtein, damerau, skeleton_levenshtein, skeleton_damerau,
uts39_confusable_count, uts39_skeleton_delta, confusable_only,
script_restriction, keyboard_distance
```

## What is NOT in scope (deferred)

- **No verdict change.** The verdict still reads only the ported Phase-1 signals;
  weighing `keyboard_distance` is deferred (architecture spec defers verdict
  redesign for new axes).
- **No `--layout` flag.** US-QWERTY only in v1. AZERTY/QWERTZ/Dvorak are a future
  extension (the "Smörgåsbord" paper shows attackers exploit other layouts, but
  that is a later axis option, not v1).
- **No shifted-symbol modelling.** Shifted chars are unmappable in v1 (handled by
  the skip rule). Adding them is a future table extension.
- **No bigram/path or fat-finger-probability model.** The continuous
  Euclidean-on-coordinates mean is the agreed model; probabilistic confusion
  matrices were surveyed and rejected (corpus-dependent, heavy).

## File layout / change set

| File | Change |
|---|---|
| `src/keyboard.rs` | NEW. `key_coord(char) -> Option<(f32,f32)>`, `max_key_distance() -> f32`, the embedded table, + unit tests. |
| `src/main.rs` | add `mod keyboard;`; add a `keyboard_distance` line to the `--help` AXES block. |
| `src/axes.rs` | add `AxisValue::NA` + handle it in `to_json`/`to_human`/`as_f64`; add `struct KeyboardDistance` + `impl Axis` (base, HigherMoreDifferent, computes per "Computation"); append `&KeyboardDistance` to `ALL_AXES`; unit tests. |
| `README.md`, `CLAUDE.md` | document the axis (incl. the NA/non-ASCII behavior) and the new module. |
| `Cargo.toml` | unchanged (no new dependency; version stays 0.3.0). |

`PairContext` is unchanged — the axis reads `ctx.a`/`ctx.b` (non-ASCII guard),
`ctx.ca`/`ctx.cb` (the substituted chars), and `ctx.align` (the substitution
positions), all already present.

## Testing

`src/keyboard.rs` unit tests:
- `key_coord` returns coordinates for sample keys; adjacent keys (e.g. `s`/`d`)
  are closer than distant keys (`q`/`p`); case is irrelevant only via the caller
  (key_coord itself takes the already-lowercased char — test lowercase inputs).
- Unmappable char (e.g. `!` or a control char) → `None`.
- `max_key_distance()` is positive and ≥ the distance between the two farthest
  sampled keys; deterministic across calls.

`src/axes.rs` unit tests (via the existing `run(a,b) -> Panel` helper):
- Adjacent-key substitution (`gigle`/`google`: o↔i adjacent) → small Float
  (assert `< 0.3`, > 0.0).
- Distant-key substitution (`gpogle`/`google`: o↔p... pick a genuinely distant
  pair, e.g. `qogle`/`google` q↔g) → larger Float (assert `> ` the adjacent case).
- Identical pair (`google`/`google`) → `Float(0.0)` (zero substitutions = accurate
  zero displacement, NOT NA).
- Pure insert/delete (`gogle`/`google`) → `Float(0.0)` (no substitutions).
- Non-ASCII either side (`paypal`/`pаypal` Cyrillic а) → `AxisValue::NA`.
- `AxisValue::NA` rendering: `to_json()` == `"null"`; `to_human()` == `"n/a"`;
  `as_f64()` == `None`.
- Result is always in [0,1] for ASCII pairs.
- Registry: `ALL_AXES` has 10 axes, `keyboard_distance` last; panel emits it last.
- Direction HigherMoreDifferent; `validate_metric("keyboard_distance")` accepted;
  `numeric_keys()` contains it.

`src/main.rs` (or via existing test patterns):
- `result_json` for a non-ASCII pair contains `"keyboard_distance":null` (key
  present, value null).
- `--metric keyboard_distance` threshold/sort: an NA row does not match a finite
  `-t` and sorts after finite rows (the `row_metric` → INFINITY path). A small
  test asserting `row_metric(&na_panel, "keyboard_distance")` is `f64::INFINITY`.

Standing gates: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check` all clean. NO `#[allow(dead_code)]` — the axis and module are
consumed immediately via `ALL_AXES`.

## Docs

- README/CLAUDE: add `keyboard_distance` to the axis list with its meaning, the
  [0,1] range, HigherMoreDifferent reading, and the **NA-for-non-ASCII** behavior
  (and that NA renders as JSON `null` / human `n/a`). Note it reuses the QWERTY
  table in `src/keyboard.rs` and adds no dependency.
- Document that `AxisValue` now has an NA variant and what `null`/`n/a` mean in
  output.

## Risks / notes

- **Normalization choice** (`/ max_key_distance`) makes the [0,1] scale
  table-relative; fine for a within-tool signal. Documented.
- **The NA → INFINITY metric semantics** mean `--sort` on `keyboard_distance`
  pushes all non-ASCII pairs to the end. That is intended (they're not keyboard
  typos) and consistent with "lower metric = more suspicious" ordering.
- **`to_human` change** is the one place a previously-total delegation becomes a
  one-arm special-case; tests pin both `null` (json) and `n/a` (human).
