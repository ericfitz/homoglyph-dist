# Design: sqdist `script_restriction` axis (Phase 2)

Date: 2026-05-24
Status: Approved
Component: `sqdist` — adds a 9th panel axis (`script_restriction`) reporting the
UTS#39 restriction level of a string pair, the direct mixed-script spoof signal
Phase 1 lacked. Builds on the v0.3.0 multi-axis architecture.

## Summary

Add one **base axis**, `script_restriction`, an `Int` carrying the UTS#39
restriction level (0–5) of the pair. It catches the `pаypal` class directly:
a string mixing Latin + Cyrillic has no consistent resolved script, so it scores
*high* (more suspicious) on this axis — independent of edit distance or
confusable skeletons. Computed via the `unicode-security` crate's validated
`RestrictionLevelDetection`. Purely additive: slots into `ALL_AXES`, and
`build_panel` / `--fields` / `--metric` / emit order pick it up automatically.
No verdict change, no new `AxisValue` variant.

Folded into the still-unreleased **v0.3.0** (nothing is published, so the version
just grows to include this axis). Adds sqdist's **first runtime dependency**.

## Decision: use the `unicode-security` crate (not DIY)

Owner-approved (2026-05-24). The UTS#39 restriction-level algorithm requires the
"augmented script set" resolution (augment each char's `Script_Extensions`; `Han`
adds synthetic `Jpan`/`Hanb`/`Kore`; `Common`/`Inherited` expand to all; intersect
across chars; then Latin-exclusion + Cyrillic/Greek special-casing). This is
subtle and security-critical — a wrong level is a missed spoof or a false alarm.
The `unicode-security` crate (unicode-rs org) implements exactly this, is the
basis of rustc's `mixed_script_idents` lint, and is `MIT/Apache-2.0` (compatible
with sqdist's `MIT OR Apache-2.0`). The DIY alternative (embedded `Scripts.txt` +
`ScriptExtensions.txt` table + hand-written intersection) was rejected: it
preserves the zero-dep property but reintroduces the exact edge-case risk the
crate eliminates. Binary size is reported, not gating.

**Trade-off accepted:** sqdist gains its first dependency tree
(`unicode-security` → `unicode-script` ~255 KB table + `unicode-normalization`),
losing the zero-dep / single-source-file purity it had through v0.3.0. The
`confusables_data.rs` embedded table (UTS#39 confusables) is unaffected — it
remains generated and embedded; only the script-level computation is delegated.

## The crate API (verified against unicode-security 0.1.2 source)

```rust
use unicode_security::{RestrictionLevel, RestrictionLevelDetection};

// RestrictionLevelDetection is implemented for &str:
let level: RestrictionLevel = some_str.detect_restriction_level();
```

`RestrictionLevel` is an ordered enum (derives `Ord`; lower = more restrictive =
safer), declared in this order:

| Variant | Ordinal | Meaning |
|---|---|---|
| `ASCIIOnly` | 0 | all chars ASCII |
| `SingleScript` | 1 | resolves to one script |
| `HighlyRestrictive` | 2 | Latin + one of Jpan/Kore/Hanb |
| `ModeratelyRestrictive` | 3 | Latin + one recommended non-Cyrillic/Greek script |
| `MinimallyRestrictive` | 4 | anything else not unrestricted |
| `Unrestricted` | 5 | mixed-script with no consistent resolution, or a char outside the general security profile |

The crate's `detect_restriction_level` for `&str` already performs the
Latin-exclusion pass and the Cyrillic/Greek demotion the research flagged as
easy to get wrong by hand — confirming the crate decision.

**Behavioral nuance (important, verified):** `pаypal` (Latin `p,y,p,a,l` +
Cyrillic `а`) does NOT resolve to a low level — the Latin∩Cyrillic resolved set
is empty, and Cyrillic is excluded from the moderate tier, so it lands at
`MinimallyRestrictive` (4) / `Unrestricted` (5). That is the desired signal:
**a homoglyph spoof scores high.** Conversely legitimate Japanese
(Han+Hiragana+Katakana) collapses to `HighlyRestrictive` (2), NOT inflated — the
crate's augmented-set logic handles it.

## The axis

| Key | Type | Direction | Phase | Definition |
|---|---|---|---|---|
| `script_restriction` | Int | HigherMoreDifferent | base | `max(ordinal(detect_restriction_level(a)), ordinal(detect_restriction_level(b)))` |

Design choices (all owner-aware):

- **Per-pair via `max`.** The crate is per-string; sqdist's panel is per-pair.
  `max` reports the more-suspicious of the two strings: if either side is
  mixed-script, the pair is flagged. (Rationale: in watchlist/typosquat use, the
  candidate being scored is the suspect; taking the max means a clean target vs.
  a mixed-script candidate still surfaces the candidate's level.)
- **Name `script_restriction`** (the research's "script-count" was a working
  title; the value is a *level*, not a count).
- **Ordinal via explicit `match`**, not `enum as u64` — robust to any future
  enum reordering and self-documenting. A pure helper
  `fn level_ordinal(level: RestrictionLevel) -> u64` maps the 6 variants to 0..=5.
- **Direction: HigherMoreDifferent** (higher level = more suspicious), consistent
  with the edit-distance axes. It is a numeric axis, so it is a valid `--metric`
  target and `numeric_keys()` includes it automatically.

### Canonical position

Appended **last** in `ALL_AXES`, after `confusable_only`. The existing 8-key
emit/JSON order is completely undisturbed; `script_restriction` simply appears as
the final key. New canonical order:

```
equal, levenshtein, damerau, skeleton_levenshtein, skeleton_damerau,
uts39_confusable_count, uts39_skeleton_delta, confusable_only, script_restriction
```

## What is NOT in scope (deferred)

- **No verdict change.** The verdict still reads only the ported Phase-1 signals
  (equal/damerau/skeleton_damerau/uts39_skeleton_delta/confusable_only). Weighing
  `script_restriction` into the spoof rule is deferred (the v0.3.0 architecture
  spec explicitly defers "any verdict redesign that weighs the new axes"). This
  axis only appears in the panel output, `--fields`, and as a `--metric` target.
- **No `AxisValue::NA`.** Every string has a defined restriction level; this axis
  is always present. (The N/A representation remains a future need for the
  Phase 3 keyboard-distance axis, untouched here.)
- **No per-string output.** The panel is per-pair; we report the combined `max`,
  not the two individual levels. (If a future need arises, a per-string variant
  is a clean additive follow-up.)
- **No `--layout`/configuration.** The restriction level is layout-independent.

## File layout / change set

| File | Change |
|---|---|
| `Cargo.toml` | add `[dependencies]` section: `unicode-security = "0.1.2"`. Version stays `0.3.0`. |
| `Cargo.lock` | regenerated by `cargo build` (adds unicode-security + transitive `unicode-script`, `unicode-normalization`). |
| `src/axes.rs` | add `use unicode_security::{RestrictionLevel, RestrictionLevelDetection};`; add `level_ordinal` helper; add `struct ScriptRestriction` + `impl Axis` (base, HigherMoreDifferent, computes the `max`); append `&ScriptRestriction` to `ALL_AXES`; add unit tests. |
| `src/main.rs` | `--help` AXES block: add a `script_restriction` line. |
| `CLAUDE.md`, `README.md` | document the new axis + the new `unicode-security` dependency (and that sqdist is no longer zero-dep). |

The axis reads `ctx.a`/`ctx.b` (the original `&str`s already on `PairContext`) —
no new precompute needed, so `PairContext` is unchanged. (It does NOT use the
char vecs or skeletons; restriction level is computed from the raw strings.)

## Testing

Unit tests in `src/axes.rs` (via the existing `run(a, b) -> Panel` helper):

- ASCII pair (`paypal`/`google`) → `script_restriction` == 0.
- Latin+Cyrillic homoglyph (`paypal`/`pаypal`, Cyrillic а) → high (assert `>= 4`;
  the spoof scores suspicious). This is the headline case.
- Pure single-script non-Latin pair (two Greek words, e.g. `αβγ`/`αβδ`) → 1
  (`SingleScript`).
- Legitimate Japanese single string vs itself (e.g. a Han+Hiragana mix) → 2
  (`HighlyRestrictive`), NOT inflated — guards the documented FP-avoidance.
- `max` behavior: pair a level-0 ASCII string with a mixed-script string → the
  pair takes the higher level (assert the result equals the mixed string's level,
  not 0).
- Direction: `ALL_AXES` lookup of `script_restriction` → `HigherMoreDifferent`.
- `level_ordinal`: all 6 variants map to 0..=5 in order (a direct table test).
- Registry: `ALL_AXES` now has 9 axes, `script_restriction` last; `panel` emits
  it last; `numeric_keys()`/`validate_metric("script_restriction")` accept it.
- `--metric script_restriction` accepted (numeric); end-to-end via `parse_from`.

Plus the standing gates: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check` all clean. (No `#[allow(dead_code)]` needed — the axis is
consumed by the binary via `ALL_AXES`/`build_panel` immediately.)

## Docs

- README/CLAUDE: add `script_restriction` to the axis list with its meaning and
  the HigherMoreDifferent reading; note it is the direct mixed-script signal.
- Note prominently that sqdist now has a runtime dependency (`unicode-security`,
  MIT/Apache-2.0) and is no longer a zero-dependency build; the embedded
  confusables table is still embedded/generated.

## Risks / notes

- **First dependency** — accepted per the decision above. Reversible (swap to DIY
  later) pre-1.0 if the dep ever becomes a problem.
- **Crate's Unicode version** may differ from the `confusables.txt` version baked
  into `confusables_data.rs` (currently 17.0.0). This is acceptable: the two data
  sets answer different questions (script membership vs. confusable skeletons) and
  need not be version-locked. Document the crate's `UNICODE_VERSION` is whatever
  ships with `unicode-security 0.1.2`; not gating.
- **`detect_restriction_level` consumes `self`** (takes `self: &str` by value, but
  `&str` is `Copy`), so calling it on `ctx.a` is fine and non-destructive.
