//! Edit-distance algorithms: Levenshtein and Damerau (OSA) + the alignment traceback.
//!
//! All distances here are UNWEIGHTED (integer): Levenshtein and Damerau
//! (OSA — adjacent transpositions). Homoglyph awareness is expressed by the
//! axes layer via skeletons and the alignment traceback, not by weighting the
//! edit cost.

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
        assert_eq!(
            ops.iter()
                .filter(|o| matches!(o, AlignOp::Sub(_, _)))
                .count(),
            0
        );
        assert_eq!(ops.iter().filter(|o| matches!(o, AlignOp::Ins)).count(), 1);
    }

    #[test]
    fn align_marks_transposition() {
        // "ca" vs "ac": one transposition, no substitutions.
        let ops = align(&cv("ca"), &cv("ac"));
        assert_eq!(
            ops.iter()
                .filter(|o| matches!(o, AlignOp::Sub(_, _)))
                .count(),
            0
        );
        assert_eq!(
            ops.iter()
                .filter(|o| matches!(o, AlignOp::Transpose))
                .count(),
            1
        );
    }

    #[test]
    fn align_is_deterministic_on_ties() {
        // Equal-length unrelated strings: every position substitutes.
        let ops1 = align(&cv("abc"), &cv("xyz"));
        let ops2 = align(&cv("abc"), &cv("xyz"));
        assert_eq!(ops1, ops2);
        assert_eq!(
            ops1.iter()
                .filter(|o| matches!(o, AlignOp::Sub(_, _)))
                .count(),
            3
        );
    }

    #[test]
    fn align_substitution_indices_address_confusable() {
        // "paypal" vs "pаypal" (Cyrillic а at index 1): one Sub(1,1) whose chars
        // are confusable.
        let a = cv("paypal");
        let b = cv("p\u{0430}ypal");
        let ops = align(&a, &b);
        let cmap = crate::confusables::ConfusableMap::uts39();
        let confusable_subs = ops
            .iter()
            .filter_map(|op| match op {
                AlignOp::Sub(i, j) => Some((*i, *j)),
                _ => None,
            })
            .filter(|&(i, j)| cmap.confusable(a[i], b[j]))
            .count();
        assert_eq!(confusable_subs, 1);
    }
}
