//! The axis panel: independent similarity signals over a string pair.
//!
//! An `Axis` is one signal (`key`, `direction`, `compute`). `ALL_AXES` lists
//! every axis in canonical order — the single source of truth for JSON keys,
//! human-row order, and `--fields`/`--metric` validation. `PairContext` holds
//! the once-per-pair precomputation (char vecs, skeletons, alignment).

// TEMPORARY: axes.rs is not consumed by the binary target until Task 7 wires
// main.rs onto the panel. Until then its public items are "dead" from the
// binary's view. Removed in Task 7.
#![allow(dead_code)]

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
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
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
        PairContext {
            a,
            b,
            ca,
            cb,
            ska,
            skb,
            sva,
            svb,
            align,
        }
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
        ALL_AXES
            .iter()
            .position(|ax| ax.key() == *k)
            .unwrap_or(usize::MAX)
    });
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            run("GO0GLE", "GOOGLE").get("confusable_only"),
            Some(AxisValue::Bool(true))
        );
        // rnicrosoft/microsoft: multi-char confusable, unequal length, still true.
        assert_eq!(
            run("rnicrosoft", "microsoft").get("confusable_only"),
            Some(AxisValue::Bool(true))
        );
        // devflovv/devflow: vv/w gap -> NOT confusable_only (regression test).
        assert_eq!(
            run("devflovv", "devflow").get("confusable_only"),
            Some(AxisValue::Bool(false))
        );
        // Identical strings are not confusable_only.
        assert_eq!(
            run("abc", "abc").get("confusable_only"),
            Some(AxisValue::Bool(false))
        );
    }

    #[test]
    fn derived_axes_read_base_values() {
        let p = run("rnicrosoft", "microsoft");
        assert!(p.get("uts39_skeleton_delta").is_some());
        assert!(p.get("confusable_only").is_some());
    }

    #[test]
    fn axis_directions() {
        let by_key = |k: &str| {
            ALL_AXES
                .iter()
                .find(|ax| ax.key() == k)
                .unwrap()
                .direction()
        };
        assert_eq!(by_key("levenshtein"), Direction::HigherMoreDifferent);
        assert_eq!(
            by_key("uts39_confusable_count"),
            Direction::HigherMoreSimilar
        );
        assert_eq!(by_key("uts39_skeleton_delta"), Direction::HigherMoreSimilar);
    }

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
