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
