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

/// True when the OSA Damerau distance of `a`→`b` is at most `k`.
///
/// Banded DP: only cells with `|i - j| <= k` can lie on a path of cost `<= k`,
/// so this is `O(n*k)` instead of `O(n*m)`, with no traceback and one row
/// allocation. Used purely as a gate, so it answers the predicate, not the
/// distance. Generic over the element type so callers can pass `&[u8]` for
/// ASCII (no char collect) or `&[char]` otherwise.
pub fn damerau_within<T: PartialEq>(a: &[T], b: &[T], k: usize) -> bool {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > k {
        return false;
    }
    // `INF` is any value that can never be beaten down to `<= k`.
    const INF: u32 = u32::MAX / 4;
    let k32 = k as u32;
    let cols = m + 1;
    let mut prev2 = vec![INF; cols]; // row i-2, for the transposition step
    let mut prev = vec![INF; cols]; // row i-1
    let mut cur = vec![INF; cols];
    for (j, cell) in prev.iter_mut().take(m.min(k) + 1).enumerate() {
        *cell = j as u32;
    }
    for i in 1..=n {
        // Only the band |i - j| <= k can matter.
        let lo = i.saturating_sub(k);
        let hi = (i + k).min(m);
        cur[..cols].fill(INF);
        if lo == 0 {
            cur[0] = i as u32;
        }
        for j in lo.max(1)..=hi {
            let sub = prev[j - 1].saturating_add(u32::from(a[i - 1] != b[j - 1]));
            let del = prev[j].saturating_add(1);
            let ins = cur[j - 1].saturating_add(1);
            let mut best = sub.min(del).min(ins);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(prev2[j - 2].saturating_add(1));
            }
            cur[j] = best;
        }
        // Whole band over `k` means every surviving path already costs more.
        if cur[lo..=hi].iter().all(|&v| v > k32) {
            return false;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m] <= k32
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
///
/// Also returns the Damerau (OSA) distance — the bottom-right cell of the matrix
/// this already fills, so callers needing both do not fill it twice.
pub fn align(a: &[char], b: &[char]) -> (Vec<AlignOp>, u64) {
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
    (ops, d[idx(n, m)])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cv(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn align_cost_matches_damerau() {
        // The axes layer takes the Damerau distance from align's matrix instead of
        // filling it twice. That is only sound if the two always agree.
        for (a, b) in [
            ("", ""),
            ("", "abc"),
            ("abc", ""),
            ("abc", "abc"),
            ("kitten", "sitting"),
            ("ca", "ac"),
            ("requests", "reqeusts"),
            ("paypal", "paypa1"),
            ("a", "bcdefg"),
            ("vv", "w"),
        ] {
            assert_eq!(
                align(&cv(a), &cv(b)).1,
                damerau(&cv(a), &cv(b)),
                "{a} vs {b}"
            );
        }
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
        let ops = align(&a, &b).0;
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
        let ops = align(&cv("ab"), &cv("abc")).0;
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
        let ops = align(&cv("ca"), &cv("ac")).0;
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
        let ops1 = align(&cv("abc"), &cv("xyz")).0;
        let ops2 = align(&cv("abc"), &cv("xyz")).0;
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
        let ops = align(&a, &b).0;
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

    #[test]
    fn damerau_within_agrees_with_damerau() {
        // The banded gate must answer exactly `damerau(a, b) <= k`, or it will
        // silently drop rows the filter would have kept.
        let words = [
            "",
            "a",
            "ab",
            "requests",
            "reqeusts",
            "rquests",
            "requestss",
            "lodash",
            "lodahs",
            "xylophone",
            "paypa1",
            "abcdefgh",
            "hgfedcba",
            "aaaa",
            "aaab",
        ];
        for a in words {
            for b in words {
                let (ca, cb): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
                let d = damerau(&ca, &cb);
                for k in 0..=5u64 {
                    assert_eq!(
                        damerau_within(&ca, &cb, k as usize),
                        d <= k,
                        "{a} vs {b} at k={k} (damerau={d})"
                    );
                    assert_eq!(
                        damerau_within(a.as_bytes(), b.as_bytes(), k as usize),
                        d <= k,
                        "bytes {a} vs {b} at k={k}"
                    );
                }
            }
        }
    }
}
