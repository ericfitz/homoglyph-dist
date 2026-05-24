# Multi-Axis Architecture Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `sqdist`'s single blended weighted-Damerau scoring model with an extensible panel of 8 independent similarity axes, split across focused modules, released as the breaking v0.3.0.

**Architecture:** An `Axis` is one independent similarity signal (`key`, `direction`, `compute`). A static `ALL_AXES` registry defines canonical emit order and replaces the old `Field` enum. A `PairContext` precomputes per-pair shared data (char vecs, skeletons, and a Damerau alignment traceback) once. The panel runs in two phases: base axes (pure functions of context) then derived axes (functions of base values). The verdict and all output (human + JSONL) read the uniform axis interface. `main.rs` shrinks to CLI/IO/dispatch.

**Tech Stack:** Rust 2021, std-only, `cargo test` (unit tests inline per module), `cargo clippy --all-targets`, `cargo fmt`. No new dependencies.

---

## Context for the implementer (read before starting)

You are refactoring `sqdist`, a single-binary Rust CLI that scores string pairs for typosquatting/homoglyph detection. **Read these first:**

- `docs/superpowers/specs/2026-05-23-multi-axis-architecture-design.md` — the approved spec this plan implements. It is the source of truth; this plan is its task breakdown.
- `CLAUDE.md` — project conventions.
- The current `src/main.rs` — everything lives in one file today (distance algorithms, the `Scores` struct, `Field` enum, `verdict`, arg parsing, three I/O modes, and ~30 tests). You will split it.

**What "axis" means:** one independent, raw similarity signal for a string pair (e.g. `levenshtein` = the unweighted edit distance; `equal` = whether the two strings are byte-identical). The v0.2.0 code blended homoglyph-awareness into a *weighted* Damerau (`hogl`) and exposed derived 0–1 scores (`normalized`). The redesign computes each signal independently and raw, so the verdict can reason over the full panel and new signals can be added without touching the core.

**The 8 axes you are building** (canonical order — this is the emit order for both JSON keys and human rows):

| # | Key | Type | Direction | Phase | Definition |
|---|---|---|---|---|---|
| 0 | `equal` | Bool | (n/a) | base | `a == b` |
| 1 | `levenshtein` | Int | HigherMoreDifferent | base | unweighted Levenshtein on the char vecs of a/b |
| 2 | `damerau` | Int | HigherMoreDifferent | base | unweighted Damerau (OSA, adjacent transpositions) on a/b |
| 3 | `skeleton_levenshtein` | Int | HigherMoreDifferent | base | Levenshtein on the UTS#39 skeletons of a/b |
| 4 | `skeleton_damerau` | Int | HigherMoreDifferent | base | Damerau (OSA) on the skeletons of a/b |
| 5 | `uts39_confusable_count` | Int | HigherMoreSimilar | base | # of substitution positions in the a→b Damerau alignment whose two chars are UTS#39-confusable |
| 6 | `uts39_skeleton_delta` | Int | HigherMoreSimilar | derived | `damerau.saturating_sub(skeleton_damerau)` (edits that vanish under skeletonization) |
| 7 | `confusable_only` | Bool | (n/a) | derived | `!equal && skeleton_levenshtein == 0` |

**Removed (breaking):** `homoglyph_damerau` (the weighted `hogl`), `normalized`, `skeleton_normalized`, the `--hogl-weight` flag, and the `Field` enum.

**Target module layout** (the binary crate root stays `src/main.rs`; all `mod` declarations live there):

| File | Responsibility |
|---|---|
| `src/distance.rs` | `levenshtein`, `damerau`, `skeleton`, `skeleton_of`, `confusable`, `AlignOp`, `align` (the traceback) |
| `src/axes.rs` | `AxisValue`, `Direction`, `Axis` trait, `PairContext`, the 8 axis impls, `ALL_AXES`, `Panel` (two-phase builder + map), axis-key parsing/selection for `--fields`, numeric-metric lookup |
| `src/verdict.rs` | `Verdict` enum + `verdict()` reading the panel |
| `src/confusables_data.rs` | unchanged (generated; `pub static CONFUSABLES`) |
| `src/main.rs` | `mod` decls, `Opts`, arg parsing, I/O, the three modes, output formatting (human + JSONL), orchestration |

**Working rules (from CLAUDE.md and the kickoff):**
- TDD: write the failing test, see it fail, implement, see it pass, commit. One logical change per commit.
- Before EVERY commit run all three and fix any issue: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. The tree is warning-clean today; keep it that way.
- Conventional Commits. End every commit message body with the trailer:
  `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`
- Commit directly to `main` (owner's standing consent). Do NOT cut a release or publish. If `git push` fails on the SSH key touch, stop and report — do not work around it.
- This is a large refactor. The strategy below keeps the crate compiling and green after every task by building the new modules first, then flipping `main.rs` over to them, then deleting the dead old code in the same flip commit.

**Build strategy (why the task order is what it is):** Tasks 1–6 add the new modules (`distance`, `axes`, `verdict`) alongside the existing `main.rs` code. Each new module is exercised by its own `#[cfg(test)]` tests so its public items are used. Task 7 rewires `main.rs` to use the new modules and **deletes** the old `Scores`/`Field`/`Metric`/`verdict`/weighted-distance code in the same commit, so there is never a committed state with unused non-test code. Verify `cargo clippy --all-targets -- -D warnings` is clean at each commit. If a brand-new public item trips dead-code/clippy before the task that consumes it, do the producing and consuming tasks back-to-back and, only if a single commit still won't pass `-D warnings`, attach a narrowly-scoped `#[allow(dead_code)]` with a `// consumed in Task N` comment and remove it in Task N. Never silence with a blanket allow.

---

## Task 1: Scaffold `distance.rs` with the existing pure functions

Move the existing distance/skeleton functions into a new module (now UNWEIGHTED integer versions; the weighted shim stays in `main.rs` until Task 7), wire up the module, and port their tests.

**Files:**
- Create: `src/distance.rs`
- Modify: `src/main.rs` (add `mod distance;`, relocate skeleton fns, rename the weighted distance fns)

- [ ] **Step 1: Create `src/distance.rs` with the relocated functions**

Create `src/distance.rs`. `skeleton_of`/`confusable`/`skeleton` move verbatim (made `pub`); `levenshtein`/`damerau` become UNWEIGHTED integer versions (no `bool`/`f64` params):

```rust
//! Edit-distance algorithms and the UTS#39 confusable-skeleton model.
//!
//! All distances here are UNWEIGHTED (integer): Levenshtein and Damerau
//! (OSA — adjacent transpositions). Homoglyph awareness is expressed by the
//! axes layer via skeletons and the alignment traceback, not by weighting the
//! edit cost.

use crate::confusables_data::CONFUSABLES;

/// Look up the confusable skeleton for a single char.
pub fn skeleton_of(c: char) -> Option<&'static str> {
    let cp = c as u32;
    CONFUSABLES
        .binary_search_by(|&(k, _)| k.cmp(&cp))
        .ok()
        .map(|i| CONFUSABLES[i].1)
}

/// Are two chars confusable under UTS#39 skeleton equality?
pub fn confusable(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let mut sa_buf = [0u8; 4];
    let mut sb_buf = [0u8; 4];
    let sa = skeleton_of(a).unwrap_or_else(|| a.encode_utf8(&mut sa_buf));
    let sb = skeleton_of(b).unwrap_or_else(|| b.encode_utf8(&mut sb_buf));
    sa == sb
}

/// UTS#39 skeleton of a string: map each code point through the confusables
/// table (or pass it through unchanged), concatenating the results. Single,
/// non-recursive pass — the table targets are already in canonical form.
pub fn skeleton(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut buf = [0u8; 4];
    for c in s.chars() {
        match skeleton_of(c) {
            Some(sk) => out.push_str(sk),
            None => out.push_str(c.encode_utf8(&mut buf)),
        }
    }
    out
}

/// Unweighted Levenshtein (no transposition) on two char slices.
pub fn levenshtein(a: &[char], b: &[char]) -> u64 {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m as u64;
    }
    if m == 0 {
        return n as u64;
    }
    let mut prev: Vec<u64> = (0..=m as u64).collect();
    let mut cur = vec![0u64; m + 1];
    for i in 1..=n {
        cur[0] = i as u64;
        for j in 1..=m {
            let sub = prev[j - 1] + u64::from(a[i - 1] != b[j - 1]);
            let del = prev[j] + 1;
            let ins = cur[j - 1] + 1;
            cur[j] = sub.min(del).min(ins);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

/// Unweighted Damerau-Levenshtein with adjacent transpositions (OSA variant).
pub fn damerau(a: &[char], b: &[char]) -> u64 {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m as u64;
    }
    if m == 0 {
        return n as u64;
    }
    let cols = m + 1;
    let mut d = vec![0u64; (n + 1) * cols];
    let idx = |i: usize, j: usize| i * cols + j;
    for i in 0..=n {
        d[idx(i, 0)] = i as u64;
    }
    for j in 0..=m {
        d[idx(0, j)] = j as u64;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = d[idx(i - 1, j - 1)] + u64::from(a[i - 1] != b[j - 1]);
            let del = d[idx(i - 1, j)] + 1;
            let ins = d[idx(i, j - 1)] + 1;
            let mut best = sub.min(del).min(ins);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                let trans = d[idx(i - 2, j - 2)] + 1;
                if trans < best {
                    best = trans;
                }
            }
            d[idx(i, j)] = best;
        }
    }
    d[idx(n, m)]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cv(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn classic_levenshtein() {
        assert_eq!(levenshtein(&cv("kitten"), &cv("sitting")), 3);
        assert_eq!(levenshtein(&cv("flaw"), &cv("lawn")), 2);
        assert_eq!(levenshtein(&cv(""), &cv("abc")), 3);
        assert_eq!(levenshtein(&cv("abc"), &cv("abc")), 0);
    }

    #[test]
    fn transposition_is_one_edit() {
        assert_eq!(levenshtein(&cv("googel"), &cv("google")), 2);
        assert_eq!(damerau(&cv("googel"), &cv("google")), 1);
        assert_eq!(damerau(&cv("ca"), &cv("ac")), 1);
    }

    #[test]
    fn digit_letter_confusable() {
        // '1' skeletons to 'l'; '0' skeletons to UPPERCASE 'O' (case-sensitive,
        // per UTS#39), so 0~O but not 0~o.
        assert!(confusable('1', 'l'));
        assert!(confusable('0', 'O'));
        assert!(!confusable('0', 'o'));
        assert!(confusable('I', 'l')); // capital I ~ lowercase L
        assert!(!confusable('x', 'y'));
    }

    #[test]
    fn skeleton_maps_multichar() {
        // UTS#39: the skeleton of 'm' is "rn", so "microsoft" and "rnicrosoft"
        // share a skeleton.
        assert_eq!(skeleton("microsoft"), skeleton("rnicrosoft"));
        assert_eq!(skeleton("microsoft"), "rnicrosoft");
    }

    #[test]
    fn skeleton_is_idempotent() {
        for s in ["microsoft", "paypal", "vvallet", "g\u{43E}\u{43E}gle", "abc123"] {
            assert_eq!(skeleton(&skeleton(s)), skeleton(s), "not idempotent for {s}");
        }
    }

    #[test]
    fn skeleton_collapses_homoglyphs() {
        assert_eq!(skeleton("p\u{0430}ypal"), skeleton("paypal"));
        assert_eq!(skeleton("xyz"), "xyz");
        assert_eq!(skeleton(""), "");
    }

    #[test]
    fn vv_w_and_cl_d_are_not_uts39_confusables() {
        // UTS#39 does NOT define vv->w / cl->d (their RHS are not source code
        // points). Pins the documented gap.
        assert_ne!(skeleton("vv"), skeleton("w"));
        assert_ne!(skeleton("cl"), skeleton("d"));
        assert_eq!(skeleton("m"), "rn"); // m->rn IS defined, for contrast.
    }
}
```

- [ ] **Step 2: Wire the module into `main.rs`; relocate skeleton fns; rename weighted distance fns**

In `src/main.rs`:
1. Add `mod distance;` directly after `mod confusables_data;`.
2. Delete the `skeleton_of`, `confusable`, `skeleton` function definitions (now in `distance.rs`).
3. Delete the `use confusables_data::CONFUSABLES;` line (only the moved skeleton fns used it).
4. Rename the remaining weighted `fn levenshtein` → `fn w_levenshtein` and `fn damerau` → `fn w_damerau` (keep their `(.., homoglyph: bool, w: f64) -> f64` signatures and bodies, which call `confusable` → now `distance::confusable`). `sub_cost` stays but calls `distance::confusable`.
5. In `score_pair`, replace `levenshtein(...)`→`w_levenshtein(...)`, `damerau(...)`→`w_damerau(...)`, and `skeleton(a)`/`skeleton(b)`→`distance::skeleton(a)`/`distance::skeleton(b)`.
6. In the `main.rs` `tests` module, re-point names: tests calling old `skeleton`/`confusable` → `distance::skeleton`/`distance::confusable`; tests calling old `levenshtein`/`damerau` (the `(.., bool, f64)` signature) → `w_levenshtein`/`w_damerau`. Do NOT silence with `#[allow(dead_code)]` — re-point every call. (These `main.rs` tests are deleted wholesale in Task 7; for now just make them compile and pass.)

- [ ] **Step 3: Run the full suite and clippy**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all tests pass (the new `distance::tests` plus the re-pointed `main.rs` tests), no clippy warnings, formatting clean.

- [ ] **Step 4: Commit**

```bash
git add src/distance.rs src/main.rs
git commit -m "$(cat <<'EOF'
refactor: extract skeleton/confusable into distance module

Relocate skeleton_of, confusable, and skeleton into a new src/distance.rs
with unweighted integer levenshtein/damerau, ahead of the multi-axis
refactor. main.rs keeps its weighted shim (w_levenshtein/w_damerau) until
the panel lands. Pure relocation; behavior unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add the alignment traceback (`AlignOp` + `align`) to `distance.rs`

`uts39_confusable_count` needs to count substitution *positions* in the a→b Damerau alignment. Add a traceback that reconstructs the edit operations from the OSA cost matrix, with deterministic tie-breaking.

**Files:**
- Modify: `src/distance.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/distance.rs`:

```rust
    #[test]
    fn align_counts_substitutions_equal_length() {
        // "abc" vs "axc": exactly one substitution at position (1,1).
        let a = cv("abc");
        let b = cv("axc");
        let ops = align(&a, &b);
        let subs: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, AlignOp::Sub(_, _)))
            .collect();
        assert_eq!(subs.len(), 1);
        assert!(matches!(subs[0], AlignOp::Sub(1, 1)));
    }

    #[test]
    fn align_handles_unequal_length() {
        // "ab" vs "abc": one insertion, zero substitutions.
        let ops = align(&cv("ab"), &cv("abc"));
        assert_eq!(ops.iter().filter(|o| matches!(o, AlignOp::Sub(_, _))).count(), 0);
        assert_eq!(ops.iter().filter(|o| matches!(o, AlignOp::Ins)).count(), 1);
    }

    #[test]
    fn align_marks_transposition() {
        // "ca" vs "ac": one transposition, no substitutions.
        let ops = align(&cv("ca"), &cv("ac"));
        assert_eq!(ops.iter().filter(|o| matches!(o, AlignOp::Sub(_, _))).count(), 0);
        assert_eq!(ops.iter().filter(|o| matches!(o, AlignOp::Transpose)).count(), 1);
    }

    #[test]
    fn align_is_deterministic_on_ties() {
        // Equal-length unrelated strings: every position substitutes.
        let ops1 = align(&cv("abc"), &cv("xyz"));
        let ops2 = align(&cv("abc"), &cv("xyz"));
        assert_eq!(ops1, ops2);
        assert_eq!(ops1.iter().filter(|o| matches!(o, AlignOp::Sub(_, _))).count(), 3);
    }

    #[test]
    fn align_substitution_indices_address_confusable() {
        // "paypal" vs "pаypal" (Cyrillic а at index 1): one Sub(1,1) whose chars
        // are confusable.
        let a = cv("paypal");
        let b = cv("p\u{0430}ypal");
        let ops = align(&a, &b);
        let confusable_subs = ops
            .iter()
            .filter_map(|op| match op {
                AlignOp::Sub(i, j) => Some((*i, *j)),
                _ => None,
            })
            .filter(|&(i, j)| confusable(a[i], b[j]))
            .count();
        assert_eq!(confusable_subs, 1);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test align 2>&1 | head -20`
Expected: FAIL — `AlignOp` and `align` are not defined (compile error).

- [ ] **Step 3: Implement `AlignOp` and `align`**

Add to `src/distance.rs` (above the `tests` module):

```rust
/// One operation in an a→b edit alignment, recovered by traceback.
/// `Sub(i, j)` substitutes `a[i]` with `b[j]` (indices into the original
/// char slices). `Match` is a zero-cost diagonal step. `Ins`/`Del` are the
/// single-char insert/delete; `Transpose` is an OSA adjacent swap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlignOp {
    Match,
    Sub(usize, usize),
    Ins,
    Del,
    Transpose,
}

/// Compute the OSA Damerau cost matrix for `a`→`b` and trace back one optimal
/// alignment, returned in forward order. Tie-break is deterministic: at each
/// cell we prefer the diagonal (match/substitution), then deletion, then
/// insertion, then transposition — chosen so the substitution COUNT is stable.
/// Used by the `uts39_confusable_count` axis to count confusable substitutions.
pub fn align(a: &[char], b: &[char]) -> Vec<AlignOp> {
    let (n, m) = (a.len(), b.len());
    let cols = m + 1;
    let mut d = vec![0u64; (n + 1) * cols];
    let idx = |i: usize, j: usize| i * cols + j;
    for i in 0..=n {
        d[idx(i, 0)] = i as u64;
    }
    for j in 0..=m {
        d[idx(0, j)] = j as u64;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = d[idx(i - 1, j - 1)] + u64::from(a[i - 1] != b[j - 1]);
            let del = d[idx(i - 1, j)] + 1;
            let ins = d[idx(i, j - 1)] + 1;
            let mut best = sub.min(del).min(ins);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                let trans = d[idx(i - 2, j - 2)] + 1;
                if trans < best {
                    best = trans;
                }
            }
            d[idx(i, j)] = best;
        }
    }

    // Traceback from (n, m) to (0, 0).
    let mut ops: Vec<AlignOp> = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        let cur = d[idx(i, j)];
        // Diagonal first: match or substitution.
        if i > 0 && j > 0 {
            let step = u64::from(a[i - 1] != b[j - 1]);
            if d[idx(i - 1, j - 1)] + step == cur {
                ops.push(if step == 0 {
                    AlignOp::Match
                } else {
                    AlignOp::Sub(i - 1, j - 1)
                });
                i -= 1;
                j -= 1;
                continue;
            }
        }
        // Deletion (consume a[i-1]).
        if i > 0 && d[idx(i - 1, j)] + 1 == cur {
            ops.push(AlignOp::Del);
            i -= 1;
            continue;
        }
        // Insertion (consume b[j-1]).
        if j > 0 && d[idx(i, j - 1)] + 1 == cur {
            ops.push(AlignOp::Ins);
            j -= 1;
            continue;
        }
        // Transposition (adjacent swap).
        if i > 1
            && j > 1
            && a[i - 1] == b[j - 2]
            && a[i - 2] == b[j - 1]
            && d[idx(i - 2, j - 2)] + 1 == cur
        {
            ops.push(AlignOp::Transpose);
            i -= 2;
            j -= 2;
            continue;
        }
        // Unreachable for a consistent matrix; guard against infinite loop.
        break;
    }
    ops.reverse();
    ops
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test align && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: the 5 new tests PASS; no warnings; format clean.

- [ ] **Step 5: Commit**

```bash
git add src/distance.rs
git commit -m "$(cat <<'EOF'
feat(distance): add OSA alignment traceback for confusable counting

Add AlignOp + align(): reconstruct one optimal a->b edit alignment from the
Damerau cost matrix with deterministic tie-breaking (diagonal, then del,
ins, transpose). The uts39_confusable_count axis will count Sub ops whose
two chars are UTS#39-confusable.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Build the axes core in `axes.rs` (`AxisValue`, `Direction`, `Axis`, `PairContext`, `Panel`)

Create the axis abstraction, the ordered result map, and the shared per-pair context. No axis impls yet — just the types and the context that precomputes shared data.

**Files:**
- Create: `src/axes.rs`
- Modify: `src/main.rs` (add `mod axes;`)

- [ ] **Step 1: Create `src/axes.rs` with the core types and tests**

```rust
//! The axis panel: independent similarity signals over a string pair.
//!
//! An `Axis` is one signal (`key`, `direction`, `compute`). `ALL_AXES` lists
//! every axis in canonical order — the single source of truth for JSON keys,
//! human-row order, and `--fields`/`--metric` validation. `PairContext` holds
//! the once-per-pair precomputation (char vecs, skeletons, alignment).

use crate::distance::{self, AlignOp};

/// The value an axis produces. The output formatter renders each variant.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AxisValue {
    Int(u64),
    Float(f64),
    Bool(bool),
}

impl AxisValue {
    /// JSON rendering: ints/bools unquoted, floats at fixed precision.
    pub fn to_json(self) -> String {
        match self {
            AxisValue::Int(n) => n.to_string(),
            AxisValue::Float(f) => format!("{f:.4}"),
            AxisValue::Bool(b) => b.to_string(),
        }
    }

    /// Human rendering — identical to JSON for the v0.3.0 panel.
    pub fn to_human(self) -> String {
        self.to_json()
    }

    /// Numeric coercion for `--metric`/`--sort`/`-t`. Bools return None and are
    /// rejected as metric targets.
    pub fn as_f64(self) -> Option<f64> {
        match self {
            AxisValue::Int(n) => Some(n as f64),
            AxisValue::Float(f) => Some(f),
            AxisValue::Bool(_) => None,
        }
    }
}

/// How to read an axis value when reasoning generically (verdict/ranking).
/// Not emitted in output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    HigherMoreSimilar,
    HigherMoreDifferent,
}

/// Which computation phase an axis belongs to. Base axes are pure functions of
/// the `PairContext`; derived axes read already-computed base values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Base,
    Derived,
}

/// Ordered key→value results of a panel run, in canonical registry order.
#[derive(Default, Debug, Clone)]
pub struct Panel {
    pub entries: Vec<(&'static str, AxisValue)>,
}

impl Panel {
    /// Look up a computed axis value by key (used by derived axes and output).
    pub fn get(&self, key: &str) -> Option<AxisValue> {
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

/// Shared per-pair precomputation, built once and passed to every base axis.
pub struct PairContext<'a> {
    pub a: &'a str,
    pub b: &'a str,
    pub ca: Vec<char>,
    pub cb: Vec<char>,
    pub ska: String,
    pub skb: String,
    pub sva: Vec<char>,
    pub svb: Vec<char>,
    pub align: Vec<AlignOp>,
}

impl<'a> PairContext<'a> {
    pub fn new(a: &'a str, b: &'a str) -> Self {
        let ca: Vec<char> = a.chars().collect();
        let cb: Vec<char> = b.chars().collect();
        let ska = distance::skeleton(a);
        let skb = distance::skeleton(b);
        let sva: Vec<char> = ska.chars().collect();
        let svb: Vec<char> = skb.chars().collect();
        let align = distance::align(&ca, &cb);
        PairContext { a, b, ca, cb, ska, skb, sva, svb, align }
    }
}

/// One independent similarity signal. Base axes read `ctx`; derived axes read
/// the base results via `base` (and ignore `ctx`).
pub trait Axis: Sync {
    /// Stable JSON key / human label.
    fn key(&self) -> &'static str;
    /// How to interpret the value (verdict/ranking); not emitted.
    fn direction(&self) -> Direction;
    /// Computation phase.
    fn phase(&self) -> Phase;
    /// Compute the value. `base` is the already-computed base map; it is empty
    /// during the base phase and fully populated for derived axes.
    fn compute(&self, ctx: &PairContext, base: &Panel) -> AxisValue;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_context_precomputes_skeletons_and_alignment() {
        let ctx = PairContext::new("paypal", "p\u{0430}ypal");
        assert_eq!(ctx.ca.len(), 6);
        assert_eq!(ctx.cb.len(), 6);
        // Cyrillic а collapses to Latin a in the skeleton.
        assert_eq!(ctx.ska, ctx.skb);
        // Alignment has exactly one substitution.
        let subs = ctx
            .align
            .iter()
            .filter(|o| matches!(o, AlignOp::Sub(_, _)))
            .count();
        assert_eq!(subs, 1);
    }

    #[test]
    fn axis_value_json_rendering() {
        assert_eq!(AxisValue::Int(3).to_json(), "3");
        assert_eq!(AxisValue::Bool(true).to_json(), "true");
        assert_eq!(AxisValue::Float(0.5).to_json(), "0.5000");
    }

    #[test]
    fn axis_value_as_f64_rejects_bool() {
        assert_eq!(AxisValue::Int(2).as_f64(), Some(2.0));
        assert_eq!(AxisValue::Float(1.5).as_f64(), Some(1.5));
        assert_eq!(AxisValue::Bool(true).as_f64(), None);
    }

    #[test]
    fn panel_get_finds_entry() {
        let mut p = Panel::default();
        p.entries.push(("damerau", AxisValue::Int(2)));
        assert_eq!(p.get("damerau"), Some(AxisValue::Int(2)));
        assert_eq!(p.get("missing"), None);
    }
}
```

> Note: the `Axis`/`Direction`/`Phase` types have no concrete impls until Task 4. Do Tasks 3 and 4 back-to-back. If committing Task 3 alone trips `-D warnings` on the as-yet-unused trait, attach `#[allow(dead_code)] // consumed in Task 4` to the `Axis` trait only and remove it in Task 4. Tests for `AxisValue`/`Panel`/`PairContext` exercise those types, so only the trait is at risk.

- [ ] **Step 2: Wire the module and run**

In `src/main.rs` add `mod axes;` after `mod distance;`.

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: the 4 new `axes::tests` PASS alongside everything else; no warnings (see the dead-code note above if the trait trips clippy).

- [ ] **Step 3: Commit**

```bash
git add src/axes.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat(axes): add AxisValue, Direction, Axis trait, Panel, and PairContext

Introduce the axis abstraction, the ordered key->value Panel map, and the
once-per-pair shared context (char vecs, skeletons, alignment traceback).
Axis impls and the panel builder follow in the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Implement the 8 axes, `ALL_AXES`, and the two-phase `build_panel`

Add all eight axis structs, the canonical registry, and the builder that runs base then derived axes into an ordered map. This is the heart of the refactor.

**Files:**
- Modify: `src/axes.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/axes.rs`:

```rust
    fn run(a: &str, b: &str) -> Panel {
        let ctx = PairContext::new(a, b);
        build_panel(&ctx)
    }

    #[test]
    fn registry_canonical_order() {
        let keys: Vec<&str> = ALL_AXES.iter().map(|ax| ax.key()).collect();
        assert_eq!(
            keys,
            vec![
                "equal",
                "levenshtein",
                "damerau",
                "skeleton_levenshtein",
                "skeleton_damerau",
                "uts39_confusable_count",
                "uts39_skeleton_delta",
                "confusable_only",
            ]
        );
    }

    #[test]
    fn panel_emits_in_registry_order() {
        let p = run("abc", "abd");
        let keys: Vec<&str> = p.entries.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.first(), Some(&"equal"));
        assert_eq!(keys.last(), Some(&"confusable_only"));
        assert_eq!(keys.len(), 8);
    }

    #[test]
    fn equal_axis() {
        assert_eq!(run("abc", "abc").get("equal"), Some(AxisValue::Bool(true)));
        assert_eq!(run("abc", "abd").get("equal"), Some(AxisValue::Bool(false)));
    }

    #[test]
    fn levenshtein_and_damerau_axes() {
        let p = run("googel", "google");
        assert_eq!(p.get("levenshtein"), Some(AxisValue::Int(2)));
        assert_eq!(p.get("damerau"), Some(AxisValue::Int(1)));
    }

    #[test]
    fn skeleton_axes_catch_multichar() {
        // rnicrosoft vs microsoft: skeletons identical -> both skeleton metrics 0.
        let p = run("rnicrosoft", "microsoft");
        assert_eq!(p.get("skeleton_levenshtein"), Some(AxisValue::Int(0)));
        assert_eq!(p.get("skeleton_damerau"), Some(AxisValue::Int(0)));
    }

    #[test]
    fn uts39_confusable_count_counts_homoglyph_sub() {
        // One Cyrillic-а substitution => count 1.
        let p = run("paypal", "p\u{0430}ypal");
        assert_eq!(p.get("uts39_confusable_count"), Some(AxisValue::Int(1)));
        // Identical: no substitutions => 0.
        let q = run("google", "google");
        assert_eq!(q.get("uts39_confusable_count"), Some(AxisValue::Int(0)));
        // A real edit is NOT counted (t->r not confusable).
        let r = run("cat", "car");
        assert_eq!(r.get("uts39_confusable_count"), Some(AxisValue::Int(0)));
    }

    #[test]
    fn uts39_skeleton_delta_saturates() {
        // paypal vs pаypal: damerau 1, skeleton_damerau 0 => delta 1.
        let p = run("paypal", "p\u{0430}ypal");
        assert_eq!(p.get("uts39_skeleton_delta"), Some(AxisValue::Int(1)));
        // Identical: delta 0 (saturating, never negative).
        let q = run("abc", "abc");
        assert_eq!(q.get("uts39_skeleton_delta"), Some(AxisValue::Int(0)));
    }

    #[test]
    fn confusable_only_axis() {
        // GO0GLE vs GOOGLE: differ but identical skeletons.
        assert_eq!(run("GO0GLE", "GOOGLE").get("confusable_only"), Some(AxisValue::Bool(true)));
        // rnicrosoft/microsoft: multi-char confusable, unequal length, still true.
        assert_eq!(run("rnicrosoft", "microsoft").get("confusable_only"), Some(AxisValue::Bool(true)));
        // devflovv/devflow: vv/w gap -> NOT confusable_only (regression test).
        assert_eq!(run("devflovv", "devflow").get("confusable_only"), Some(AxisValue::Bool(false)));
        // Identical strings are not confusable_only.
        assert_eq!(run("abc", "abc").get("confusable_only"), Some(AxisValue::Bool(false)));
    }

    #[test]
    fn derived_axes_read_base_values() {
        let p = run("rnicrosoft", "microsoft");
        assert!(p.get("uts39_skeleton_delta").is_some());
        assert!(p.get("confusable_only").is_some());
    }

    #[test]
    fn axis_directions() {
        let by_key = |k: &str| ALL_AXES.iter().find(|ax| ax.key() == k).unwrap().direction();
        assert_eq!(by_key("levenshtein"), Direction::HigherMoreDifferent);
        assert_eq!(by_key("uts39_confusable_count"), Direction::HigherMoreSimilar);
        assert_eq!(by_key("uts39_skeleton_delta"), Direction::HigherMoreSimilar);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib 2>&1 | head -20`
Expected: FAIL — `ALL_AXES` and `build_panel` not defined (compile error).

- [ ] **Step 3: Implement the axes, registry, and builder**

Add to `src/axes.rs` (above the `tests` module). Each axis is a unit struct implementing `Axis`.

```rust
// ---- Base axes ----

struct Equal;
impl Axis for Equal {
    fn key(&self) -> &'static str {
        "equal"
    }
    fn direction(&self) -> Direction {
        // Boolean; direction is irrelevant. Sensible default.
        Direction::HigherMoreSimilar
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        AxisValue::Bool(ctx.a == ctx.b)
    }
}

struct Levenshtein;
impl Axis for Levenshtein {
    fn key(&self) -> &'static str {
        "levenshtein"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreDifferent
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        AxisValue::Int(distance::levenshtein(&ctx.ca, &ctx.cb))
    }
}

struct Damerau;
impl Axis for Damerau {
    fn key(&self) -> &'static str {
        "damerau"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreDifferent
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        AxisValue::Int(distance::damerau(&ctx.ca, &ctx.cb))
    }
}

struct SkeletonLevenshtein;
impl Axis for SkeletonLevenshtein {
    fn key(&self) -> &'static str {
        "skeleton_levenshtein"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreDifferent
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        AxisValue::Int(distance::levenshtein(&ctx.sva, &ctx.svb))
    }
}

struct SkeletonDamerau;
impl Axis for SkeletonDamerau {
    fn key(&self) -> &'static str {
        "skeleton_damerau"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreDifferent
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        AxisValue::Int(distance::damerau(&ctx.sva, &ctx.svb))
    }
}

struct Uts39ConfusableCount;
impl Axis for Uts39ConfusableCount {
    fn key(&self) -> &'static str {
        "uts39_confusable_count"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreSimilar
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        let count = ctx
            .align
            .iter()
            .filter_map(|op| match op {
                AlignOp::Sub(i, j) => Some((*i, *j)),
                _ => None,
            })
            .filter(|&(i, j)| distance::confusable(ctx.ca[i], ctx.cb[j]))
            .count();
        AxisValue::Int(count as u64)
    }
}

// ---- Derived axes ----

struct Uts39SkeletonDelta;
impl Axis for Uts39SkeletonDelta {
    fn key(&self) -> &'static str {
        "uts39_skeleton_delta"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreSimilar
    }
    fn phase(&self) -> Phase {
        Phase::Derived
    }
    fn compute(&self, _ctx: &PairContext, base: &Panel) -> AxisValue {
        let dam = match base.get("damerau") {
            Some(AxisValue::Int(n)) => n,
            _ => 0,
        };
        let skel = match base.get("skeleton_damerau") {
            Some(AxisValue::Int(n)) => n,
            _ => 0,
        };
        AxisValue::Int(dam.saturating_sub(skel))
    }
}

struct ConfusableOnly;
impl Axis for ConfusableOnly {
    fn key(&self) -> &'static str {
        "confusable_only"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreSimilar
    }
    fn phase(&self) -> Phase {
        Phase::Derived
    }
    fn compute(&self, _ctx: &PairContext, base: &Panel) -> AxisValue {
        let equal = matches!(base.get("equal"), Some(AxisValue::Bool(true)));
        let skel_lev_zero = matches!(base.get("skeleton_levenshtein"), Some(AxisValue::Int(0)));
        AxisValue::Bool(!equal && skel_lev_zero)
    }
}

/// Every axis in canonical order — the single source of truth for JSON keys,
/// human-row order, and `--fields`/`--metric` validation.
pub static ALL_AXES: &[&dyn Axis] = &[
    &Equal,
    &Levenshtein,
    &Damerau,
    &SkeletonLevenshtein,
    &SkeletonDamerau,
    &Uts39ConfusableCount,
    &Uts39SkeletonDelta,
    &ConfusableOnly,
];

/// Run the panel: phase 1 computes every base axis into the map (registry
/// order), phase 2 computes every derived axis (reading a snapshot of the base
/// map). Returns `Panel.entries` in canonical registry order.
pub fn build_panel(ctx: &PairContext) -> Panel {
    let mut panel = Panel::default();
    // Phase 1: base axes, in registry order.
    for ax in ALL_AXES.iter().filter(|ax| ax.phase() == Phase::Base) {
        let v = ax.compute(ctx, &panel);
        panel.entries.push((ax.key(), v));
    }
    // Phase 2: derived axes read a frozen snapshot of the base results.
    let base_snapshot = panel.clone();
    for ax in ALL_AXES.iter().filter(|ax| ax.phase() == Phase::Derived) {
        let v = ax.compute(ctx, &base_snapshot);
        panel.entries.push((ax.key(), v));
    }
    // Re-order entries into canonical ALL_AXES order so emit order matches the
    // registry regardless of phase grouping.
    panel.entries.sort_by_key(|(k, _)| {
        ALL_AXES.iter().position(|ax| ax.key() == *k).unwrap_or(usize::MAX)
    });
    panel
}
```

> Note: `ALL_AXES` is `&[&dyn Axis]`; the trait's `Sync` supertrait makes the static legal. If the compiler asks for an explicit `+ Sync`, write `&[&(dyn Axis + Sync)]`. If you added the temporary `#[allow(dead_code)]` to the `Axis` trait in Task 3, remove it now.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all `axes::tests` PASS; no warnings; format clean. (The `main.rs` old code still compiles unchanged.)

- [ ] **Step 5: Commit**

```bash
git add src/axes.rs
git commit -m "$(cat <<'EOF'
feat(axes): implement 8-axis panel and two-phase builder

Add equal, levenshtein, damerau, skeleton_levenshtein, skeleton_damerau,
uts39_confusable_count (base) and uts39_skeleton_delta, confusable_only
(derived). ALL_AXES defines canonical emit order; build_panel runs base
then derived phases and returns results in registry order.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Add `--fields` and `--metric` axis-key handling to `axes.rs`

The old `Field` enum handled `--fields` parsing and emission order; replace it with axis-key-based parsing. The old `Metric` enum drove `-t`/`--sort`; replace it with a numeric-axis-key lookup.

**Files:**
- Modify: `src/axes.rs`

- [ ] **Step 1: Write the failing tests**

Add to `axes::tests`:

```rust
    #[test]
    fn parse_fields_canonical_order_and_dedup() {
        // User order ignored; canonical order enforced; dups collapse.
        let f = parse_fields("confusable_only,damerau,damerau").unwrap();
        assert_eq!(f, vec!["damerau", "confusable_only"]);
    }

    #[test]
    fn parse_fields_all_keys() {
        let all = parse_fields(
            "equal,levenshtein,damerau,skeleton_levenshtein,skeleton_damerau,uts39_confusable_count,uts39_skeleton_delta,confusable_only",
        )
        .unwrap();
        assert_eq!(all.len(), 8);
    }

    #[test]
    fn parse_fields_rejects_unknown() {
        let e = parse_fields("damerau,bogus").unwrap_err();
        assert!(e.contains("bogus"), "must name the offender: {e}");
        assert!(e.contains("levenshtein"), "must list valid keys: {e}");
    }

    #[test]
    fn metric_accepts_numeric_keys() {
        assert!(validate_metric("skeleton_damerau").is_ok());
        assert!(validate_metric("levenshtein").is_ok());
        assert!(validate_metric("uts39_confusable_count").is_ok());
    }

    #[test]
    fn metric_rejects_bool_and_unknown_keys() {
        let e = validate_metric("equal").unwrap_err();
        assert!(e.contains("equal"));
        assert!(validate_metric("confusable_only").is_err());
        let u = validate_metric("nope").unwrap_err();
        assert!(u.contains("nope"));
    }

    #[test]
    fn metric_value_reads_panel() {
        let p = run("paypal", "p\u{0430}ypal");
        // skeleton_damerau is 0 for a pure homoglyph spoof.
        assert_eq!(metric_value(&p, "skeleton_damerau"), Some(0.0));
        // bool axis -> None (not a metric).
        assert_eq!(metric_value(&p, "equal"), None);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib parse_fields 2>&1 | head -20`
Expected: FAIL — `parse_fields`, `validate_metric`, `metric_value` not defined.

- [ ] **Step 3: Implement**

Add to `src/axes.rs` (above `tests`):

```rust
/// The full list of axis keys in canonical order.
pub fn all_keys() -> Vec<&'static str> {
    ALL_AXES.iter().map(|ax| ax.key()).collect()
}

/// The two boolean axes are not valid numeric metrics.
fn is_bool_axis(key: &str) -> bool {
    matches!(key, "equal" | "confusable_only")
}

/// Keys of numeric (Int/Float) axes — the valid `--metric` targets.
pub fn numeric_keys() -> Vec<&'static str> {
    ALL_AXES
        .iter()
        .map(|ax| ax.key())
        .filter(|k| !is_bool_axis(k))
        .collect()
}

/// Parse a comma-separated field list into canonical-ordered, de-duplicated
/// axis keys. Errors (naming the offender + valid keys) on any unknown field.
pub fn parse_fields(spec: &str) -> Result<Vec<&'static str>, String> {
    let keys = all_keys();
    let mut seen = vec![false; keys.len()];
    for raw in spec.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        match keys.iter().position(|k| *k == name) {
            Some(i) => seen[i] = true,
            None => {
                return Err(format!("unknown field: {name} (valid: {})", keys.join(", ")));
            }
        }
    }
    Ok(keys
        .into_iter()
        .enumerate()
        .filter(|(i, _)| seen[*i])
        .map(|(_, k)| k)
        .collect())
}

/// Validate a `--metric` key: must be a known numeric axis. Bool axes and
/// unknown keys are rejected with a message listing valid numeric keys.
pub fn validate_metric(key: &str) -> Result<&'static str, String> {
    let numeric = numeric_keys();
    match numeric.iter().find(|k| **k == key) {
        Some(k) => Ok(*k),
        None => Err(format!(
            "invalid --metric: {key} (valid numeric axes: {})",
            numeric.join(", ")
        )),
    }
}

/// The numeric value of `key` in a computed panel, for `-t`/`--sort`. None for
/// bool axes or missing keys.
pub fn metric_value(panel: &Panel, key: &str) -> Option<f64> {
    panel.get(key).and_then(|v| v.as_f64())
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/axes.rs
git commit -m "$(cat <<'EOF'
feat(axes): add --fields parsing and numeric --metric validation

parse_fields validates against axis keys (canonical order, dedup, errors
naming the offender + valid keys). validate_metric accepts any numeric
axis and rejects the two bool axes. metric_value reads a numeric axis out
of a computed panel for -t/--sort.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Re-source the verdict into `verdict.rs` reading the panel

Move `Verdict` + `verdict()` into a new module, re-sourced to read axis values from a `Panel` instead of the old `Scores` struct. Behavior must be identical to v0.2.0.

**Files:**
- Create: `src/verdict.rs`
- Modify: `src/main.rs` (add `mod verdict;`)

- [ ] **Step 1: Create `src/verdict.rs` with tests**

```rust
//! The single-pair human verdict: classify a scored pair as IDENTICAL,
//! LIKELY SPOOF, or LIKELY BENIGN. Reads the axis panel; behavior is
//! equivalent to v0.2.0 (uts39_skeleton_delta replaces the old
//! damerau-skeleton numerator; confusable_only is unchanged in meaning).

use crate::axes::{AxisValue, Panel};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Identical,
    LikelySpoof,
    LikelyBenign,
}

impl Verdict {
    pub fn tag(self) -> &'static str {
        match self {
            Verdict::Identical => "IDENTICAL",
            Verdict::LikelySpoof => "LIKELY SPOOF",
            Verdict::LikelyBenign => "LIKELY BENIGN",
        }
    }
}

/// Read an Int axis from the panel (0 if absent — never happens for base axes).
fn int_axis(panel: &Panel, key: &str) -> u64 {
    match panel.get(key) {
        Some(AxisValue::Int(n)) => n,
        _ => 0,
    }
}

/// Read a Bool axis from the panel (false if absent).
fn bool_axis(panel: &Panel, key: &str) -> bool {
    matches!(panel.get(key), Some(AxisValue::Bool(true)))
}

/// Classify a scored pair into a human verdict + explanation sentence.
/// `len_a`/`len_b` are character counts; `len_tolerance` bounds the
/// length-difference ratio allowed for a spoof verdict (default 0.25).
pub fn verdict(panel: &Panel, len_a: usize, len_b: usize, len_tolerance: f64) -> (Verdict, String) {
    if bool_axis(panel, "equal") {
        return (Verdict::Identical, "The strings are identical.".to_string());
    }

    let dam = int_axis(panel, "damerau");
    let skel = int_axis(panel, "skeleton_damerau");
    let delta = int_axis(panel, "uts39_skeleton_delta"); // damerau - skeleton_damerau, saturating
    let confusable_only = bool_axis(panel, "confusable_only");

    let maxlen = len_a.max(len_b).max(1) as f64;
    let len_diff_ratio = (len_a as f64 - len_b as f64).abs() / maxlen;
    let homoglyph_share = if dam == 0 {
        0.0
    } else {
        delta as f64 / dam as f64
    };

    let is_spoof = confusable_only
        || (homoglyph_share > 0.5 && len_a.max(len_b) >= 3 && len_diff_ratio <= len_tolerance);

    let edits = if dam == 1 {
        "1 edit".to_string()
    } else {
        format!("{dam} edits")
    };

    if is_spoof {
        let detail = if confusable_only {
            "every differing character is a homoglyph (the strings are visually identical)"
        } else {
            "most of the difference comes from homoglyphs (visually confusable characters)"
        };
        let msg = format!(
            "The strings differ by {edits}, but {detail}. High likelihood of an attempt to confuse."
        );
        return (Verdict::LikelySpoof, msg);
    }

    let msg = if dam <= 2 {
        let homo_note = if skel < dam {
            " (with only minor homoglyph involvement)"
        } else {
            ""
        };
        format!(
            "The strings differ by {edits} with no significant homoglyph involvement{homo_note} — likely a typo."
        )
    } else {
        format!(
            "The strings differ by {edits} with no significant homoglyph involvement — they appear unrelated."
        )
    };
    (Verdict::LikelyBenign, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axes::{build_panel, PairContext};

    fn panel(a: &str, b: &str) -> Panel {
        build_panel(&PairContext::new(a, b))
    }

    #[test]
    fn verdict_identical() {
        let (cat, msg) = verdict(&panel("abc", "abc"), 3, 3, 0.25);
        assert_eq!(cat, Verdict::Identical);
        assert!(msg.to_lowercase().contains("identical"));
    }

    #[test]
    fn verdict_single_char_spoof() {
        // GO0GLE vs GOOGLE: confusable_only true.
        let (cat, _) = verdict(&panel("GOOGLE", "GO0GLE"), 6, 6, 0.25);
        assert_eq!(cat, Verdict::LikelySpoof);
    }

    #[test]
    fn verdict_multichar_spoof_unequal_length() {
        let (cat, _) = verdict(&panel("rnicrosoft", "microsoft"), 10, 9, 0.25);
        assert_eq!(cat, Verdict::LikelySpoof);
    }

    #[test]
    fn verdict_benign_typo() {
        let (cat, msg) = verdict(&panel("google", "gogle"), 6, 5, 0.25);
        assert_eq!(cat, Verdict::LikelyBenign);
        assert!(msg.to_lowercase().contains("typo"));
    }

    #[test]
    fn verdict_benign_unrelated() {
        let (cat, msg) = verdict(&panel("apple", "xylophone"), 5, 9, 0.25);
        assert_eq!(cat, Verdict::LikelyBenign);
        assert!(msg.to_lowercase().contains("unrelated"));
    }

    #[test]
    fn verdict_length_tolerance_boundary() {
        // a0 vs aOxyz: '0'~'O' but xyz are real edits, confusable_only false,
        // lengths 2 vs 5 (ratio 0.6 > 0.25) -> benign.
        let (cat, _) = verdict(&panel("a0", "aOxyz"), 2, 5, 0.25);
        assert_eq!(cat, Verdict::LikelyBenign);
    }
}
```

- [ ] **Step 2: Wire and run**

In `src/main.rs` add `mod verdict;` after `mod axes;`.

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: the 6 `verdict::tests` PASS. The old `verdict`/`Verdict` in `main.rs` still exist and are still used by `main.rs`; both compile. There will be two `Verdict` types until Task 7 deletes the old one. If clippy flags the new module's items as unused before Task 7, do Tasks 6 and 7 back-to-back; only if a lone commit won't pass `-D warnings`, attach `#[allow(dead_code)] // consumed in Task 7` to the new `Verdict`/`verdict` and remove it in Task 7.

- [ ] **Step 3: Commit**

```bash
git add src/verdict.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat(verdict): re-source verdict to read the axis panel

Move Verdict + verdict() into src/verdict.rs, reading equal / damerau /
skeleton_damerau / uts39_skeleton_delta / confusable_only from the panel.
Behavior-equivalent to v0.2.0. main.rs is rewired in the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Rewire `main.rs` onto the panel and delete the old scoring code

The flip. Rewrite `main.rs`'s scoring/emit/dispatch to use `build_panel` + the axis output, generalize `--metric` to an axis key, drop `--hogl-weight`, and **delete** the old `Scores`, `score_pair`, `Field`, `Metric`, `metric_value`, `sort_and_truncate`, `score_candidate`, `process_list`, `result_json`, `selected_fields`, `emit`, `w_levenshtein`, `w_damerau`, `sub_cost`, the old `verdict`/`Verdict`, and the old `main.rs` `tests` that exercised them.

**Files:**
- Modify: `src/main.rs` (substantial rewrite — replace the whole file)

- [ ] **Step 1: Replace `src/main.rs` in full**

Replace the entire contents of `src/main.rs` with:

```rust
//! sqdist — string distance for typosquatting / homoglyph detection.
//!
//! Computes a panel of independent similarity axes (Levenshtein, Damerau,
//! their UTS#39-skeleton variants, and confusable-involvement signals) between
//! two strings. See src/axes.rs for the axis registry and src/verdict.rs for
//! the single-pair human verdict.

mod axes;
mod confusables_data;
mod distance;
mod verdict;

use axes::{
    build_panel, metric_value, parse_fields, validate_metric, AxisValue, PairContext, Panel,
    ALL_AXES,
};
use std::env;
use std::process::ExitCode;
use verdict::verdict;

/// A scored row carried through batch/list modes: the two strings + the panel.
type Row = (String, String, Panel);

/// Compute the panel for a pair.
fn score_pair(a: &str, b: &str) -> Panel {
    build_panel(&PairContext::new(a, b))
}

/// The selected axis keys to emit, in canonical order: all when None.
fn selected_keys(fields: Option<&[&'static str]>) -> Vec<&'static str> {
    match fields {
        None => ALL_AXES.iter().map(|ax| ax.key()).collect(),
        Some(list) => list.to_vec(),
    }
}

/// One JSONL record for a scored pair. `keys` names the two strings.
fn result_json(
    a: &str,
    b: &str,
    panel: &Panel,
    keys: (&str, &str),
    fields: Option<&[&'static str]>,
) -> String {
    let mut out = format!("{{\"{}\":{:?},\"{}\":{:?}", keys.0, a, keys.1, b);
    for k in selected_keys(fields) {
        if let Some(v) = panel.get(k) {
            out.push_str(&format!(",\"{}\":{}", k, v.to_json()));
        }
    }
    out.push('}');
    out
}

/// Human single-pair output: one padded row per selected axis, then a blank
/// line, then the verdict.
fn emit_human(a: &str, b: &str, panel: &Panel, fields: Option<&[&'static str]>, len_tolerance: f64) {
    for k in selected_keys(fields) {
        if let Some(v) = panel.get(k) {
            // Longest key is uts39_confusable_count (22); pad to 24.
            println!("{k:<24} {}", v.to_human());
        }
    }
    let la = a.chars().count();
    let lb = b.chars().count();
    let (cat, msg) = verdict(panel, la, lb, len_tolerance);
    println!("\n[{}] {}", cat.tag(), msg);
}

/// The metric distance of a row, or +inf if the metric key is missing/non-numeric
/// (cannot happen — the key is validated at parse time).
fn row_metric(panel: &Panel, metric: &str) -> f64 {
    metric_value(panel, metric).unwrap_or(f64::INFINITY)
}

/// Sort rows ascending by the active metric (most suspicious first) and
/// optionally keep only the first `top`. Stable: ties preserve input order.
fn sort_and_truncate(mut rows: Vec<Row>, metric: &str, top: Option<usize>) -> Vec<Row> {
    rows.sort_by(|x, y| {
        row_metric(&x.2, metric)
            .partial_cmp(&row_metric(&y.2, metric))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(n) = top {
        rows.truncate(n);
    }
    rows
}

/// Score `string` against one raw candidate line. Returns the scored row, or
/// None if blank or over threshold on the active metric.
fn score_candidate(string: &str, raw: &str, metric: &str, threshold: Option<f64>) -> Option<Row> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    let panel = score_pair(string, line);
    if let Some(t) = threshold {
        if row_metric(&panel, metric) > t {
            return None;
        }
    }
    Some((string.to_string(), line.to_string(), panel))
}

/// Score `string` against each line, collecting kept rows, sorting/truncating
/// when ranking is requested.
fn process_list<I: Iterator<Item = String>>(
    string: &str,
    lines: I,
    metric: &str,
    threshold: Option<f64>,
    sort: bool,
    top: Option<usize>,
) -> Vec<Row> {
    let mut rows: Vec<Row> = lines
        .filter_map(|raw| score_candidate(string, &raw, metric, threshold))
        .collect();
    if sort || top.is_some() {
        rows = sort_and_truncate(rows, metric, top);
    }
    rows
}

/// Batch-mode success: with a threshold, requires at least one match; without,
/// always succeeds.
fn batch_matched_ok(threshold: Option<f64>, matched: bool) -> bool {
    threshold.is_none() || matched
}

struct Opts {
    json: bool,
    threshold: Option<f64>,
    stdin: bool,
    string: Option<String>,
    list: Option<String>,
    sort: bool,
    top: Option<usize>,
    metric: &'static str,
    positionals: Vec<String>,
    fields: Option<Vec<&'static str>>,
    len_tolerance: f64,
}

fn print_usage() {
    eprintln!(
        "sqdist - typosquat / homoglyph string distance\n\n\
         USAGE:\n\
         \x20   sqdist [OPTIONS] <STRING_A> <STRING_B>      # single pair\n\
         \x20   sqdist [OPTIONS] --stdin                    # batch: pre-paired lines\n\
         \x20   sqdist [OPTIONS] --string <S> --list <FILE> # score <S> vs each line\n\n\
         OPTIONS:\n\
         \x20   -t, --threshold <F>     Alert when the --metric distance <= F. Single-pair:\n\
         \x20                           sets exit code. Batch: filters output; exit 1 if none match.\n\
         \x20   -m, --metric <AXIS>     Numeric axis for -t and --sort (default skeleton_damerau)\n\
         \x20       --fields <LIST>     Comma-separated axes to show (default: all). See AXES.\n\
         \x20       --len-tolerance <F> Max length-difference ratio for a spoof verdict (default 0.25)\n\
         \x20   -s, --stdin             Batch: read TAB/comma pairs from stdin, emit JSONL\n\
         \x20       --string <S>        (with --list) the single string to compare\n\
         \x20       --list <FILE>       (with --string) score <S> against each non-blank line\n\
         \x20       --sort              List mode: emit most-suspicious-first (buffers)\n\
         \x20       --top <N>           List mode: keep only the N closest (implies --sort)\n\
         \x20   -j, --json              Emit JSON (single-pair mode)\n\
         \x20   -v, --version           Print version and commit, then exit\n\
         \x20   -h, --help              This help\n\n\
         AXES:\n\
         \x20   equal                   the strings are byte-identical (bool)\n\
         \x20   levenshtein             min single-char insert/delete/substitute edits\n\
         \x20   damerau                 like levenshtein, but an adjacent swap counts as one edit\n\
         \x20   skeleton_levenshtein    levenshtein after reducing both to UTS#39 skeletons\n\
         \x20   skeleton_damerau        damerau after reducing both to UTS#39 skeletons\n\
         \x20                           (~0 when visually identical, incl. multi-char confusables)\n\
         \x20   uts39_confusable_count  # of aligned substitutions that are UTS#39-confusable (experimental)\n\
         \x20   uts39_skeleton_delta    damerau - skeleton_damerau; edits that vanish under\n\
         \x20                           skeletonization (experimental, may change)\n\
         \x20   confusable_only         true when the strings differ but share an identical skeleton\n\n\
         OUTPUT KEYS: single-pair/stdin use a,b; list mode uses input,match. Batch is JSONL.\n"
    );
}

fn parse_from(argv: Vec<String>) -> Result<Opts, String> {
    let mut args = argv.into_iter();
    let mut opts = Opts {
        json: false,
        threshold: None,
        stdin: false,
        string: None,
        list: None,
        sort: false,
        top: None,
        metric: "skeleton_damerau",
        positionals: Vec::new(),
        fields: None,
        len_tolerance: 0.25,
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-v" | "--version" => {
                println!(
                    "sqdist {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    env!("SQDIST_GIT_SHA")
                );
                std::process::exit(0);
            }
            "-j" | "--json" => opts.json = true,
            "-s" | "--stdin" => opts.stdin = true,
            "--sort" => opts.sort = true,
            "--string" => opts.string = Some(args.next().ok_or("--string needs a value")?),
            "--list" => opts.list = Some(args.next().ok_or("--list needs a value")?),
            "--top" => {
                let v = args.next().ok_or("--top needs a value")?;
                let n: usize = v.parse().map_err(|_| "invalid --top")?;
                if n == 0 {
                    return Err("--top must be a positive integer".into());
                }
                opts.top = Some(n);
                opts.sort = true;
            }
            "-m" | "--metric" => {
                let v = args.next().ok_or("--metric needs a value")?;
                opts.metric = validate_metric(&v)?;
            }
            "--fields" => {
                let v = args.next().ok_or("--fields needs a value")?;
                opts.fields = Some(parse_fields(&v)?);
            }
            "--len-tolerance" => {
                let v = args.next().ok_or("--len-tolerance needs a value")?;
                let t: f64 = v.parse().map_err(|_| "invalid --len-tolerance")?;
                if !(0.0..=1.0).contains(&t) {
                    return Err("--len-tolerance must be between 0.0 and 1.0".into());
                }
                opts.len_tolerance = t;
            }
            "-t" | "--threshold" => {
                let v = args.next().ok_or("--threshold needs a value")?;
                opts.threshold = Some(v.parse().map_err(|_| "invalid --threshold")?);
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(format!("unknown option: {s}"));
            }
            _ => opts.positionals.push(a),
        }
    }

    let list_mode = opts.list.is_some() || opts.string.is_some();
    if list_mode {
        if opts.list.is_none() || opts.string.is_none() {
            return Err("--list and --string must be used together".into());
        }
        if opts.stdin {
            return Err("--list cannot be combined with --stdin".into());
        }
        if !opts.positionals.is_empty() {
            return Err("--list mode takes no positional arguments".into());
        }
        return Ok(opts);
    }
    if opts.stdin {
        if !opts.positionals.is_empty() {
            return Err("--stdin takes no positional arguments".into());
        }
        return Ok(opts);
    }
    if opts.positionals.len() != 2 {
        return Err(format!(
            "expected 2 string arguments, got {}",
            opts.positionals.len()
        ));
    }
    Ok(opts)
}

fn parse_args() -> Result<Opts, String> {
    parse_from(env::args().skip(1).collect())
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };

    use std::io::{self, BufRead, Write};

    // List mode: score --string against each line of --list, emit input/match JSONL.
    if let (Some(string), Some(path)) = (opts.string.as_ref(), opts.list.as_ref()) {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("error: cannot open --list file {path:?}: {e}");
                return ExitCode::from(2);
            }
        };
        let lines = io::BufReader::new(file).lines().map_while(Result::ok);
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        let mut matched = false;
        if opts.sort || opts.top.is_some() {
            let rows = process_list(
                string,
                lines,
                opts.metric,
                opts.threshold,
                opts.sort,
                opts.top,
            );
            matched = !rows.is_empty();
            for (a, b, panel) in &rows {
                let _ = writeln!(
                    out,
                    "{}",
                    result_json(a, b, panel, ("input", "match"), opts.fields.as_deref())
                );
            }
        } else {
            for raw in lines {
                if let Some((a, b, panel)) =
                    score_candidate(string, &raw, opts.metric, opts.threshold)
                {
                    matched = true;
                    let _ = writeln!(
                        out,
                        "{}",
                        result_json(&a, &b, &panel, ("input", "match"), opts.fields.as_deref())
                    );
                }
            }
        }
        return if batch_matched_ok(opts.threshold, matched) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Stdin batch mode: pre-paired lines, a/b JSONL.
    if opts.stdin {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        let mut matched = false;
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, ['\t', ',']);
            let (la, lb) = match (parts.next(), parts.next()) {
                (Some(x), Some(y)) => (x, y),
                _ => {
                    let _ = writeln!(out, "{{\"error\":\"malformed line\",\"line\":{line:?}}}");
                    continue;
                }
            };
            let panel = score_pair(la, lb);
            if let Some(t) = opts.threshold {
                if row_metric(&panel, opts.metric) > t {
                    continue;
                }
            }
            matched = true;
            let _ = writeln!(
                out,
                "{}",
                result_json(la, lb, &panel, ("a", "b"), opts.fields.as_deref())
            );
        }
        return if batch_matched_ok(opts.threshold, matched) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Single-pair mode.
    let a = &opts.positionals[0];
    let b = &opts.positionals[1];
    let panel = score_pair(a, b);
    if opts.json {
        println!(
            "{}",
            result_json(a, b, &panel, ("a", "b"), opts.fields.as_deref())
        );
    } else {
        emit_human(a, b, &panel, opts.fields.as_deref(), opts.len_tolerance);
    }
    if let Some(t) = opts.threshold {
        return if row_metric(&panel, opts.metric) <= t {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_json_uses_given_keys_and_all_axes() {
        let panel = score_pair("paypal", "p\u{0430}ypal");
        let line = result_json("paypal", "p\u{0430}ypal", &panel, ("a", "b"), None);
        assert!(line.starts_with("{\"a\":\"paypal\""));
        assert!(line.contains("\"damerau\":"));
        assert!(line.contains("\"skeleton_damerau\":"));
        assert!(line.contains("\"uts39_confusable_count\":"));
        assert!(line.contains("\"uts39_skeleton_delta\":"));
        assert!(line.contains("\"confusable_only\":true"));
        assert!(!line.contains("homoglyph_damerau"));
        assert!(!line.contains("normalized"));

        let line2 = result_json("paypal", "p\u{0430}ypal", &panel, ("input", "match"), None);
        assert!(line2.starts_with("{\"input\":\"paypal\",\"match\":"));
    }

    #[test]
    fn result_json_respects_field_filter() {
        let panel = score_pair("GOOGLE", "GO0GLE");
        let only = parse_fields("damerau,confusable_only").unwrap();
        let line = result_json("GOOGLE", "GO0GLE", &panel, ("a", "b"), Some(&only));
        assert!(line.starts_with("{\"a\":\"GOOGLE\",\"b\":\"GO0GLE\""));
        assert!(line.contains("\"damerau\":"));
        assert!(line.contains("\"confusable_only\":"));
        assert!(!line.contains("\"levenshtein\":"));
        assert!(!line.contains("\"skeleton_damerau\":"));
    }

    #[test]
    fn sort_and_truncate_orders_and_caps() {
        let rows = vec![
            ("abc".to_string(), "abXYZ".to_string(), score_pair("abc", "abXYZ")),
            ("abc".to_string(), "abc".to_string(), score_pair("abc", "abc")),
            ("abc".to_string(), "abd".to_string(), score_pair("abc", "abd")),
        ];
        let sorted = sort_and_truncate(rows, "skeleton_damerau", None);
        assert_eq!(sorted[0].1, "abc"); // identical -> 0 first
        assert!(
            row_metric(&sorted[0].2, "skeleton_damerau")
                <= row_metric(&sorted[1].2, "skeleton_damerau")
        );
        assert!(
            row_metric(&sorted[1].2, "skeleton_damerau")
                <= row_metric(&sorted[2].2, "skeleton_damerau")
        );

        let rows2 = vec![
            ("abc".to_string(), "abd".to_string(), score_pair("abc", "abd")),
            ("abc".to_string(), "abc".to_string(), score_pair("abc", "abc")),
        ];
        let top1 = sort_and_truncate(rows2, "skeleton_damerau", Some(1));
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].1, "abc");
    }

    #[test]
    fn sort_is_stable_on_ties() {
        let rows = vec![
            ("x".to_string(), "first".to_string(), score_pair("x", "first")),
            ("x".to_string(), "secnd".to_string(), score_pair("x", "secnd")),
        ];
        let a = row_metric(&rows[0].2, "skeleton_damerau");
        let b = row_metric(&rows[1].2, "skeleton_damerau");
        assert!((a - b).abs() < 1e-9, "precondition: scores must tie");
        let sorted = sort_and_truncate(rows, "skeleton_damerau", None);
        assert_eq!(sorted[0].1, "first");
        assert_eq!(sorted[1].1, "secnd");
    }

    #[test]
    fn process_list_filters_sorts_caps() {
        let lines = vec![
            "paypal".to_string(),
            "p\u{0430}ypal".to_string(),
            "completely-different".to_string(),
        ];
        let out = process_list(
            "paypal",
            lines.clone().into_iter(),
            "skeleton_damerau",
            None,
            true,
            Some(2),
        );
        assert_eq!(out.len(), 2);
        assert!(row_metric(&out[0].2, "skeleton_damerau").abs() < 1e-9);
        assert!(row_metric(&out[1].2, "skeleton_damerau").abs() < 1e-9);

        let out2 = process_list(
            "paypal",
            lines.into_iter(),
            "skeleton_damerau",
            Some(0.0),
            false,
            None,
        );
        assert_eq!(out2.len(), 2);

        let out3 = process_list(
            "paypal",
            vec!["".to_string(), "  ".to_string(), "paypal".to_string()].into_iter(),
            "skeleton_damerau",
            None,
            false,
            None,
        );
        assert_eq!(out3.len(), 1);
    }

    #[test]
    fn score_candidate_trims_skips_and_thresholds() {
        assert!(score_candidate("paypal", "", "skeleton_damerau", None).is_none());
        assert!(score_candidate("paypal", "   ", "skeleton_damerau", None).is_none());

        let r = score_candidate("paypal", "  p\u{0430}ypal  ", "skeleton_damerau", None)
            .expect("should score");
        assert_eq!(r.0, "paypal");
        assert_eq!(r.1, "p\u{0430}ypal");
        assert_eq!(r.2.get("confusable_only"), Some(AxisValue::Bool(true)));

        assert!(score_candidate("paypal", "zzzzzz", "skeleton_damerau", Some(0.0)).is_none());
        assert!(score_candidate("paypal", "p\u{0430}ypal", "skeleton_damerau", Some(0.0)).is_some());
    }

    #[test]
    fn batch_exit_codes() {
        assert!(batch_matched_ok(None, false));
        assert!(batch_matched_ok(None, true));
        assert!(batch_matched_ok(Some(0.5), true));
        assert!(!batch_matched_ok(Some(0.5), false));
    }

    #[test]
    fn git_sha_env_is_present() {
        assert!(!env!("SQDIST_GIT_SHA").is_empty());
    }

    #[test]
    fn parse_list_mode() {
        let o = parse_from(vec![
            "--string".into(), "paypal".into(),
            "--list".into(), "names.txt".into(),
            "--metric".into(), "skeleton_damerau".into(),
        ])
        .unwrap();
        assert_eq!(o.string.as_deref(), Some("paypal"));
        assert_eq!(o.list.as_deref(), Some("names.txt"));
        assert_eq!(o.metric, "skeleton_damerau");
        assert!(o.positionals.is_empty());
    }

    #[test]
    fn parse_top_implies_sort() {
        let o = parse_from(vec![
            "--string".into(), "x".into(), "--list".into(), "f".into(), "--top".into(), "5".into(),
        ])
        .unwrap();
        assert_eq!(o.top, Some(5));
        assert!(o.sort);
    }

    #[test]
    fn parse_rejects_mode_conflicts() {
        assert!(parse_from(vec!["a".into(), "b".into(), "--list".into(), "f".into()]).is_err());
        assert!(parse_from(vec!["--stdin".into(), "--list".into(), "f".into()]).is_err());
        assert!(parse_from(vec!["--list".into(), "f".into()]).is_err());
        assert!(parse_from(vec!["--string".into(), "x".into()]).is_err());
        assert!(parse_from(vec![
            "--string".into(), "x".into(), "--list".into(), "f".into(), "--metric".into(), "bogus".into(),
        ])
        .is_err());
    }

    #[test]
    fn parse_single_pair_still_works() {
        let o = parse_from(vec!["paypal".into(), "p\u{0430}ypal".into()]).unwrap();
        assert_eq!(o.positionals.len(), 2);
        assert!(o.list.is_none());
        assert!(!o.stdin);
        assert_eq!(o.metric, "skeleton_damerau");
    }

    #[test]
    fn metric_rejects_bool_axis_flag() {
        assert!(parse_from(vec!["--metric".into(), "equal".into(), "a".into(), "b".into()]).is_err());
        assert!(parse_from(vec!["--metric".into(), "confusable_only".into(), "a".into(), "b".into()]).is_err());
    }

    #[test]
    fn hogl_weight_flag_removed() {
        assert!(parse_from(vec!["-w".into(), "0.1".into(), "a".into(), "b".into()]).is_err());
        assert!(parse_from(vec!["--hogl-weight".into(), "0.1".into(), "a".into(), "b".into()]).is_err());
    }

    #[test]
    fn parse_fields_flag() {
        let o = parse_from(vec![
            "--fields".into(), "damerau,confusable_only".into(), "a".into(), "b".into(),
        ])
        .unwrap();
        assert_eq!(o.fields.as_deref(), Some(&["damerau", "confusable_only"][..]));
    }

    #[test]
    fn parse_fields_flag_rejects_unknown() {
        assert!(parse_from(vec![
            "--fields".into(), "damerau,nope".into(), "a".into(), "b".into(),
        ])
        .is_err());
    }

    #[test]
    fn parse_len_tolerance_default_and_override() {
        let d = parse_from(vec!["a".into(), "b".into()]).unwrap();
        assert!((d.len_tolerance - 0.25).abs() < 1e-9);
        let o = parse_from(vec![
            "--len-tolerance".into(), "0.4".into(), "a".into(), "b".into(),
        ])
        .unwrap();
        assert!((o.len_tolerance - 0.4).abs() < 1e-9);
    }

    #[test]
    fn parse_len_tolerance_rejects_out_of_range() {
        assert!(parse_from(vec!["--len-tolerance".into(), "5.0".into(), "a".into(), "b".into()]).is_err());
        assert!(parse_from(vec!["--len-tolerance".into(), "-0.5".into(), "a".into(), "b".into()]).is_err());
        assert!(parse_from(vec!["--len-tolerance".into(), "0.0".into(), "a".into(), "b".into()]).is_ok());
        assert!(parse_from(vec!["--len-tolerance".into(), "1.0".into(), "a".into(), "b".into()]).is_ok());
    }
}
```

- [ ] **Step 2: Run the full suite, clippy, fmt, and a manual smoke test**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: ALL tests across all four modules pass; no warnings; format clean.

Then smoke-test the binary end to end:

```bash
cargo build
./target/debug/sqdist paypal "p$(printf 'а')ypal"      # human: 8 panel rows + [LIKELY SPOOF]
./target/debug/sqdist -j paypal "p$(printf 'а')ypal"   # JSON: a/b keys, new axis keys, no normalized/homoglyph_damerau
./target/debug/sqdist --metric equal a b; echo "exit=$?"    # error: bool axis rejected, exit 2
./target/debug/sqdist -w 0.1 a b; echo "exit=$?"            # error: unknown option -w, exit 2
printf 'paypal\tpaypal\n' | ./target/debug/sqdist --stdin   # JSONL a/b
```
Expected: single-pair shows the 8 axis rows (labels padded to col 24) then a `[LIKELY SPOOF]` line; JSON contains `uts39_confusable_count`/`uts39_skeleton_delta`/`confusable_only` and NOT `normalized`/`homoglyph_damerau`; both error cases print usage and exit 2.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
refactor!: rewire main onto the axis panel; drop weighted hogl model

main.rs now scores via build_panel and emits axis values in registry
order. Generalize --metric to any numeric axis key (default
skeleton_damerau); drop --hogl-weight, the Field/Metric/Scores types, the
normalized/homoglyph_damerau fields, and the old verdict. Three modes,
streaming-vs-buffered list path, batch exit codes, and a/b vs input/match
keying all preserved.

BREAKING CHANGE: JSON keys changed — homoglyph_damerau, normalized, and
skeleton_normalized removed; equal, skeleton_levenshtein,
uts39_confusable_count, uts39_skeleton_delta added. --hogl-weight removed.
--metric now takes an axis key.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Bump version to 0.3.0

**Files:**
- Modify: `Cargo.toml:3`
- Modify: `Cargo.lock` (the `sqdist` package version)

- [ ] **Step 1: Bump Cargo.toml**

Change `version = "0.2.0"` to `version = "0.3.0"` in `Cargo.toml`.

- [ ] **Step 2: Update Cargo.lock**

Run: `cargo build`
Then confirm: `grep -A1 'name = "sqdist"' Cargo.lock`
Expected: `version = "0.3.0"`.

- [ ] **Step 3: Verify and commit**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all pass.

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore: bump version to 0.3.0

Breaking multi-axis architecture refactor.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Rewrite the docs (CLAUDE.md + README) for the axis panel

Update both docs to describe the axis panel, the new field list, the generalized `--metric`, the removed flags/fields, and the breaking JSON-contract change. The `--help` text is already updated (Task 7).

**Files:**
- Modify: `CLAUDE.md`
- Modify: `README.md`

- [ ] **Step 1: Read the current docs**

Read `CLAUDE.md` and `README.md` in full so edits match existing structure/voice. (Both are prose; no tests. Edit surgically.)

- [ ] **Step 2: Update `CLAUDE.md`**

Make these changes (preserve structure/headings; change content):

1. **"What this is"** — replace "three string-distance metrics … homoglyph-weighted Damerau" with: sqdist computes a **panel of independent similarity axes** (Levenshtein, Damerau, their UTS#39-skeleton variants, and confusable-involvement signals) for typosquatting/homoglyph detection. Drop the `0.1` weighted-substitution framing.
2. **Architecture / file list** — replace "Two source files" with the five-file layout: `main.rs` (CLI/IO/modes/output), `distance.rs` (edit distances, skeleton model, alignment traceback), `axes.rs` (Axis trait, AxisValue, Direction, PairContext, the 8 axes, ALL_AXES, panel builder, --fields/--metric handling), `verdict.rs` (Verdict + verdict()), `confusables_data.rs` (generated, unchanged).
3. **Replace the "confusable model" / "Modes and structure" / "Output contract" sections** to describe the 8-axis panel (list the keys + meanings from the spec table), the two-phase base/derived computation, `--metric <axis-key>` (default `skeleton_damerau`, numeric only — bool axes rejected), `--fields` validating against axis keys, and the removal of `--hogl-weight`/`homoglyph_damerau`/`normalized`/`skeleton_normalized`/the `Field` enum.
4. **Flag `uts39_confusable_count` and `uts39_skeleton_delta` as experimental, may change.**
5. **Note the breaking JSON-contract change** prominently (downstream parsers must update key names) and that this is v0.3.0.
6. Keep the "Regenerating the confusables table" and `gen_confusables.py` sections as-is (still accurate).
7. The `cargo test` note currently says "6 unit tests in src/main.rs"; replace with: unit tests live in each module's `#[cfg(test)]` block; `cargo test` runs all. Drop the `cargo test digit_letter_confusable` example or keep it (the test still exists in `distance.rs`).

- [ ] **Step 3: Update `README.md`**

Mirror the same changes for end users: the axis list + meanings, an example single-pair output showing the 8 rows + a verdict, an example JSON line with the new keys, the `--metric <axis>` usage (default `skeleton_damerau`), and a prominent **breaking change** note for upgraders from 0.2.x (key renames/removals, `--hogl-weight` gone). Update any example invocation using `--hogl-weight`/`-w`, `--metric homoglyph|skeleton`, or referencing `homoglyph_damerau`/`normalized`.

- [ ] **Step 4: Verify docs match the binary**

Run `cargo run -- --help` and confirm the README/CLAUDE `AXES:` block matches the help text. Run `cargo run -- -j paypal "p$(printf 'а')ypal"` and confirm the README's example JSON line has the same keys in the same order.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md README.md
git commit -m "$(cat <<'EOF'
docs: rewrite CLAUDE.md and README for the v0.3.0 axis panel

Describe the 8-axis panel, the five-module layout, --metric <axis-key>,
--fields against axis keys, and the removal of --hogl-weight /
homoglyph_damerau / normalized / skeleton_normalized. Flag the two uts39_*
axes as experimental and note the breaking JSON-contract change.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Final full-suite verification

A clean-room verification gate before declaring Phase 1 done. No code changes; if anything fails, fix it under TDD and re-run.

- [ ] **Step 1: Full verification**

Run each and confirm:
```bash
cargo fmt --check                              # clean
cargo clippy --all-targets -- -D warnings      # no warnings
cargo test                                     # all tests pass
cargo build --release                          # release binary builds
./target/release/sqdist -v                     # prints "sqdist 0.3.0 (<sha>)"
```
Expected: fmt clean, no clippy warnings, all tests green, release builds, version is `0.3.0`.

- [ ] **Step 2: Confirm the spec's test checklist is satisfied**

Cross-check against the spec's Testing section — each item must have a passing test:
- per-axis values + directions (axes.rs)
- `equal` true/false; classic levenshtein/damerau; skeleton variants catch m/rn; `uts39_confusable_count` counts Cyrillic-а and ignores a real edit; `uts39_skeleton_delta` saturates at 0; `confusable_only` true for GO0GLE/GOOGLE and rnicrosoft/microsoft, false for devflovv/devflow (axes.rs)
- panel two-phase ordering / derived reads base / canonical emit order (axes.rs)
- alignment traceback: sub count for equal/unequal length + deterministic on ties (distance.rs)
- verdict: all categories, behavior-equivalent (verdict.rs)
- `--metric`: accepts numeric, rejects bool + unknown (axes.rs + main.rs)
- `--fields`: validates, canonical order, dedup (axes.rs + main.rs)
- preserved CLI: batch exit codes, streaming vs buffered, --version, --len-tolerance range, input/match keys (main.rs)

If any item lacks a test, add it (failing → implement-if-needed → pass), then commit separately.

- [ ] **Step 3: Report completion**

Report to the user: Phase 1 (multi-axis architecture refactor, v0.3.0) complete — summarize what changed, confirm the verification gate passed, and note that nothing has been pushed or released (per the kickoff). Stop for review before Phase 2.

---

## Self-review notes (for the executor)

- **Spec coverage:** Every spec section maps to a task — core abstraction (T3), PairContext (T3), two-phase panel (T4), the 8 axes (T4), alignment traceback (T2), removed fields/flags (T7), verdict re-sourced (T6), CLI changes (T5+T7), file layout (T1–T7), output format (T7), version bump (T8), migration/docs (T9), testing (throughout + T10).
- **Known tricky spots:** (1) Task 1 straddles old+new code — the weighted-shim rename is the fiddliest part; if it fights you, fold Tasks 1–7 tighter, but never commit a state where `cargo test`/clippy fail. (2) `ALL_AXES` as a static of trait objects — `&[&dyn Axis]` works because the trait is `Sync`; fall back to `&[&(dyn Axis + Sync)]` if the compiler asks. (3) Float formatting: no Float axis ships in v0.3.0, but `AxisValue::Float` formatting (`{:.4}`) is implemented and unit-tested so Phase 3's kbd axis can rely on it.
- **No N/A variant yet:** the research doc flags that the kbd axis (Phase 3) needs an `AxisValue::NA`. That is explicitly out of scope here (deferred to Phase 3's spec). Do NOT add it now.
```
