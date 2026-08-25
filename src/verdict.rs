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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TyposquatClass {
    Identical,
    SameProject,
    LikelyTyposquat,
    PossibleCombosquat,
    Unrelated,
}

impl TyposquatClass {
    pub fn tag(self) -> &'static str {
        match self {
            TyposquatClass::Identical => "IDENTICAL",
            TyposquatClass::SameProject => "SAME PROJECT",
            TyposquatClass::LikelyTyposquat => "LIKELY TYPOSQUAT",
            TyposquatClass::PossibleCombosquat => "POSSIBLE COMBOSQUAT",
            TyposquatClass::Unrelated => "UNRELATED",
        }
    }
    pub fn json_key(self) -> &'static str {
        match self {
            TyposquatClass::Identical => "identical",
            TyposquatClass::SameProject => "same_project",
            TyposquatClass::LikelyTyposquat => "likely_typosquat",
            TyposquatClass::PossibleCombosquat => "possible_combosquat",
            TyposquatClass::Unrelated => "unrelated",
        }
    }
}

/// True when the shorter scored name (min 3 chars) is a delimited prefix,
/// suffix, or token of the longer. Separators: `-`, `_`, `.`, `/`.
/// `react` vs `reactive` is false (no delimiter); `lodash` vs `lodash-utils`
/// is true.
fn is_possible_combosquat(a: &str, b: &str) -> bool {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    if a == b {
        return false;
    }
    let (short, long) = if a.chars().count() <= b.chars().count() {
        (a.as_str(), b.as_str())
    } else {
        (b.as_str(), a.as_str())
    };
    if short.chars().count() < 3 {
        return false;
    }
    const SEPS: [char; 4] = ['-', '_', '.', '/'];
    for sep in SEPS {
        let prefix = format!("{short}{sep}");
        let suffix = format!("{sep}{short}");
        if long.starts_with(&prefix) || long.ends_with(&suffix) {
            return true;
        }
    }
    long.split(|c| SEPS.contains(&c)).any(|tok| tok == short)
}

pub fn classify_typosquat(
    originals_equal: bool,
    same_project: bool,
    panel: &Panel,
    scored_a: &str,
    scored_b: &str,
) -> (TyposquatClass, String) {
    if originals_equal {
        return (
            TyposquatClass::Identical,
            "The strings are identical.".into(),
        );
    }
    if same_project {
        return (
            TyposquatClass::SameProject,
            "The names differ only by registry normalization (same project).".into(),
        );
    }
    let dam = int_axis(panel, "damerau");
    let confusable_only = bool_axis(panel, "confusable_only");
    let long = scored_a.chars().count().max(scored_b.chars().count());
    let likely = (confusable_only || dam <= 1) && long >= 3;
    if likely {
        let reason = if confusable_only {
            "Strings differ but share a confusable skeleton (visual lookalike).".into()
        } else {
            let mut r = "1 Damerau edit.".to_string();
            if let Some(AxisValue::Float(k)) = panel.get("keyboard_distance") {
                if k.abs() < 1e-12 {
                    r.push_str(" Keyboard distance 0 (no far-key substitutions).");
                }
            }
            r
        };
        return (TyposquatClass::LikelyTyposquat, reason);
    }
    if is_possible_combosquat(scored_a, scored_b) {
        return (
            TyposquatClass::PossibleCombosquat,
            "One name is the other plus a delimited affix (possible combosquat).".into(),
        );
    }
    (
        TyposquatClass::Unrelated,
        "No typosquat signal (damerau > 1 and not confusable-only).".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axes::{build_panel, AxisValue, PairContext};

    fn panel(a: &str, b: &str) -> Panel {
        let cmap = crate::confusables::ConfusableMap::uts39();
        build_panel(&PairContext::new(a, b, &cmap))
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

    use super::TyposquatClass;

    fn class(a: &str, b: &str, same_project: bool) -> TyposquatClass {
        let p = panel(a, b);
        classify_typosquat(a == b, same_project, &p, a, b).0
    }

    #[test]
    fn typosquat_class_labels() {
        assert_eq!(TyposquatClass::Identical.tag(), "IDENTICAL");
        assert_eq!(TyposquatClass::Identical.json_key(), "identical");
        assert_eq!(TyposquatClass::SameProject.tag(), "SAME PROJECT");
        assert_eq!(TyposquatClass::SameProject.json_key(), "same_project");
        assert_eq!(TyposquatClass::LikelyTyposquat.tag(), "LIKELY TYPOSQUAT");
        assert_eq!(
            TyposquatClass::LikelyTyposquat.json_key(),
            "likely_typosquat"
        );
        assert_eq!(
            TyposquatClass::PossibleCombosquat.tag(),
            "POSSIBLE COMBOSQUAT"
        );
        assert_eq!(
            TyposquatClass::PossibleCombosquat.json_key(),
            "possible_combosquat"
        );
        assert_eq!(TyposquatClass::Unrelated.tag(), "UNRELATED");
        assert_eq!(TyposquatClass::Unrelated.json_key(), "unrelated");
    }

    #[test]
    fn typosquat_identical() {
        assert_eq!(class("lodash", "lodash", false), TyposquatClass::Identical);
    }

    #[test]
    fn typosquat_same_project_wins_over_damerau() {
        // Raw pair would be damerau=1; identity-gate must win.
        assert_eq!(
            class("foo_bar", "foo-bar", true),
            TyposquatClass::SameProject
        );
    }

    #[test]
    fn typosquat_lodahs_is_likely() {
        assert_eq!(
            class("lodash", "lodahs", false),
            TyposquatClass::LikelyTyposquat
        );
    }

    #[test]
    fn typosquat_1odash_visual() {
        assert_eq!(
            class("lodash", "1odash", false),
            TyposquatClass::LikelyTyposquat
        );
        let p = panel("lodash", "1odash");
        assert!(matches!(
            p.get("confusable_only"),
            Some(AxisValue::Bool(true))
        ));
    }

    #[test]
    fn typosquat_rnicrosoft_despite_damerau_2() {
        assert_eq!(
            class("microsoft", "rnicrosoft", false),
            TyposquatClass::LikelyTyposquat
        );
        let p = panel("microsoft", "rnicrosoft");
        match p.get("damerau") {
            Some(AxisValue::Int(n)) => assert!(n >= 2, "damerau={n}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typosquat_go0gle() {
        assert_eq!(
            class("google", "go0gle", false),
            TyposquatClass::LikelyTyposquat
        );
    }

    #[test]
    fn typosquat_short_unrelated() {
        assert_eq!(class("ab", "ac", false), TyposquatClass::Unrelated);
    }

    #[test]
    fn typosquat_combosquat_affix() {
        assert_eq!(
            class("lodash", "lodash-utils", false),
            TyposquatClass::PossibleCombosquat
        );
        assert_eq!(
            class("requests", "python-requests", false),
            TyposquatClass::PossibleCombosquat
        );
        assert_eq!(
            class("lodash", "my-lodash-utils", false),
            TyposquatClass::PossibleCombosquat
        );
    }

    #[test]
    fn typosquat_not_combosquat_without_delimiter() {
        assert_eq!(class("react", "reactive", false), TyposquatClass::Unrelated);
        assert_eq!(
            class("lodash", "lodashutils", false),
            TyposquatClass::Unrelated
        );
        assert_eq!(
            class("lodash", "xylophone", false),
            TyposquatClass::Unrelated
        );
    }
}
