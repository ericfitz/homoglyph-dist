//! sqdist — string distance for typosquatting / homoglyph detection.
//!
//! Computes Levenshtein, Damerau-Levenshtein, and a homoglyph-aware
//! (UTS#39 confusable-skeleton) weighted distance between two strings.

mod confusables_data;

use confusables_data::CONFUSABLES;
use std::env;
use std::process::ExitCode;

/// Look up the confusable skeleton for a single char.
fn skeleton_of(c: char) -> Option<&'static str> {
    let cp = c as u32;
    CONFUSABLES
        .binary_search_by(|&(k, _)| k.cmp(&cp))
        .ok()
        .map(|i| CONFUSABLES[i].1)
}

/// Are two chars confusable under UTS#39 skeleton equality?
fn confusable(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let mut sa_buf = [0u8; 4];
    let mut sb_buf = [0u8; 4];
    let sa = skeleton_of(a).unwrap_or_else(|| a.encode_utf8(&mut sa_buf));
    let sb = skeleton_of(b).unwrap_or_else(|| b.encode_utf8(&mut sb_buf));
    sa == sb
}

/// Substitution cost under the chosen model.
fn sub_cost(a: char, b: char, homoglyph: bool, homo_weight: f64) -> f64 {
    if a == b {
        0.0
    } else if homoglyph && confusable(a, b) {
        homo_weight
    } else {
        1.0
    }
}

/// Plain / weighted Levenshtein (no transposition).
fn levenshtein(a: &[char], b: &[char], homoglyph: bool, w: f64) -> f64 {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m as f64;
    }
    if m == 0 {
        return n as f64;
    }
    let mut prev: Vec<f64> = (0..=m).map(|x| x as f64).collect();
    let mut cur = vec![0.0f64; m + 1];
    for i in 1..=n {
        cur[0] = i as f64;
        for j in 1..=m {
            let s = prev[j - 1] + sub_cost(a[i - 1], b[j - 1], homoglyph, w);
            let del = prev[j] + 1.0;
            let ins = cur[j - 1] + 1.0;
            cur[j] = s.min(del).min(ins);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

/// Weighted Damerau-Levenshtein with adjacent transpositions (OSA variant).
fn damerau(a: &[char], b: &[char], homoglyph: bool, w: f64) -> f64 {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m as f64;
    }
    if m == 0 {
        return n as f64;
    }
    let cols = m + 1;
    let mut d = vec![0.0f64; (n + 1) * cols];
    let idx = |i: usize, j: usize| i * cols + j;
    for i in 0..=n {
        d[idx(i, 0)] = i as f64;
    }
    for j in 0..=m {
        d[idx(0, j)] = j as f64;
    }
    for i in 1..=n {
        for j in 1..=m {
            let s = d[idx(i - 1, j - 1)] + sub_cost(a[i - 1], b[j - 1], homoglyph, w);
            let del = d[idx(i - 1, j)] + 1.0;
            let ins = d[idx(i, j - 1)] + 1.0;
            let mut best = s.min(del).min(ins);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                let trans = d[idx(i - 2, j - 2)] + 1.0;
                if trans < best {
                    best = trans;
                }
            }
            d[idx(i, j)] = best;
        }
    }
    d[idx(n, m)]
}

struct Scores {
    lev: u64,
    dam: u64,
    homo: f64,
    norm: f64,
    confusable_only: bool,
}

fn score_pair(a: &str, b: &str, homo_weight: f64) -> Scores {
    let ca: Vec<char> = a.chars().collect();
    let cb: Vec<char> = b.chars().collect();
    let lev = levenshtein(&ca, &cb, false, 0.0);
    let dam = damerau(&ca, &cb, false, 0.0);
    let homo = damerau(&ca, &cb, true, homo_weight);
    let maxlen = ca.len().max(cb.len()).max(1) as f64;
    let confusable_only = a != b
        && ca.len() == cb.len()
        && ca.iter().zip(cb.iter()).all(|(&x, &y)| confusable(x, y));
    Scores {
        lev: lev as u64,
        dam: dam as u64,
        homo,
        norm: homo / maxlen,
        confusable_only,
    }
}

fn emit(a: &str, b: &str, s: &Scores, json: bool) {
    if json {
        println!(
            "{{\"a\":{:?},\"b\":{:?},\"levenshtein\":{},\"damerau\":{},\"homoglyph_damerau\":{},\"normalized\":{:.4},\"confusable_only\":{}}}",
            a, b, s.lev, s.dam, s.homo, s.norm, s.confusable_only
        );
    } else {
        println!("levenshtein         {}", s.lev);
        println!("damerau             {}", s.dam);
        println!("homoglyph_damerau   {}", s.homo);
        println!("normalized          {:.4}", s.norm);
        println!("confusable_only     {}", s.confusable_only);
    }
}

struct Opts {
    homo_weight: f64,
    json: bool,
    threshold: Option<f64>,
    stdin: bool,
}

fn print_usage() {
    eprintln!(
        "sqdist - typosquat / homoglyph string distance\n\n\
         USAGE:\n    sqdist [OPTIONS] <STRING_A> <STRING_B>\n\n\
         OPTIONS:\n\
         \x20   -w, --homo-weight <F>   Cost of a homoglyph substitution (default 0.1)\n\
         \x20   -t, --threshold <F>     Exit 0 if homoglyph distance <= F (alert), else 1\n\
         \x20   -s, --stdin             Batch mode: read TAB- or comma-separated pairs from\n\
         \x20                           stdin, emit one JSON object per line. With -t, only\n\
         \x20                           lines at/under the threshold are emitted (alerts).\n\
         \x20   -j, --json              Emit JSON (single-pair mode)\n\
         \x20   -h, --help              This help\n\n\
         OUTPUT (default): levenshtein, damerau, homoglyph_damerau, normalized, confusable_only\n"
    );
}

fn parse_args() -> Result<(String, String, Opts), String> {
    let mut args = env::args().skip(1);
    let mut opts = Opts {
        homo_weight: 0.1,
        json: false,
        threshold: None,
        stdin: false,
    };
    let mut positionals: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-j" | "--json" => opts.json = true,
            "-s" | "--stdin" => opts.stdin = true,
            "-w" | "--homo-weight" => {
                let v = args.next().ok_or("--homo-weight needs a value")?;
                opts.homo_weight = v.parse().map_err(|_| "invalid --homo-weight")?;
            }
            "-t" | "--threshold" => {
                let v = args.next().ok_or("--threshold needs a value")?;
                opts.threshold = Some(v.parse().map_err(|_| "invalid --threshold")?);
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(format!("unknown option: {s}"));
            }
            _ => positionals.push(a),
        }
    }
    if opts.stdin {
        if !positionals.is_empty() {
            return Err("--stdin takes no positional arguments".into());
        }
        return Ok((String::new(), String::new(), opts));
    }
    if positionals.len() != 2 {
        return Err(format!("expected 2 string arguments, got {}", positionals.len()));
    }
    let b = positionals.pop().unwrap();
    let a = positionals.pop().unwrap();
    Ok((a, b, opts))
}

fn main() -> ExitCode {
    let (a, b, opts) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };

    if opts.stdin {
        use std::io::{self, BufRead, Write};
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.is_empty() {
                continue;
            }
            // Accept tab- or comma-separated pairs.
            let mut parts = line.splitn(2, |c| c == '\t' || c == ',');
            let (la, lb) = match (parts.next(), parts.next()) {
                (Some(x), Some(y)) => (x, y),
                _ => {
                    let _ = writeln!(out, "{{\"error\":\"malformed line\",\"line\":{line:?}}}");
                    continue;
                }
            };
            let s = score_pair(la, lb, opts.homo_weight);
            // In stdin mode we always emit JSON lines (one per pair) for easy parsing,
            // optionally filtered by threshold.
            if let Some(t) = opts.threshold {
                if s.homo > t {
                    continue; // only emit alerts at/under threshold
                }
            }
            let _ = writeln!(
                out,
                "{{\"a\":{:?},\"b\":{:?},\"levenshtein\":{},\"damerau\":{},\"homoglyph_damerau\":{},\"normalized\":{:.4},\"confusable_only\":{}}}",
                la, lb, s.lev, s.dam, s.homo, s.norm, s.confusable_only
            );
        }
        return ExitCode::SUCCESS;
    }

    let s = score_pair(&a, &b, opts.homo_weight);
    emit(&a, &b, &s, opts.json);

    if let Some(t) = opts.threshold {
        return if s.homo <= t { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cv(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn classic_levenshtein() {
        assert_eq!(levenshtein(&cv("kitten"), &cv("sitting"), false, 0.0), 3.0);
        assert_eq!(levenshtein(&cv("flaw"), &cv("lawn"), false, 0.0), 2.0);
        assert_eq!(levenshtein(&cv(""), &cv("abc"), false, 0.0), 3.0);
        assert_eq!(levenshtein(&cv("abc"), &cv("abc"), false, 0.0), 0.0);
    }

    #[test]
    fn transposition_is_one_edit() {
        assert_eq!(levenshtein(&cv("googel"), &cv("google"), false, 0.0), 2.0);
        assert_eq!(damerau(&cv("googel"), &cv("google"), false, 0.0), 1.0);
        assert_eq!(damerau(&cv("ca"), &cv("ac"), false, 0.0), 1.0);
    }

    #[test]
    fn homoglyph_cheaper_than_sub() {
        let spoof = "p\u{0430}ypal";
        let real = "paypal";
        assert_eq!(damerau(&cv(spoof), &cv(real), false, 0.0), 1.0);
        let h = damerau(&cv(spoof), &cv(real), true, 0.1);
        assert!((h - 0.1).abs() < 1e-9, "got {h}");
    }

    #[test]
    fn full_homoglyph_word_near_zero() {
        let spoof = "g\u{043E}\u{043E}gle";
        let real = "google";
        let h = damerau(&cv(spoof), &cv(real), true, 0.1);
        assert!((h - 0.2).abs() < 1e-9, "two homoglyphs should be 0.2, got {h}");
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
    fn identical_is_zero_everywhere() {
        assert_eq!(damerau(&cv("abc"), &cv("abc"), true, 0.1), 0.0);
    }
}
