//! Stagger-aware US-QWERTY key coordinates for the keyboard_distance axis.
//!
//! Physical key positions are factual data (not copyrightable); `clavier` (MIT)
//! and dnstwist (Apache-2.0) were consulted only as cross-references. Coordinates
//! are in "key units": x = column index + a per-row stagger offset, y = row index
//! (number row 0 .. bottom row 3). Self-contained; no dependency.

/// (x, y) of a key in key-units, or None for chars not on the modelled keyboard.
/// Callers pass an already-ASCII-lowercased char (case shares a physical key).
pub fn key_coord(c: char) -> Option<(f32, f32)> {
    // (row chars, y, x stagger offset). Columns are 0-based within the row.
    const ROWS: &[(&str, f32, f32)] = &[
        ("1234567890-=", 0.0, 0.0),
        ("qwertyuiop[]", 1.0, 0.5),
        ("asdfghjkl;'", 2.0, 0.75),
        ("zxcvbnm,./", 3.0, 1.25),
    ];
    for &(chars, y, off) in ROWS {
        if let Some(col) = chars.chars().position(|k| k == c) {
            return Some((col as f32 + off, y));
        }
    }
    None
}

/// The maximum Euclidean distance between any two modelled keys — the normalizer
/// so keyboard_distance lands in [0,1]. Fixed by the table above; `max_key_distance`
/// (test-only) recomputes it and `const_matches_computed` asserts they agree bit-for-bit.
pub const MAX_KEY_DISTANCE: f32 = 11.543396;

/// Recompute [`MAX_KEY_DISTANCE`] from the table. Test-only: it is O(K^2) with a
/// `sqrt` per pair, which used to run on every scored pair.
#[cfg(test)]
pub fn max_key_distance() -> f32 {
    const ALL: &str = "1234567890-=qwertyuiop[]asdfghjkl;'zxcvbnm,./";
    let coords: Vec<(f32, f32)> = ALL.chars().filter_map(key_coord).collect();
    let mut max = 0.0f32;
    for (i, &(x1, y1)) in coords.iter().enumerate() {
        for &(x2, y2) in &coords[i + 1..] {
            let d = ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
            if d > max {
                max = d;
            }
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: char, b: char) -> f32 {
        let (x1, y1) = key_coord(a).unwrap();
        let (x2, y2) = key_coord(b).unwrap();
        ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt()
    }

    #[test]
    fn const_matches_computed() {
        assert_eq!(MAX_KEY_DISTANCE.to_bits(), max_key_distance().to_bits());
    }

    #[test]
    fn known_keys_have_coords() {
        assert!(key_coord('q').is_some());
        assert!(key_coord('m').is_some());
        assert!(key_coord('0').is_some());
        assert!(key_coord('/').is_some());
    }

    #[test]
    fn unmappable_chars_are_none() {
        assert!(key_coord('!').is_none());
        assert!(key_coord('\u{0007}').is_none());
        assert!(key_coord(' ').is_none());
    }

    #[test]
    fn adjacent_closer_than_distant() {
        assert!(dist('s', 'd') < dist('q', 'p'));
        assert!(dist('f', 'g') < dist('a', 'l'));
    }

    #[test]
    fn max_key_distance_is_positive_and_bounds_pairs() {
        let m = max_key_distance();
        assert!(m > 0.0);
        assert!(dist('q', 'p') <= m);
        assert!(dist('1', '/') <= m);
        assert_eq!(max_key_distance(), m);
    }
}
