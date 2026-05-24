# keyboard_distance Axis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 10th panel axis, `keyboard_distance` (mean QWERTY key distance over substituted alignment positions, [0,1], NA for non-ASCII), plus the `AxisValue::NA` variant — using a self-contained embedded key table, no new dependency.

**Architecture:** A new `src/keyboard.rs` holds a stagger-aware US-QWERTY `char → (f32,f32)` table (`key_coord`) + a normalizer (`max_key_distance`). A new base `Axis` in `src/axes.rs` reads `ctx.align`'s `Sub(i,j)` positions, looks up ASCII-folded key coords, and reports the normalized mean distance — or `AxisValue::NA` when either string has non-ASCII. The new `AxisValue::NA` variant renders as JSON `null` / human `n/a`; `as_f64()` → `None`, so the existing `row_metric` INFINITY fallback already gives correct `-t`/`--sort` semantics. Appended last in `ALL_AXES`.

**Tech Stack:** Rust 2021, std-only (no new dependency). `cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`.

---

## Context for the implementer (read before starting)

You are adding ONE axis + one enum variant + one small module to `sqdist`, a Rust CLI scoring string pairs for typosquat/homoglyph detection. **Read first:**
- `docs/superpowers/specs/2026-05-24-keyboard-distance-axis-design.md` — the approved spec (source of truth).
- `CLAUDE.md` — conventions.
- `src/axes.rs` — the axis panel. `AxisValue` enum (`Int(u64)`/`Float(f64)`/`Bool(bool)`) with `to_json`/`to_human`/`as_f64`. Each axis = unit struct + `impl Axis` (`key`/`direction`/`phase`/`compute`). `ALL_AXES: &[&dyn Axis]` registry (currently 9 axes). `PairContext` has `pub a/b: &str`, `pub ca/cb: Vec<char>`, `pub align: Vec<AlignOp>`. `AlignOp::Sub(i,j)` indexes into ca/cb.

**Current `AxisValue` (src/axes.rs ~line 13):**
```rust
pub enum AxisValue {
    Int(u64),
    #[allow(dead_code)] // Float-valued axes arrive in Phase 3 (keyboard-distance)
    Float(f64),
    Bool(bool),
}
```
`to_json` and `as_f64` `match` exhaustively over it; `to_human` just delegates to `to_json`. **Phase 3 IS now**, so the `#[allow(dead_code)]` on `Float` MUST be removed in this work (the new axis produces `Float`).

**The axis:** key `keyboard_distance`, type Float|NA, `Direction::HigherMoreDifferent`, `Phase::Base`. Compute:
1. If `ctx.a` or `ctx.b` has any non-ASCII char → `AxisValue::NA`.
2. Else collect `AlignOp::Sub(i,j)` from `ctx.align`; for each, Euclidean distance between `key_coord(ascii_lower(ca[i]))` and `key_coord(ascii_lower(cb[j]))`, skipping any sub where either char is unmappable (`key_coord` None).
3. Zero usable substitutions (identical / pure ins-del / all unmappable) → `AxisValue::Float(0.0)` — accurate zero displacement, NOT NA.
4. Else `AxisValue::Float((mean_distance / max_key_distance()).clamp(0.0, 1.0))`.

Append `&KeyboardDistance` LAST in `ALL_AXES` (after `&ScriptRestriction`).

**Working rules (CLAUDE.md):**
- TDD: failing test → see it fail → implement → see it pass → commit.
- Before EVERY commit: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — all clean.
- Conventional Commits; end every message with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
- Commit directly to `main`. Do NOT push/release. Do NOT bump version (stays 0.3.0).
- No `#[allow(dead_code)]` for the new code (consumed immediately via `ALL_AXES`). And REMOVE the existing `Float` allow (now consumed).

---

## Task 1: Add `src/keyboard.rs` (key table + normalizer)

**Files:**
- Create: `src/keyboard.rs`
- Modify: `src/main.rs` (add `mod keyboard;`)

- [ ] **Step 1: Create `src/keyboard.rs` with the table, functions, and tests**

```rust
//! Stagger-aware US-QWERTY key coordinates for the keyboard_distance axis.
//!
//! Physical key positions are factual data (not copyrightable); `clavier` (MIT)
//! and dnstwist (Apache-2.0) were consulted only as cross-references. Coordinates
//! are in "key units": x = column index + a per-row stagger offset, y = row index
//! (number row 0 .. bottom row 3). Self-contained; no dependency.

/// (x, y) of a key in key-units, or None for chars not on the modelled keyboard.
/// Callers pass an already-ASCII-lowercased char (case shares a physical key).
pub fn key_coord(c: char) -> Option<(f32, f32)> {
    // (row chars, y, x stagger offset). Columns are 0-based within the row.
    const ROWS: &[(&str, f32, f32)] = &[
        ("1234567890-=", 0.0, 0.0),
        ("qwertyuiop[]", 1.0, 0.5),
        ("asdfghjkl;'", 2.0, 0.75),
        ("zxcvbnm,./", 3.0, 1.25),
    ];
    for &(chars, y, off) in ROWS {
        if let Some(col) = chars.chars().position(|k| k == c) {
            return Some((col as f32 + off, y));
        }
    }
    None
}

/// The maximum Euclidean distance between any two modelled keys — the normalizer
/// so keyboard_distance lands in [0,1]. Deterministic; computed from the table.
pub fn max_key_distance() -> f32 {
    const ALL: &str = "1234567890-=qwertyuiop[]asdfghjkl;'zxcvbnm,./";
    let coords: Vec<(f32, f32)> = ALL.chars().filter_map(key_coord).collect();
    let mut max = 0.0f32;
    for (i, &(x1, y1)) in coords.iter().enumerate() {
        for &(x2, y2) in &coords[i + 1..] {
            let d = ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
            if d > max {
                max = d;
            }
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: char, b: char) -> f32 {
        let (x1, y1) = key_coord(a).unwrap();
        let (x2, y2) = key_coord(b).unwrap();
        ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt()
    }

    #[test]
    fn known_keys_have_coords() {
        assert!(key_coord('q').is_some());
        assert!(key_coord('m').is_some());
        assert!(key_coord('0').is_some());
        assert!(key_coord('/').is_some());
    }

    #[test]
    fn unmappable_chars_are_none() {
        assert!(key_coord('!').is_none()); // shifted symbol, not modelled
        assert!(key_coord('\u{0007}').is_none()); // control char
        assert!(key_coord(' ').is_none()); // space not modelled
    }

    #[test]
    fn adjacent_closer_than_distant() {
        // s/d are home-row neighbours; q/p are top-row opposite ends.
        assert!(dist('s', 'd') < dist('q', 'p'));
        // f/g adjacent vs a/l far apart on the home row.
        assert!(dist('f', 'g') < dist('a', 'l'));
    }

    #[test]
    fn max_key_distance_is_positive_and_bounds_pairs() {
        let m = max_key_distance();
        assert!(m > 0.0);
        // every modelled pair is <= the max.
        assert!(dist('q', 'p') <= m);
        assert!(dist('1', '/') <= m);
        // deterministic across calls.
        assert_eq!(max_key_distance(), m);
    }
}
```

- [ ] **Step 2: Wire the module and run**

In `src/main.rs`, add `mod keyboard;` with the other module declarations (alphabetical placement among `mod axes; mod confusables_data; mod distance; mod verdict;` is fine — i.e. after `mod distance;` / before `mod verdict;`, or wherever keeps them sorted).

NOTE: `keyboard::key_coord`/`max_key_distance` are not consumed by the binary YET (the axis in Task 2 consumes them). To avoid a binary-target dead-code failure under `-D warnings` for THIS commit, do Task 1 and Task 2 BACK-TO-BACK and commit them together as ONE commit (the axis consumes the module, so nothing is dead). Therefore: implement Task 1's file now, but do NOT commit yet — proceed to Task 2 and make a single combined commit at the end of Task 2. (Run the keyboard tests now to confirm they pass: `cargo test --lib keyboard`.)

Run: `cargo test --lib keyboard`
Expected: the 4 keyboard tests pass. (Do not run the full clippy gate yet — `key_coord`/`max_key_distance` are unused by the binary until Task 2; that's expected and resolved by committing Task 1+2 together.)

(No commit in this task — see Task 2.)

---

## Task 2: Add `AxisValue::NA` + the `keyboard_distance` axis (single commit with Task 1)

**Files:**
- Modify: `src/axes.rs`
- (commits together with the `src/keyboard.rs` + `mod keyboard;` from Task 1)

- [ ] **Step 1: Write the failing tests in `src/axes.rs`**

Add to the EXISTING `tests` module (has `use super::*;` and `run(a,b)->Panel`):

```rust
    #[test]
    fn axis_value_na_renders() {
        assert_eq!(AxisValue::NA.to_json(), "null");
        assert_eq!(AxisValue::NA.to_human(), "n/a");
        assert_eq!(AxisValue::NA.as_f64(), None);
    }

    #[test]
    fn keyboard_distance_identical_is_zero() {
        // No substitutions -> accurate zero displacement (NOT NA).
        assert_eq!(run("google", "google").get("keyboard_distance"), Some(AxisValue::Float(0.0)));
    }

    #[test]
    fn keyboard_distance_pure_insert_delete_is_zero() {
        // gogle vs google: one deletion, no substitutions -> 0.0.
        assert_eq!(run("gogle", "google").get("keyboard_distance"), Some(AxisValue::Float(0.0)));
    }

    #[test]
    fn keyboard_distance_non_ascii_is_na() {
        // Cyrillic а -> axis undefined -> NA.
        assert_eq!(run("paypal", "p\u{0430}ypal").get("keyboard_distance"), Some(AxisValue::NA));
    }

    #[test]
    fn keyboard_distance_adjacent_less_than_distant() {
        // Adjacent-key sub (o->i, both top row neighbours) vs distant (o->q).
        let adjacent = match run("gigle", "gogle").get("keyboard_distance") {
            Some(AxisValue::Float(f)) => f,
            other => panic!("expected Float, got {other:?}"),
        };
        let distant = match run("gqgle", "gogle").get("keyboard_distance") {
            Some(AxisValue::Float(f)) => f,
            other => panic!("expected Float, got {other:?}"),
        };
        assert!(adjacent > 0.0, "adjacent sub should be > 0, got {adjacent}");
        assert!(adjacent < distant, "adjacent ({adjacent}) should be < distant ({distant})");
        assert!(distant <= 1.0, "normalized distance must be <= 1, got {distant}");
    }

    #[test]
    fn keyboard_distance_is_last_and_registry_has_ten() {
        let keys: Vec<&str> = ALL_AXES.iter().map(|ax| ax.key()).collect();
        assert_eq!(keys.len(), 10);
        assert_eq!(keys.last(), Some(&"keyboard_distance"));
    }

    #[test]
    fn keyboard_distance_direction_and_numeric() {
        let ax = ALL_AXES.iter().find(|ax| ax.key() == "keyboard_distance").unwrap();
        assert_eq!(ax.direction(), Direction::HigherMoreDifferent);
        assert!(validate_metric("keyboard_distance").is_ok());
        assert!(numeric_keys().contains(&"keyboard_distance"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib keyboard_distance 2>&1 | head -20`
Expected: FAIL — `AxisValue::NA` undefined and no `keyboard_distance` axis (compile errors + missing key).

- [ ] **Step 3: Add the `NA` variant and update the three methods**

In `src/axes.rs`, change the `AxisValue` enum: REMOVE the `#[allow(dead_code)]` line above `Float` (it's now consumed) and ADD the `NA` variant:

```rust
pub enum AxisValue {
    Int(u64),
    Float(f64),
    Bool(bool),
    /// The axis is not applicable to this pair (e.g. keyboard_distance on
    /// non-ASCII input). Renders as JSON null / human "n/a".
    NA,
}
```

Update `to_json` to handle `NA`:
```rust
    pub fn to_json(self) -> String {
        match self {
            AxisValue::Int(n) => n.to_string(),
            AxisValue::Float(f) => format!("{f:.4}"),
            AxisValue::Bool(b) => b.to_string(),
            AxisValue::NA => "null".to_string(),
        }
    }
```

Change `to_human` to special-case `NA` (it currently just delegates to `to_json`):
```rust
    pub fn to_human(self) -> String {
        match self {
            AxisValue::NA => "n/a".to_string(),
            other => other.to_json(),
        }
    }
```

Update `as_f64` to handle `NA`:
```rust
    pub fn as_f64(self) -> Option<f64> {
        match self {
            AxisValue::Int(n) => Some(n as f64),
            AxisValue::Float(f) => Some(f),
            AxisValue::Bool(_) => None,
            AxisValue::NA => None,
        }
    }
```

- [ ] **Step 4: Add the axis + register it**

Add to `src/axes.rs` immediately AFTER the `ScriptRestriction` axis impl and BEFORE `pub static ALL_AXES`:

```rust
struct KeyboardDistance;
impl Axis for KeyboardDistance {
    fn key(&self) -> &'static str {
        "keyboard_distance"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreDifferent
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        // Undefined for non-ASCII: emitting 0.0 would falsely imply key proximity.
        if !ctx.a.is_ascii() || !ctx.b.is_ascii() {
            return AxisValue::NA;
        }
        // Mean Euclidean key distance over substituted alignment positions,
        // ASCII-folded (case shares a key), skipping any unmappable char.
        let mut sum = 0.0f32;
        let mut count = 0u32;
        for op in &ctx.align {
            if let crate::distance::AlignOp::Sub(i, j) = op {
                let ca = ctx.ca[*i].to_ascii_lowercase();
                let cb = ctx.cb[*j].to_ascii_lowercase();
                if let (Some((x1, y1)), Some((x2, y2))) =
                    (crate::keyboard::key_coord(ca), crate::keyboard::key_coord(cb))
                {
                    sum += ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
                    count += 1;
                }
            }
        }
        // Zero usable substitutions: accurate zero displacement (not NA).
        if count == 0 {
            return AxisValue::Float(0.0);
        }
        let mean = sum / count as f32;
        let normalized = (mean / crate::keyboard::max_key_distance()).clamp(0.0, 1.0);
        AxisValue::Float(normalized as f64)
    }
}
```

Append `&KeyboardDistance` as the LAST entry of `ALL_AXES`:
```rust
pub static ALL_AXES: &[&dyn Axis] = &[
    &Equal,
    &Levenshtein,
    &Damerau,
    &SkeletonLevenshtein,
    &SkeletonDamerau,
    &Uts39ConfusableCount,
    &Uts39SkeletonDelta,
    &ConfusableOnly,
    &ScriptRestriction,
    &KeyboardDistance,
];
```

Do NOT touch `is_bool_axis` (keyboard_distance is numeric — `numeric_keys`/`validate_metric` include it automatically).

- [ ] **Step 5: Fix the two registry tests that assert the old count**

Two Phase-1/2 tests assert the registry size/last key:
- `registry_canonical_order` — asserts the full key vec. Add `"keyboard_distance"` as the 10th element of its expected vec (after `"script_restriction"`).
- `panel_emits_in_registry_order` — change its `keys.last()` expectation to `Some(&"keyboard_distance")` and `keys.len()` to `10`.

(`parse_fields_all_keys` was hardened in Phase 2 to derive from `all_keys()`, so it needs NO change — it auto-covers the 10th key.)

- [ ] **Step 6: Run the full suite + gates**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: ALL pass (the existing suite + the new keyboard.rs tests + the new axes.rs tests). NO clippy warnings — in particular, removing the `Float` allow must NOT reintroduce a dead-code warning (the axis now produces `Float`). If clippy says `Float` is still dead, the axis isn't wired correctly — investigate; do NOT re-add the allow. fmt clean.

- [ ] **Step 7: Commit (Task 1 + Task 2 together)**

```bash
git add src/keyboard.rs src/main.rs src/axes.rs
git commit -m "$(cat <<'EOF'
feat(axes): add keyboard_distance axis and AxisValue::NA

New base axis: mean US-QWERTY key distance over substituted alignment
positions, normalized to [0,1], HigherMoreDifferent, appended last. NA
(JSON null / human n/a) when either string is non-ASCII; zero substitutions
= accurate 0.0. Self-contained key table in src/keyboard.rs, no dependency.
Adds the AxisValue::NA variant (handled in to_json/to_human/as_f64) and
drops the now-consumed Float dead-code allow. No verdict change.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: NA threshold/sort semantics test + `--help`/docs

**Files:**
- Modify: `src/main.rs` (a test + the `--help` AXES block)
- Modify: `README.md`, `CLAUDE.md`

- [ ] **Step 1: Add an NA-metric semantics test (TDD)**

The spec requires: an NA row's metric value is `f64::INFINITY` (via `row_metric`'s `unwrap_or`), so it never matches a finite `-t` and sorts last. Add to the `src/main.rs` `tests` module:

```rust
    #[test]
    fn na_metric_value_is_infinity() {
        // keyboard_distance is NA for a non-ASCII pair; row_metric falls back to
        // +inf so the row never matches a finite -t and sorts last.
        let panel = score_pair("paypal", "p\u{0430}ypal");
        assert!(row_metric(&panel, "keyboard_distance").is_infinite());
    }
```

Run: `cargo test na_metric_value_is_infinity` — should PASS immediately (the behavior already exists via `as_f64() -> None` + `row_metric`'s `unwrap_or(f64::INFINITY)`). This test PINS that contract. If it does NOT pass, stop and report — the NA wiring is wrong.

- [ ] **Step 2: Add `keyboard_distance` to the `--help` AXES block**

In `src/main.rs` `print_usage`, the AXES block's last line is now the `script_restriction` line ending with `\n\n\`. Change it to end with `\n\` and add a `keyboard_distance` line carrying the block-ending `\n\n\`:
```
         \x20   script_restriction      UTS#39 restriction level 0-5 (higher = more mixed-script/suspicious)\n\
         \x20   keyboard_distance       mean QWERTY key distance over substitutions, 0-1 (n/a if non-ASCII)\n\n\
```

Verify render:
Run: `cargo build && ./target/debug/sqdist --help 2>&1 | sed -n '/AXES:/,/OUTPUT KEYS/p'`
Expected: all 10 axes listed ending with `keyboard_distance ...`, columns aligned, no stray `\n`, then OUTPUT KEYS.

Also confirm end-to-end:
Run: `./target/debug/sqdist -j gigle gogle | grep -o '"keyboard_distance":[0-9.]*'` → a small float; and `./target/debug/sqdist -j paypal "p$(printf '\xd0\xb0')ypal" | grep -o '"keyboard_distance":[a-z]*'` → `"keyboard_distance":null`. Confirm it's the LAST key both times.

- [ ] **Step 3: Update README.md and CLAUDE.md**

Read both. In each, add `keyboard_distance` (last) to the axis list/table: "mean physical US-QWERTY key distance over the substituted positions, normalized to [0,1]; near 0 = adjacent-key fat-finger typo, near 1 = far-apart/deliberate. Direction: higher = more different/suspicious. **NA** (JSON `null`, human `n/a`) when either string contains a non-ASCII character (keyboard distance is undefined there); zero substitutions = 0.0 (accurate). Self-contained QWERTY table; no dependency." Match each file's style; bump axis-count mentions 9→10 and any test-count mention to the new total. In CLAUDE.md's file list, add `src/keyboard.rs` (the embedded QWERTY table). Document that `AxisValue` now has an `NA` variant → `null`/`n/a`.

- [ ] **Step 4: Gates + commit**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — all clean.
```bash
git add src/main.rs README.md CLAUDE.md
git commit -m "$(cat <<'EOF'
docs: document keyboard_distance axis + NA; pin NA metric semantics

Add keyboard_distance to the --help AXES block and README/CLAUDE axis
lists (incl. the NA-for-non-ASCII behavior, null/n/a rendering, src/keyboard.rs).
Add a test pinning that an NA row's metric value is +inf (never matches a
finite -t, sorts last).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Final verification

- [ ] **Step 1: Full gate**
```bash
cargo fmt --check                              # clean
cargo clippy --all-targets -- -D warnings      # no warnings
cargo test                                     # all pass (was 64; +4 keyboard +7 axes +1 main = 76)
cargo build --release                          # exit 0
./target/release/sqdist -v                     # sqdist 0.3.0 (<sha>); touch .git/HEAD first if SHA looks stale
```

- [ ] **Step 2: Behavioral spot-checks**
```bash
B=./target/release/sqdist; CYR=$(printf '\xd0\xb0')
$B -j gigle gogle | grep -o '"keyboard_distance":[0-9.]*'        # small float (adjacent o/i)
$B -j gqgle gogle | grep -o '"keyboard_distance":[0-9.]*'        # larger float (distant o/q)
$B -j google google | grep -o '"keyboard_distance":[0-9.]*'      # 0.0000 (identical)
$B -j paypal "p${CYR}ypal" | grep -o '"keyboard_distance":[a-z]*'# null (non-ASCII)
$B paypal "p${CYR}ypal" | grep keyboard_distance                 # human row shows "n/a"
$B -m keyboard_distance -t 0 google google >/dev/null; echo "identical -t0 exit=$? (expect 0)"
$B -m keyboard_distance -t 0 paypal "p${CYR}ypal" >/dev/null; echo "na -t0 exit=$? (expect 1: NA never matches)"
```
Confirm: adjacent < distant; identical = 0.0000; non-ASCII = null (json) / n/a (human); NA row never matches a finite threshold.

- [ ] **Step 3: Spec test-checklist cross-check** — confirm every item in the spec's Testing section has a passing test (keyboard.rs: coords/unmappable/adjacency/max; axes.rs: identical=0, ins/del=0, non-ASCII=NA, adjacent<distant, [0,1], NA rendering, registry-10, direction/numeric; main.rs: NA metric=inf, json null). Add any missing one (failing → fix → pass), commit separately.

- [ ] **Step 4: Report completion** — summarize; confirm gate passed; nothing pushed/released. In auto mode, continue to Phase 4 (supplemental confusables) unless the owner has said otherwise.

---

## Self-review notes (for the executor)

- **Spec coverage:** keyboard table+normalizer (T1); AxisValue::NA + the axis + registry (T2); NA metric semantics + --help + docs (T3); verification (T4). Deliberate non-actions (no verdict change, no --layout, no shifted symbols, no row_metric change) require NO task.
- **Watch-points:** (1) Tasks 1+2 commit TOGETHER — `keyboard.rs` is dead until the axis consumes it, so a standalone Task-1 commit would fail `-D warnings`. (2) REMOVE the `Float` `#[allow(dead_code)]` — the axis now produces Float; if clippy then flags Float dead, the wiring is wrong (don't re-add the allow). (3) The two registry tests (`registry_canonical_order`, `panel_emits_in_registry_order`) need the +10 update; `parse_fields_all_keys` does NOT (it derives from all_keys()). (4) `--help` string-literal edit is fragile (`\x20`/`\n\`); verify the render. (5) Don't bump version (stays 0.3.0). (6) `row_metric` is UNCHANGED — NA→None→INFINITY already works; T3 Step 1 just pins it.
- **Determinism:** `key_coord`/`max_key_distance` are pure; `f32` distance math is deterministic for fixed inputs. Tests use inequalities (adjacent < distant) and exact 0.0, avoiding float-equality fragility except the genuine 0.0 case.
