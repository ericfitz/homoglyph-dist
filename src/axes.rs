//! The axis panel: independent similarity signals over a string pair.
//!
//! An `Axis` is one signal (`key`, `direction`, `compute`). `ALL_AXES` lists
//! every axis in canonical order — the single source of truth for JSON keys,
//! human-row order, and `--fields`/`--metric` validation. `PairContext` holds
//! the once-per-pair precomputation (char vecs, skeletons, alignment).

use crate::confusables::ConfusableMap;
use crate::distance::{self, AlignOp};
use unicode_security::{GeneralSecurityProfile, RestrictionLevel, RestrictionLevelDetection};

/// The value an axis produces. The output formatter renders each variant.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AxisValue {
    Int(u64),
    Float(f64),
    Bool(bool),
    /// The axis is not applicable to this pair (e.g. keyboard_distance on
    /// non-ASCII input). Renders as JSON null / human "n/a".
    NA,
}

impl AxisValue {
    /// JSON rendering: ints/bools unquoted, floats at fixed precision.
    pub fn to_json(self) -> String {
        match self {
            AxisValue::Int(n) => n.to_string(),
            AxisValue::Float(f) => format!("{f:.4}"),
            AxisValue::Bool(b) => b.to_string(),
            AxisValue::NA => "null".to_string(),
        }
    }

    /// Human rendering — delegates to to_json except NA renders as "n/a".
    pub fn to_human(self) -> String {
        match self {
            AxisValue::NA => "n/a".to_string(),
            other => other.to_json(),
        }
    }

    /// Numeric coercion for `--metric`/`--sort`/`-t`. Bools and NA return None
    /// and are rejected as metric targets.
    pub fn as_f64(self) -> Option<f64> {
        match self {
            AxisValue::Int(n) => Some(n as f64),
            AxisValue::Float(f) => Some(f),
            AxisValue::Bool(_) => None,
            AxisValue::NA => None,
        }
    }
}

/// How to read an axis value when reasoning generically (verdict/ranking).
/// Not emitted in output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // carried for future ranking; see design spec
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
    pub sva: Vec<char>,
    pub svb: Vec<char>,
    pub align: Vec<AlignOp>,
    /// Damerau (OSA) distance of `ca`/`cb` — the bottom-right cell of the matrix
    /// `align` already filled, so the `damerau` axis does not fill it a second time.
    pub damerau: u64,
    pub cmap: &'a ConfusableMap,
}

impl<'a> PairContext<'a> {
    pub fn new(a: &'a str, b: &'a str, cmap: &'a ConfusableMap) -> Self {
        let ca: Vec<char> = a.chars().collect();
        let cb: Vec<char> = b.chars().collect();
        let sva = cmap.skeleton_chars(a);
        let svb = cmap.skeleton_chars(b);
        let (align, damerau) = distance::align(&ca, &cb);
        PairContext {
            a,
            b,
            ca,
            cb,
            sva,
            svb,
            align,
            damerau,
            cmap,
        }
    }
}

/// One independent similarity signal. Base axes read `ctx`; derived axes read
/// the base results via `base` (and ignore `ctx`).
pub trait Axis: Sync {
    /// Stable JSON key / human label.
    fn key(&self) -> &'static str;
    /// How to interpret the value (verdict/ranking); not emitted.
    #[allow(dead_code)] // carried for future ranking; see design spec
    fn direction(&self) -> Direction;
    /// Computation phase.
    fn phase(&self) -> Phase;
    /// Compute the value. `base` is the already-computed base map; it is empty
    /// during the base phase and fully populated for derived axes.
    fn compute(&self, ctx: &PairContext, base: &Panel) -> AxisValue;
}

/// Map a UTS#39 restriction level to its 0–5 ordinal (lower = more restrictive
/// = safer). Explicit match so the mapping is stable if the enum is reordered.
/// UTS#39 restriction level of one string, as an ordinal.
///
/// ASCII fast path: `detect_restriction_level` intersects an `AugmentedScriptSet`
/// per char, which is pure waste on the ASCII names that dominate real corpora.
/// An all-ASCII string is `ASCIIOnly` (0) unless some char is outside the UTS#39
/// identifier profile, in which case the crate returns `Unrestricted` (5) — so the
/// fast path is exactly equivalent, not an approximation.
fn restriction_ordinal(s: &str) -> u64 {
    if s.is_ascii() {
        return if s.chars().all(GeneralSecurityProfile::identifier_allowed) {
            0
        } else {
            5
        };
    }
    level_ordinal(s.detect_restriction_level())
}

fn level_ordinal(level: RestrictionLevel) -> u64 {
    match level {
        RestrictionLevel::ASCIIOnly => 0,
        RestrictionLevel::SingleScript => 1,
        RestrictionLevel::HighlyRestrictive => 2,
        RestrictionLevel::ModeratelyRestrictive => 3,
        RestrictionLevel::MinimallyRestrictive => 4,
        RestrictionLevel::Unrestricted => 5,
    }
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
        AxisValue::Int(ctx.damerau)
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
            .filter(|&(i, j)| ctx.cmap.confusable(ctx.ca[i], ctx.cb[j]))
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

struct ScriptRestriction;
impl Axis for ScriptRestriction {
    fn key(&self) -> &'static str {
        "script_restriction"
    }
    fn direction(&self) -> Direction {
        Direction::HigherMoreDifferent
    }
    fn phase(&self) -> Phase {
        Phase::Base
    }
    fn compute(&self, ctx: &PairContext, _base: &Panel) -> AxisValue {
        // UTS#39 restriction level of the pair: the more-suspicious (max) of the
        // two strings' levels. Latin+Cyrillic etc. has no consistent resolved
        // script and scores high — the direct mixed-script spoof signal.
        let la = restriction_ordinal(ctx.a);
        let lb = restriction_ordinal(ctx.b);
        AxisValue::Int(la.max(lb))
    }
}

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
                if let (Some((x1, y1)), Some((x2, y2))) = (
                    crate::keyboard::key_coord(ca),
                    crate::keyboard::key_coord(cb),
                ) {
                    sum += ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
                    count += 1;
                }
            }
        }
        if count == 0 {
            return AxisValue::Float(0.0);
        }
        let mean = sum / count as f32;
        let normalized = (mean / crate::keyboard::MAX_KEY_DISTANCE).clamp(0.0, 1.0);
        AxisValue::Float(normalized as f64)
    }
}

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
                return Err(format!(
                    "unknown field: {name} (valid: {})",
                    keys.join(", ")
                ));
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
    &ScriptRestriction,
    &KeyboardDistance,
];

/// Run the panel: phase 1 computes every base axis into the map (registry
/// order), phase 2 computes every derived axis (reading a snapshot of the base
/// map). Returns `Panel.entries` in canonical registry order.
pub fn build_panel(ctx: &PairContext) -> Panel {
    let mut panel = Panel {
        entries: Vec::with_capacity(ALL_AXES.len()),
    };
    // Phase 1: base axes, in registry order.
    for ax in ALL_AXES.iter().filter(|ax| ax.phase() == Phase::Base) {
        let v = ax.compute(ctx, &panel);
        panel.entries.push((ax.key(), v));
    }
    // Phase 2: derived axes read the base results. No snapshot clone is needed —
    // each value is computed before its own key is pushed, and no derived axis
    // reads another derived axis.
    for ax in ALL_AXES.iter().filter(|ax| ax.phase() == Phase::Derived) {
        let v = ax.compute(ctx, &panel);
        panel.entries.push((ax.key(), v));
    }
    // Re-order entries into canonical ALL_AXES order so emit order matches the
    // registry regardless of phase grouping.
    panel.entries.sort_unstable_by_key(|(k, _)| {
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
        let cmap = crate::confusables::ConfusableMap::uts39();
        build_panel(&PairContext::new(a, b, &cmap))
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
                "script_restriction",
                "keyboard_distance",
            ]
        );
    }

    #[test]
    fn panel_emits_in_registry_order() {
        let p = run("abc", "abd");
        let keys: Vec<&str> = p.entries.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.first(), Some(&"equal"));
        assert_eq!(keys.last(), Some(&"keyboard_distance"));
        assert_eq!(keys.len(), 10);
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
        let cmap = crate::confusables::ConfusableMap::uts39();
        let ctx = PairContext::new("paypal", "p\u{0430}ypal", &cmap);
        assert_eq!(ctx.ca.len(), 6);
        assert_eq!(ctx.cb.len(), 6);
        // Cyrillic а collapses to Latin a in the skeleton.
        assert_eq!(ctx.sva, ctx.svb);
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

    #[test]
    fn parse_fields_canonical_order_and_dedup() {
        // User order ignored; canonical order enforced; dups collapse.
        let f = parse_fields("confusable_only,damerau,damerau").unwrap();
        assert_eq!(f, vec!["damerau", "confusable_only"]);
    }

    #[test]
    fn parse_fields_all_keys() {
        // Every registry key round-trips through parse_fields (guards against
        // drift when axes are added).
        let spec = all_keys().join(",");
        let parsed = parse_fields(&spec).unwrap();
        assert_eq!(parsed.len(), all_keys().len());
        assert_eq!(parsed, all_keys());
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

    #[test]
    fn level_ordinal_maps_all_variants() {
        assert_eq!(level_ordinal(RestrictionLevel::ASCIIOnly), 0);
        assert_eq!(level_ordinal(RestrictionLevel::SingleScript), 1);
        assert_eq!(level_ordinal(RestrictionLevel::HighlyRestrictive), 2);
        assert_eq!(level_ordinal(RestrictionLevel::ModeratelyRestrictive), 3);
        assert_eq!(level_ordinal(RestrictionLevel::MinimallyRestrictive), 4);
        assert_eq!(level_ordinal(RestrictionLevel::Unrestricted), 5);
    }

    #[test]
    fn script_restriction_ascii_is_zero() {
        assert_eq!(
            run("paypal", "google").get("script_restriction"),
            Some(AxisValue::Int(0))
        );
    }

    #[test]
    fn script_restriction_homoglyph_scores_high() {
        let p = run("paypal", "p\u{0430}ypal");
        match p.get("script_restriction") {
            Some(AxisValue::Int(n)) => assert!(n >= 4, "expected >= 4, got {n}"),
            other => panic!("expected Int, got {other:?}"),
        }
    }

    #[test]
    fn script_restriction_single_script_non_latin() {
        let p = run("\u{03b1}\u{03b2}\u{03b3}", "\u{03b1}\u{03b2}\u{03b4}");
        assert_eq!(p.get("script_restriction"), Some(AxisValue::Int(1)));
    }

    #[test]
    fn script_restriction_takes_max_of_pair() {
        let mixed = "p\u{0430}ypal";
        let p = run("abc", mixed);
        match p.get("script_restriction") {
            Some(AxisValue::Int(n)) => assert!(
                n >= 4,
                "max should surface the mixed string's level, got {n}"
            ),
            other => panic!("expected Int, got {other:?}"),
        }
    }

    #[test]
    fn script_restriction_legit_japanese_not_inflated() {
        // 日本の (Han+Hiragana, no Latin) resolves to {Jpan} -> SingleScript (1),
        // NOT inflated to a mixed-script level. (Latin+Japanese would be
        // HighlyRestrictive=2.)
        let jp = "\u{65e5}\u{672c}\u{306e}";
        let p = run(jp, jp);
        assert_eq!(p.get("script_restriction"), Some(AxisValue::Int(1)));
    }

    #[test]
    fn script_restriction_is_second_to_last_and_registry_has_ten() {
        let keys: Vec<&str> = ALL_AXES.iter().map(|ax| ax.key()).collect();
        assert_eq!(keys.len(), 10);
        assert_eq!(keys[8], "script_restriction");
    }

    #[test]
    fn script_restriction_direction_and_numeric() {
        let ax = ALL_AXES
            .iter()
            .find(|ax| ax.key() == "script_restriction")
            .unwrap();
        assert_eq!(ax.direction(), Direction::HigherMoreDifferent);
        assert!(validate_metric("script_restriction").is_ok());
        assert!(numeric_keys().contains(&"script_restriction"));
    }

    #[test]
    fn axis_value_na_renders() {
        assert_eq!(AxisValue::NA.to_json(), "null");
        assert_eq!(AxisValue::NA.to_human(), "n/a");
        assert_eq!(AxisValue::NA.as_f64(), None);
    }

    #[test]
    fn keyboard_distance_identical_is_zero() {
        assert_eq!(
            run("google", "google").get("keyboard_distance"),
            Some(AxisValue::Float(0.0))
        );
    }

    #[test]
    fn keyboard_distance_pure_insert_delete_is_zero() {
        assert_eq!(
            run("gogle", "google").get("keyboard_distance"),
            Some(AxisValue::Float(0.0))
        );
    }

    #[test]
    fn keyboard_distance_non_ascii_is_na() {
        assert_eq!(
            run("paypal", "p\u{0430}ypal").get("keyboard_distance"),
            Some(AxisValue::NA)
        );
    }

    #[test]
    fn keyboard_distance_adjacent_less_than_distant() {
        let adjacent = match run("gigle", "gogle").get("keyboard_distance") {
            Some(AxisValue::Float(f)) => f,
            other => panic!("expected Float, got {other:?}"),
        };
        let distant = match run("gqgle", "gogle").get("keyboard_distance") {
            Some(AxisValue::Float(f)) => f,
            other => panic!("expected Float, got {other:?}"),
        };
        assert!(adjacent > 0.0, "adjacent sub should be > 0, got {adjacent}");
        assert!(
            adjacent < distant,
            "adjacent ({adjacent}) should be < distant ({distant})"
        );
        assert!(
            distant <= 1.0,
            "normalized distance must be <= 1, got {distant}"
        );
    }

    #[test]
    fn keyboard_distance_is_last_and_registry_has_ten() {
        let keys: Vec<&str> = ALL_AXES.iter().map(|ax| ax.key()).collect();
        assert_eq!(keys.len(), 10);
        assert_eq!(keys.last(), Some(&"keyboard_distance"));
    }

    #[test]
    fn keyboard_distance_direction_and_numeric() {
        let ax = ALL_AXES
            .iter()
            .find(|ax| ax.key() == "keyboard_distance")
            .unwrap();
        assert_eq!(ax.direction(), Direction::HigherMoreDifferent);
        assert!(validate_metric("keyboard_distance").is_ok());
        assert!(numeric_keys().contains(&"keyboard_distance"));
    }
}
