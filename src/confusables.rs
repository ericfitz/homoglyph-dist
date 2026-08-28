//! The runtime confusable model: the active single-char skeleton map and digraph
//! rules for a run, selected by `--confusables`. Distinct from the generated
//! `confusables_data` (UTS#39), `flowcrypt_data`, and `digraph_data` tables.

use crate::confusables_data::CONFUSABLES;
use std::borrow::Cow;

/// Which supplemental confusable sources are enabled. UTS#39 is always on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub flowcrypt: bool,
    pub digraph: bool,
}

/// Parse a `--confusables` comma-list into `Sources`. `uts39` is always on
/// (listing it is a no-op; omitting it does not disable it). Unknown source →
/// Err naming the offender + valid names. Empty/whitespace → default (uts39).
pub fn parse_sources(spec: &str) -> Result<Sources, String> {
    let mut s = Sources::default();
    for raw in spec.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        match name {
            "uts39" => {}
            "flowcrypt" => s.flowcrypt = true,
            "digraph" => s.digraph = true,
            other => {
                return Err(format!(
                    "unknown confusable source: {other} (valid: uts39, flowcrypt, digraph)"
                ));
            }
        }
    }
    Ok(s)
}

/// The active confusable model for a run.
pub struct ConfusableMap {
    /// code point -> skeleton string, sorted by key. UTS#39 entries take
    /// precedence on key collision.
    singles: Cow<'static, [(u32, &'static str)]>,
    /// (digraph source, replacement); longest-match-first. Empty unless enabled.
    digraphs: &'static [(&'static str, &'static str)],
}

impl ConfusableMap {
    /// UTS#39-only map (the default).
    pub fn uts39() -> Self {
        Self::from_sources(&Sources::default())
    }

    /// Build from the enabled source set. UTS#39 is always the base; enabled
    /// supplements are merged in only for keys UTS#39 does not already define
    /// (UTS#39 wins on collision).
    pub fn from_sources(sources: &Sources) -> Self {
        // UTS#39 is the always-on base, sorted by key.
        // Borrowed unless a supplement actually adds entries.
        let mut singles: Cow<'static, [(u32, &'static str)]> = Cow::Borrowed(CONFUSABLES);
        if sources.flowcrypt {
            // Merge FlowCrypt entries only for keys UTS#39 does not already
            // define (UTS#39 wins). Collect additions first so we never
            // binary-search `singles` while mutating it, then extend + re-sort.
            let mut add: Vec<(u32, &'static str)> = Vec::new();
            for &(cp, sk) in crate::flowcrypt_data::FLOWCRYPT {
                if singles.binary_search_by(|&(k, _)| k.cmp(&cp)).is_err()
                    && !add.iter().any(|&(k, _)| k == cp)
                {
                    add.push((cp, sk));
                }
            }
            let owned = singles.to_mut();
            owned.extend(add);
            owned.sort_by_key(|&(k, _)| k);
        }
        let digraphs: &'static [(&'static str, &'static str)] = if sources.digraph {
            crate::digraph_data::DIGRAPHS
        } else {
            &[]
        };
        ConfusableMap { singles, digraphs }
    }

    /// Skeleton of one char via the single-char map. None if unmapped.
    pub fn skeleton_of(&self, c: char) -> Option<&str> {
        let cp = c as u32;
        self.singles
            .binary_search_by(|&(k, _)| k.cmp(&cp))
            .ok()
            .map(|i| self.singles[i].1)
    }

    /// Are two chars confusable under this map's single-char skeletons?
    pub fn confusable(&self, a: char, b: char) -> bool {
        if a == b {
            return true;
        }
        let mut sa_buf = [0u8; 4];
        let mut sb_buf = [0u8; 4];
        let sa = self
            .skeleton_of(a)
            .unwrap_or_else(|| a.encode_utf8(&mut sa_buf));
        let sb = self
            .skeleton_of(b)
            .unwrap_or_else(|| b.encode_utf8(&mut sb_buf));
        sa == sb
    }

    /// Full skeleton of a string as chars: longest-match-first over digraph
    /// source keys, else per-char single-char mapping, else the char unchanged.
    /// Single, non-recursive pass.
    ///
    /// Chars, not a `String`, because every caller on the hot path wants
    /// `Vec<char>` — going through a `String` cost two allocations per string.
    pub fn skeleton_chars(&self, s: &str) -> Vec<char> {
        let mut out: Vec<char> = Vec::with_capacity(s.len());
        if self.digraphs.is_empty() {
            // Default source set: no multi-char keys, so no lookahead buffer.
            for c in s.chars() {
                match self.skeleton_of(c) {
                    Some(sk) => out.extend(sk.chars()),
                    None => out.push(c),
                }
            }
            return out;
        }
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // Try digraphs longest-first. Current keys are all length 2; the
            // matcher is written to try longer keys first for future-proofing.
            let mut best_len = 0usize;
            let mut best_rep = "";
            for &(src, rep) in self.digraphs {
                let klen = src.chars().count();
                if klen > best_len
                    && i + klen <= chars.len()
                    && src.chars().eq(chars[i..i + klen].iter().copied())
                {
                    best_len = klen;
                    best_rep = rep;
                }
            }
            if best_len > 0 {
                out.extend(best_rep.chars());
                i += best_len;
            } else {
                let c = chars[i];
                match self.skeleton_of(c) {
                    Some(sk) => out.extend(sk.chars()),
                    None => out.push(c),
                }
                i += 1;
            }
        }
        out
    }

    /// Length in chars of the skeleton of `s`, without building it.
    ///
    /// Used by the list-mode emit gate: skeletons that differ in length can
    /// never be equal, and edit distance is at least the length difference.
    pub fn skeleton_len(&self, s: &str) -> usize {
        if !self.digraphs.is_empty() {
            return self.skeleton_chars(s).len();
        }
        s.chars()
            .map(|c| self.skeleton_of(c).map_or(1, |sk| sk.chars().count()))
            .sum()
    }

    /// String form of [`ConfusableMap::skeleton_chars`]. Test-only: the hot
    /// path wants chars.
    #[cfg(test)]
    pub fn skeleton(&self, s: &str) -> String {
        self.skeleton_chars(s).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> ConfusableMap {
        ConfusableMap::uts39()
    }

    #[test]
    fn digit_letter_confusable() {
        let m = m();
        assert!(m.confusable('1', 'l'));
        assert!(m.confusable('0', 'O'));
        assert!(!m.confusable('0', 'o'));
        assert!(m.confusable('I', 'l'));
        assert!(!m.confusable('x', 'y'));
    }

    #[test]
    fn skeleton_maps_multichar() {
        let m = m();
        assert_eq!(m.skeleton("microsoft"), m.skeleton("rnicrosoft"));
        assert_eq!(m.skeleton("microsoft"), "rnicrosoft");
    }

    #[test]
    fn skeleton_is_idempotent_for_uts39() {
        let m = m();
        for s in [
            "microsoft",
            "paypal",
            "vvallet",
            "g\u{43E}\u{43E}gle",
            "abc123",
        ] {
            assert_eq!(
                m.skeleton(&m.skeleton(s)),
                m.skeleton(s),
                "not idempotent for {s}"
            );
        }
    }

    #[test]
    fn skeleton_collapses_homoglyphs() {
        let m = m();
        assert_eq!(m.skeleton("p\u{0430}ypal"), m.skeleton("paypal"));
        assert_eq!(m.skeleton("xyz"), "xyz");
        assert_eq!(m.skeleton(""), "");
    }

    #[test]
    fn vv_w_and_cl_d_are_not_uts39_confusables_by_default() {
        // Default (uts39 only): the vv/w, cl/d gaps STAY (digraph source off).
        let m = m();
        assert_ne!(m.skeleton("vv"), m.skeleton("w"));
        assert_ne!(m.skeleton("cl"), m.skeleton("d"));
        assert_eq!(m.skeleton("m"), "rn"); // m->rn IS uts39, for contrast.
    }

    #[test]
    fn parse_sources_default_and_all() {
        assert_eq!(parse_sources("uts39").unwrap(), Sources::default());
        assert_eq!(parse_sources("").unwrap(), Sources::default());
        let all = parse_sources("uts39,flowcrypt,digraph").unwrap();
        assert!(all.flowcrypt && all.digraph);
        // dedup / order-insensitive
        assert_eq!(
            parse_sources("digraph,digraph").unwrap(),
            Sources {
                flowcrypt: false,
                digraph: true
            }
        );
    }

    #[test]
    fn parse_sources_rejects_unknown() {
        let e = parse_sources("uts39,bogus").unwrap_err();
        assert!(e.contains("bogus"), "names offender: {e}");
        assert!(e.contains("flowcrypt"), "lists valid: {e}");
    }

    #[test]
    fn digraph_source_closes_gaps_when_enabled() {
        let m = ConfusableMap::from_sources(&Sources {
            flowcrypt: false,
            digraph: true,
        });
        // vv->w, cl->d close; nn anchors to m's UTS#39 skeleton "rn" so nn unifies with m.
        assert_eq!(m.skeleton("vv"), m.skeleton("w"));
        assert_eq!(m.skeleton("devflovv"), m.skeleton("devflow"));
        assert_eq!(m.skeleton("cl"), m.skeleton("d"));
        assert_eq!(m.skeleton("nn"), m.skeleton("m")); // both -> "rn"
                                                       // rn<->m is handled by UTS#39 (m->rn), not the digraph table:
        assert_eq!(m.skeleton("rn"), m.skeleton("m"));
        // Default (digraph off) still has the vv/w gap (regression pin).
        let d = ConfusableMap::uts39();
        assert_ne!(d.skeleton("vv"), d.skeleton("w"));
    }

    #[test]
    fn digraph_does_not_break_uts39_rn_m() {
        // rn<->m must STILL unify with digraph enabled (UTS#39 m->rn owns it;
        // we deliberately omit a rn->m digraph that would invert it).
        let m = ConfusableMap::from_sources(&Sources {
            flowcrypt: false,
            digraph: true,
        });
        assert_eq!(m.skeleton("rnicrosoft"), m.skeleton("microsoft"));
    }

    #[test]
    fn digraph_longest_match_first() {
        // A digraph match is taken before the per-char mapping at that position.
        let m = ConfusableMap::from_sources(&Sources {
            flowcrypt: false,
            digraph: true,
        });
        // "cl" -> "d": the whole digraph collapses, not c then l.
        assert_eq!(m.skeleton("cl"), "d");
        // mid-word: "vvallet" -> "wallet"-skeleton.
        assert_eq!(m.skeleton("vvallet"), m.skeleton("wallet"));
    }

    #[test]
    fn flowcrypt_source_collapses_lookalike_when_enabled() {
        // U+00E0 (à) is a FlowCrypt look-alike of ASCII 'a' (from flowcrypt_data.rs)
        // and is NOT a UTS#39 confusable, so it only collapses with flowcrypt on.
        let look = '\u{00E0}';
        let off = ConfusableMap::uts39();
        let on = ConfusableMap::from_sources(&Sources {
            flowcrypt: true,
            digraph: false,
        });
        assert!(!off.confusable(look, 'a'), "off by default");
        assert!(on.confusable(look, 'a'), "collapses to 'a' under flowcrypt");
    }

    #[test]
    fn uts39_wins_over_flowcrypt_on_collision() {
        // Cyrillic а (U+0430) is in BOTH UTS#39 (-> 'a') and the FlowCrypt table.
        // With flowcrypt on, the UTS#39 mapping must be used (precedence), so it
        // still skeletons to 'a' — never double-inserted or overridden.
        let on = ConfusableMap::from_sources(&Sources {
            flowcrypt: true,
            digraph: false,
        });
        assert_eq!(on.skeleton("\u{0430}"), on.skeleton("a"));
    }
}
