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
        for s in [
            "microsoft",
            "paypal",
            "vvallet",
            "g\u{43E}\u{43E}gle",
            "abc123",
        ] {
            assert_eq!(
                skeleton(&skeleton(s)),
                skeleton(s),
                "not idempotent for {s}"
            );
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
