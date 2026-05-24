//! The single-pair human verdict: classify a scored pair as IDENTICAL,
//! LIKELY SPOOF, or LIKELY BENIGN. Reads the axis panel; behavior is
//! equivalent to v0.2.0 (uts39_skeleton_delta replaces the old
//! damerau-skeleton numerator; confusable_only is unchanged in meaning).

// TEMPORARY: verdict.rs is not consumed by the binary target until Task 7
// rewires main.rs. Until then its items are "dead" from the binary's view.
// Removed in Task 7.
#![allow(dead_code)]

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
