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

/// UTS#39 skeleton of a string: map each code point through the confusables
/// table (or pass it through unchanged), concatenating the results. Single,
/// non-recursive pass — the table targets are already in canonical form.
fn skeleton(s: &str) -> String {
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

/// Substitution cost under the chosen model.
fn sub_cost(a: char, b: char, homoglyph: bool, hogl_weight: f64) -> f64 {
    if a == b {
        0.0
    } else if homoglyph && confusable(a, b) {
        hogl_weight
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Metric {
    Homoglyph,
    Skeleton,
}

/// The distance used for thresholding/sorting, per the chosen metric.
fn metric_value(s: &Scores, m: Metric) -> f64 {
    match m {
        Metric::Homoglyph => s.hogl,
        Metric::Skeleton => s.skel,
    }
}

/// Sort results ascending by the active metric (most suspicious first) and
/// optionally keep only the first `top`. Stable: ties preserve input order.
fn sort_and_truncate(
    mut results: Vec<(String, String, Scores)>,
    metric: Metric,
    top: Option<usize>,
) -> Vec<(String, String, Scores)> {
    results.sort_by(|x, y| {
        metric_value(&x.2, metric)
            .partial_cmp(&metric_value(&y.2, metric))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(n) = top {
        results.truncate(n);
    }
    results
}

/// Score `string` against one raw candidate line. Returns the scored row
/// (with the trimmed candidate as `match`), or `None` if the line is blank or
/// its `metric` distance exceeds `threshold`. Shared by the streaming and
/// buffered (sort/top) list-mode paths so both apply identical trim/skip/filter
/// rules.
fn score_candidate(
    string: &str,
    raw: &str,
    hogl_weight: f64,
    metric: Metric,
    threshold: Option<f64>,
) -> Option<(String, String, Scores)> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    let s = score_pair(string, line, hogl_weight);
    if let Some(t) = threshold {
        if metric_value(&s, metric) > t {
            return None;
        }
    }
    Some((string.to_string(), line.to_string(), s))
}

/// Score `string` against each line, collecting the kept rows and sorting /
/// truncating them. Used only when ranking is requested (`--sort`/`--top`);
/// the unsorted path streams via `score_candidate` without buffering.
fn process_list<I: Iterator<Item = String>>(
    string: &str,
    lines: I,
    hogl_weight: f64,
    metric: Metric,
    threshold: Option<f64>,
    sort: bool,
    top: Option<usize>,
) -> Vec<(String, String, Scores)> {
    let mut results: Vec<(String, String, Scores)> = lines
        .filter_map(|raw| score_candidate(string, &raw, hogl_weight, metric, threshold))
        .collect();
    if sort || top.is_some() {
        results = sort_and_truncate(results, metric, top);
    }
    results
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Levenshtein,
    Damerau,
    HomoglyphDamerau,
    SkeletonDamerau,
    Normalized,
    SkeletonNormalized,
    ConfusableOnly,
}

impl Field {
    /// Canonical emission order (matches the Scores struct / output contract).
    const ALL: [Field; 7] = [
        Field::Levenshtein,
        Field::Damerau,
        Field::HomoglyphDamerau,
        Field::SkeletonDamerau,
        Field::Normalized,
        Field::SkeletonNormalized,
        Field::ConfusableOnly,
    ];

    /// The JSON key / human-row label for this field.
    fn name(self) -> &'static str {
        match self {
            Field::Levenshtein => "levenshtein",
            Field::Damerau => "damerau",
            Field::HomoglyphDamerau => "homoglyph_damerau",
            Field::SkeletonDamerau => "skeleton_damerau",
            Field::Normalized => "normalized",
            Field::SkeletonNormalized => "skeleton_normalized",
            Field::ConfusableOnly => "confusable_only",
        }
    }

    fn from_name(s: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.name() == s)
    }

    /// This field's value formatted for JSON / human output.
    fn value_string(self, s: &Scores) -> String {
        match self {
            Field::Levenshtein => s.lev.to_string(),
            Field::Damerau => s.dam.to_string(),
            Field::HomoglyphDamerau => s.hogl.to_string(),
            Field::SkeletonDamerau => s.skel.to_string(),
            Field::Normalized => format!("{:.4}", s.norm),
            Field::SkeletonNormalized => format!("{:.4}", s.skel_norm),
            Field::ConfusableOnly => s.confusable_only.to_string(),
        }
    }
}

/// Parse a comma-separated field list into canonical-ordered, de-duplicated
/// Fields. Errors (naming the offender + valid names) on any unknown field.
#[allow(dead_code)] // TODO(task-5): remove once used
fn parse_fields(spec: &str) -> Result<Vec<Field>, String> {
    let mut seen = [false; Field::ALL.len()];
    for raw in spec.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        match Field::from_name(name) {
            Some(f) => {
                let idx = Field::ALL.iter().position(|&x| x == f).unwrap();
                seen[idx] = true;
            }
            None => {
                let valid: Vec<&str> = Field::ALL.iter().map(|f| f.name()).collect();
                return Err(format!(
                    "unknown field: {name} (valid: {})",
                    valid.join(", ")
                ));
            }
        }
    }
    Ok(Field::ALL
        .into_iter()
        .enumerate()
        .filter(|(i, _)| seen[*i])
        .map(|(_, f)| f)
        .collect())
}

struct Scores {
    lev: u64,
    dam: u64,
    hogl: f64,      // per-char weighted Damerau (single-char confusables)
    skel: f64,      // Damerau on full skeletons (multi-char aware)
    norm: f64,      // hogl / max(len)
    skel_norm: f64, // skel / max(skeleton len)
    confusable_only: bool,
}

fn score_pair(a: &str, b: &str, hogl_weight: f64) -> Scores {
    let ca: Vec<char> = a.chars().collect();
    let cb: Vec<char> = b.chars().collect();
    let lev = levenshtein(&ca, &cb, false, 0.0);
    let dam = damerau(&ca, &cb, false, 0.0);
    let hogl = damerau(&ca, &cb, true, hogl_weight);

    let ska = skeleton(a);
    let skb = skeleton(b);
    let sva: Vec<char> = ska.chars().collect();
    let svb: Vec<char> = skb.chars().collect();
    let skel = damerau(&sva, &svb, false, 0.0);

    let maxlen = ca.len().max(cb.len()).max(1) as f64;
    let skel_maxlen = sva.len().max(svb.len()).max(1) as f64;
    let confusable_only = a != b && ska == skb;

    Scores {
        lev: lev as u64,
        dam: dam as u64,
        hogl,
        skel,
        norm: hogl / maxlen,
        skel_norm: skel / skel_maxlen,
        confusable_only,
    }
}

/// One JSONL record for a scored pair. `keys` names the two strings (e.g.
/// ("a","b") or ("input","match")). `fields` = None emits all score fields;
/// Some(list) emits only those (identifier keys are always included).
fn result_json(
    a: &str,
    b: &str,
    s: &Scores,
    keys: (&str, &str),
    fields: Option<&[Field]>,
) -> String {
    let mut out = format!("{{\"{}\":{:?},\"{}\":{:?}", keys.0, a, keys.1, b);
    for f in selected_fields(fields) {
        out.push_str(&format!(",\"{}\":{}", f.name(), f.value_string(s)));
    }
    out.push('}');
    out
}

/// The fields to emit, in canonical order: all when None, else the given slice
/// (already canonical-ordered by parse_fields).
fn selected_fields(fields: Option<&[Field]>) -> Vec<Field> {
    match fields {
        None => Field::ALL.to_vec(),
        Some(list) => list.to_vec(),
    }
}

fn emit(a: &str, b: &str, s: &Scores, json: bool, fields: Option<&[Field]>) {
    if json {
        println!("{}", result_json(a, b, s, ("a", "b"), fields));
    } else {
        // Pad labels to a fixed column so values align (longest label is
        // "skeleton_normalized" = 19 chars; pad to 20 then a space).
        for f in selected_fields(fields) {
            println!("{:<20} {}", f.name(), f.value_string(s));
        }
    }
}

struct Opts {
    hogl_weight: f64,
    json: bool,
    threshold: Option<f64>,
    stdin: bool,
    string: Option<String>,
    list: Option<String>,
    sort: bool,
    top: Option<usize>,
    metric: Metric,
    positionals: Vec<String>,
}

fn print_usage() {
    eprintln!(
        "sqdist - typosquat / homoglyph string distance\n\n\
         USAGE:\n\
         \x20   sqdist [OPTIONS] <STRING_A> <STRING_B>      # single pair\n\
         \x20   sqdist [OPTIONS] --stdin                    # batch: pre-paired lines\n\
         \x20   sqdist [OPTIONS] --string <S> --list <FILE> # score <S> vs each line\n\n\
         OPTIONS:\n\
         \x20   -w, --hogl-weight <F>   Cost of a homoglyph substitution (default 0.1)\n\
         \x20   -t, --threshold <F>     Alert (emit / exit 0) when the --metric distance <= F\n\
         \x20   -m, --metric <M>        Distance for -t and --sort: homoglyph|skeleton (default skeleton)\n\
         \x20   -s, --stdin             Batch: read TAB/comma pairs from stdin, emit JSONL\n\
         \x20       --string <S>        (with --list) the single string to compare\n\
         \x20       --list <FILE>       (with --string) score <S> against each non-blank line\n\
         \x20       --sort              List mode: emit most-suspicious-first (buffers)\n\
         \x20       --top <N>           List mode: keep only the N closest (implies --sort)\n\
         \x20   -j, --json              Emit JSON (single-pair mode)\n\
         \x20   -h, --help              This help\n\n\
         OUTPUT FIELDS: levenshtein, damerau, homoglyph_damerau, skeleton_damerau,\n\
         \x20             normalized, skeleton_normalized, confusable_only\n\
         \x20  single-pair/stdin keys: a,b   |   list-mode keys: input,match\n"
    );
}

fn parse_from(argv: Vec<String>) -> Result<Opts, String> {
    let mut args = argv.into_iter();
    let mut opts = Opts {
        hogl_weight: 0.1,
        json: false,
        threshold: None,
        stdin: false,
        string: None,
        list: None,
        sort: false,
        top: None,
        metric: Metric::Skeleton,
        positionals: Vec::new(),
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-j" | "--json" => opts.json = true,
            "-s" | "--stdin" => opts.stdin = true,
            "--sort" => opts.sort = true,
            "--string" => {
                opts.string = Some(args.next().ok_or("--string needs a value")?);
            }
            "--list" => {
                opts.list = Some(args.next().ok_or("--list needs a value")?);
            }
            "--top" => {
                let v = args.next().ok_or("--top needs a value")?;
                let n: usize = v.parse().map_err(|_| "invalid --top")?;
                if n == 0 {
                    return Err("--top must be a positive integer".into());
                }
                opts.top = Some(n);
                opts.sort = true;
            }
            "-m" | "--metric" => {
                let v = args.next().ok_or("--metric needs a value")?;
                opts.metric = match v.as_str() {
                    "homoglyph" => Metric::Homoglyph,
                    "skeleton" => Metric::Skeleton,
                    other => {
                        return Err(format!(
                            "invalid --metric: {other} (use homoglyph|skeleton)"
                        ))
                    }
                };
            }
            "-w" | "--hogl-weight" => {
                let v = args.next().ok_or("--hogl-weight needs a value")?;
                opts.hogl_weight = v.parse().map_err(|_| "invalid --hogl-weight")?;
            }
            "-t" | "--threshold" => {
                let v = args.next().ok_or("--threshold needs a value")?;
                opts.threshold = Some(v.parse().map_err(|_| "invalid --threshold")?);
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(format!("unknown option: {s}"));
            }
            _ => opts.positionals.push(a),
        }
    }

    // Mode resolution: exactly one of {single-pair positionals, --stdin, --list}.
    let list_mode = opts.list.is_some() || opts.string.is_some();
    if list_mode {
        if opts.list.is_none() || opts.string.is_none() {
            return Err("--list and --string must be used together".into());
        }
        if opts.stdin {
            return Err("--list cannot be combined with --stdin".into());
        }
        if !opts.positionals.is_empty() {
            return Err("--list mode takes no positional arguments".into());
        }
        return Ok(opts);
    }
    if opts.stdin {
        if !opts.positionals.is_empty() {
            return Err("--stdin takes no positional arguments".into());
        }
        return Ok(opts);
    }
    if opts.positionals.len() != 2 {
        return Err(format!(
            "expected 2 string arguments, got {}",
            opts.positionals.len()
        ));
    }
    Ok(opts)
}

fn parse_args() -> Result<Opts, String> {
    parse_from(env::args().skip(1).collect())
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };

    use std::io::{self, BufRead, Write};

    // List mode: score --string against each line of --list, emit input/match JSONL.
    if let (Some(string), Some(path)) = (opts.string.as_ref(), opts.list.as_ref()) {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("error: cannot open --list file {path:?}: {e}");
                return ExitCode::from(2);
            }
        };
        let lines = io::BufReader::new(file).lines().map_while(Result::ok);
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        if opts.sort || opts.top.is_some() {
            // Ranking requires all rows up front: buffer, sort/truncate, emit.
            let results = process_list(
                string,
                lines,
                opts.hogl_weight,
                opts.metric,
                opts.threshold,
                opts.sort,
                opts.top,
            );
            for (a, b, s) in &results {
                let _ = writeln!(out, "{}", result_json(a, b, s, ("input", "match"), None));
            }
        } else {
            // No ranking: stream each kept row straight out, no buffering.
            for raw in lines {
                if let Some((a, b, s)) =
                    score_candidate(string, &raw, opts.hogl_weight, opts.metric, opts.threshold)
                {
                    let _ = writeln!(out, "{}", result_json(&a, &b, &s, ("input", "match"), None));
                }
            }
        }
        return ExitCode::SUCCESS;
    }

    // Stdin batch mode: pre-paired lines, a/b JSONL.
    if opts.stdin {
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
            let mut parts = line.splitn(2, ['\t', ',']);
            let (la, lb) = match (parts.next(), parts.next()) {
                (Some(x), Some(y)) => (x, y),
                _ => {
                    let _ = writeln!(out, "{{\"error\":\"malformed line\",\"line\":{line:?}}}");
                    continue;
                }
            };
            let s = score_pair(la, lb, opts.hogl_weight);
            if let Some(t) = opts.threshold {
                if metric_value(&s, opts.metric) > t {
                    continue;
                }
            }
            let _ = writeln!(out, "{}", result_json(la, lb, &s, ("a", "b"), None));
        }
        return ExitCode::SUCCESS;
    }

    // Single-pair mode.
    let a = &opts.positionals[0];
    let b = &opts.positionals[1];
    let s = score_pair(a, b, opts.hogl_weight);
    emit(a, b, &s, opts.json, None);
    if let Some(t) = opts.threshold {
        return if metric_value(&s, opts.metric) <= t {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
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
        assert!(
            (h - 0.2).abs() < 1e-9,
            "two homoglyphs should be 0.2, got {h}"
        );
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
        // Cyrillic а -> same skeleton as Latin paypal.
        assert_eq!(skeleton("p\u{0430}ypal"), skeleton("paypal"));
        // Unmapped chars pass through unchanged.
        assert_eq!(skeleton("xyz"), "xyz");
        // Empty string.
        assert_eq!(skeleton(""), "");
    }

    #[test]
    fn skeleton_damerau_catches_multichar_spoof() {
        let s = score_pair("rnicrosoft", "microsoft", 0.1);
        // Per-char metric can't align "rn" to "m", so it costs real edits.
        assert!(
            s.hogl > 1.0,
            "homoglyph_damerau should be > 1, got {}",
            s.hogl
        );
        // Skeleton metric sees identical skeletons => zero.
        assert!(
            s.skel.abs() < 1e-9,
            "skeleton_damerau should be ~0, got {}",
            s.skel
        );
        assert!(s.confusable_only, "should be confusable_only");
    }

    #[test]
    fn confusable_only_spans_unequal_lengths() {
        // "rnicrosoft" (10) vs "microsoft" (9): different lengths, same skeleton.
        let s = score_pair("rnicrosoft", "microsoft", 0.1);
        assert!(s.confusable_only);
        // Genuinely different strings are not confusable_only.
        let t = score_pair("google", "gogle", 0.1);
        assert!(!t.confusable_only);
    }

    #[test]
    fn skeleton_normalized_guards_zero() {
        // Two empty strings: a == b so confusable_only is false; norm fields 0.
        let s = score_pair("", "", 0.1);
        assert_eq!(s.skel_norm, 0.0);
        assert!(!s.confusable_only);
    }

    #[test]
    fn metric_value_selects_field() {
        let s = score_pair("rnicrosoft", "microsoft", 0.1);
        assert_eq!(metric_value(&s, Metric::Homoglyph), s.hogl);
        assert_eq!(metric_value(&s, Metric::Skeleton), s.skel);
    }

    #[test]
    fn sort_and_truncate_orders_and_caps() {
        // Build results with known skeleton distances by pairing against "abc".
        let pairs = vec![
            (
                "abc".to_string(),
                "abXYZ".to_string(),
                score_pair("abc", "abXYZ", 0.1),
            ),
            (
                "abc".to_string(),
                "abc".to_string(),
                score_pair("abc", "abc", 0.1),
            ),
            (
                "abc".to_string(),
                "abd".to_string(),
                score_pair("abc", "abd", 0.1),
            ),
        ];
        let sorted = sort_and_truncate(pairs, Metric::Skeleton, None);
        // Ascending by skeleton distance: identical (0) first.
        assert_eq!(sorted[0].1, "abc");
        assert!(
            metric_value(&sorted[0].2, Metric::Skeleton)
                <= metric_value(&sorted[1].2, Metric::Skeleton)
        );
        assert!(
            metric_value(&sorted[1].2, Metric::Skeleton)
                <= metric_value(&sorted[2].2, Metric::Skeleton)
        );

        // --top caps the output length.
        let pairs2 = vec![
            (
                "abc".to_string(),
                "abd".to_string(),
                score_pair("abc", "abd", 0.1),
            ),
            (
                "abc".to_string(),
                "abc".to_string(),
                score_pair("abc", "abc", 0.1),
            ),
        ];
        let top1 = sort_and_truncate(pairs2, Metric::Skeleton, Some(1));
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].1, "abc"); // closest kept
    }

    #[test]
    fn result_json_uses_given_keys_and_all_fields() {
        let s = score_pair("paypal", "p\u{0430}ypal", 0.1);
        let line = result_json("paypal", "p\u{0430}ypal", &s, ("a", "b"), None);
        assert!(line.starts_with("{\"a\":\"paypal\""));
        assert!(line.contains("\"homoglyph_damerau\":"));
        assert!(line.contains("\"skeleton_damerau\":"));
        assert!(line.contains("\"normalized\":"));
        assert!(line.contains("\"skeleton_normalized\":"));
        assert!(line.contains("\"confusable_only\":true"));

        // File-mode keys.
        let line2 = result_json("paypal", "p\u{0430}ypal", &s, ("input", "match"), None);
        assert!(line2.starts_with("{\"input\":\"paypal\",\"match\":"));
    }

    #[test]
    fn sort_is_stable_on_ties() {
        // Equal scores must preserve input order.
        let pairs = vec![
            (
                "x".to_string(),
                "first".to_string(),
                score_pair("x", "first", 0.1),
            ),
            (
                "x".to_string(),
                "secnd".to_string(),
                score_pair("x", "secnd", 0.1),
            ),
        ];
        // Both 5-char non-confusable => same skeleton distance.
        let a = metric_value(&pairs[0].2, Metric::Skeleton);
        let b = metric_value(&pairs[1].2, Metric::Skeleton);
        assert!((a - b).abs() < 1e-9, "precondition: scores must tie");
        let sorted = sort_and_truncate(pairs, Metric::Skeleton, None);
        assert_eq!(sorted[0].1, "first");
        assert_eq!(sorted[1].1, "secnd");
    }

    #[test]
    fn parse_list_mode() {
        let o = parse_from(vec![
            "--string".into(),
            "paypal".into(),
            "--list".into(),
            "names.txt".into(),
            "--metric".into(),
            "skeleton".into(),
        ])
        .unwrap();
        assert_eq!(o.string.as_deref(), Some("paypal"));
        assert_eq!(o.list.as_deref(), Some("names.txt"));
        assert_eq!(o.metric, Metric::Skeleton);
        assert!(o.positionals.is_empty());
    }

    #[test]
    fn parse_top_implies_sort() {
        let o = parse_from(vec![
            "--string".into(),
            "x".into(),
            "--list".into(),
            "f".into(),
            "--top".into(),
            "5".into(),
        ])
        .unwrap();
        assert_eq!(o.top, Some(5));
        assert!(o.sort);
    }

    #[test]
    fn parse_rejects_mode_conflicts() {
        // positionals + --list
        assert!(parse_from(vec!["a".into(), "b".into(), "--list".into(), "f".into(),]).is_err());
        // --stdin + --list
        assert!(parse_from(vec!["--stdin".into(), "--list".into(), "f".into(),]).is_err());
        // --list without --string
        assert!(parse_from(vec!["--list".into(), "f".into()]).is_err());
        // --string without --list
        assert!(parse_from(vec!["--string".into(), "x".into()]).is_err());
        // bad metric
        assert!(parse_from(vec![
            "--string".into(),
            "x".into(),
            "--list".into(),
            "f".into(),
            "--metric".into(),
            "bogus".into(),
        ])
        .is_err());
    }

    #[test]
    fn parse_single_pair_still_works() {
        let o = parse_from(vec!["paypal".into(), "p\u{0430}ypal".into()]).unwrap();
        assert_eq!(o.positionals.len(), 2);
        assert!(o.list.is_none());
        assert!(!o.stdin);
        assert_eq!(o.metric, Metric::Skeleton); // default
    }

    #[test]
    fn process_list_filters_sorts_caps() {
        let lines = vec![
            "paypal".to_string(),        // identical -> skel 0
            "p\u{0430}ypal".to_string(), // homoglyph -> skel 0, confusable_only
            "completely-different".to_string(),
        ];
        // No threshold, sort by skeleton, top 2: the two zero-distance lines.
        let out = process_list(
            "paypal",
            lines.clone().into_iter(),
            0.1,
            Metric::Skeleton,
            None,
            true,
            Some(2),
        );
        assert_eq!(out.len(), 2);
        assert!(metric_value(&out[0].2, Metric::Skeleton).abs() < 1e-9);
        assert!(metric_value(&out[1].2, Metric::Skeleton).abs() < 1e-9);

        // Threshold filters: only skeleton distance <= 0.0 kept (the 2 matches).
        let out2: Vec<_> = process_list(
            "paypal",
            lines.into_iter(),
            0.1,
            Metric::Skeleton,
            Some(0.0),
            false,
            None,
        );
        assert_eq!(out2.len(), 2);

        // Blank lines are skipped.
        let out3 = process_list(
            "paypal",
            vec!["".to_string(), "  ".to_string(), "paypal".to_string()].into_iter(),
            0.1,
            Metric::Skeleton,
            None,
            false,
            None,
        );
        assert_eq!(out3.len(), 1);
    }

    #[test]
    fn score_candidate_trims_skips_and_thresholds() {
        // Blank / whitespace-only -> None.
        assert!(score_candidate("paypal", "", 0.1, Metric::Skeleton, None).is_none());
        assert!(score_candidate("paypal", "   ", 0.1, Metric::Skeleton, None).is_none());

        // A match is returned with the trimmed candidate; input is the string arg.
        let r = score_candidate("paypal", "  p\u{0430}ypal  ", 0.1, Metric::Skeleton, None)
            .expect("should score");
        assert_eq!(r.0, "paypal");
        assert_eq!(r.1, "p\u{0430}ypal"); // trimmed
        assert!(r.2.confusable_only);

        // Threshold against the chosen metric drops over-threshold rows.
        // skeleton distance of an unrelated name > 0.0 => dropped.
        assert!(score_candidate("paypal", "zzzzzz", 0.1, Metric::Skeleton, Some(0.0)).is_none());
        // skeleton distance 0 (homoglyph) <= 0.0 => kept.
        assert!(
            score_candidate("paypal", "p\u{0430}ypal", 0.1, Metric::Skeleton, Some(0.0)).is_some()
        );
    }

    #[test]
    fn git_sha_env_is_present() {
        // build.rs always sets this (real SHA or "unknown").
        let sha = env!("SQDIST_GIT_SHA");
        assert!(!sha.is_empty());
    }

    #[test]
    fn parse_fields_valid_and_canonical_order() {
        // User order is ignored; canonical order is enforced.
        let f = parse_fields("confusable_only,damerau").unwrap();
        assert_eq!(f, vec![Field::Damerau, Field::ConfusableOnly]);
        // All names parse.
        let all = parse_fields(
            "levenshtein,damerau,homoglyph_damerau,skeleton_damerau,normalized,skeleton_normalized,confusable_only",
        )
        .unwrap();
        assert_eq!(all.len(), 7);
    }

    #[test]
    fn parse_fields_rejects_unknown() {
        let e = parse_fields("damerau,bogus").unwrap_err();
        assert!(e.contains("bogus"), "error should name the bad field: {e}");
    }

    #[test]
    fn parse_fields_dedups() {
        // Repeated names collapse to one, still canonical order.
        let f = parse_fields("damerau,damerau,levenshtein").unwrap();
        assert_eq!(f, vec![Field::Levenshtein, Field::Damerau]);
    }

    #[test]
    fn result_json_respects_field_filter() {
        let s = score_pair("GOOGLE", "GO0GLE", 0.1);
        // Filtered: only the two requested fields, plus identifier keys.
        let only = vec![Field::Damerau, Field::ConfusableOnly];
        let line = result_json("GOOGLE", "GO0GLE", &s, ("a", "b"), Some(&only));
        assert!(line.starts_with("{\"a\":\"GOOGLE\",\"b\":\"GO0GLE\""));
        assert!(line.contains("\"damerau\":"));
        assert!(line.contains("\"confusable_only\":"));
        assert!(!line.contains("\"levenshtein\":"));
        assert!(!line.contains("\"skeleton_damerau\":"));
        // None = all fields (back-compat).
        let full = result_json("GOOGLE", "GO0GLE", &s, ("a", "b"), None);
        assert!(full.contains("\"levenshtein\":"));
        assert!(full.contains("\"skeleton_normalized\":"));
    }
}
