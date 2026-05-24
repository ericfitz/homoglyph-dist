//! sqdist — string distance for typosquatting / homoglyph detection.
//!
//! Computes a panel of independent similarity axes (Levenshtein, Damerau,
//! their UTS#39-skeleton variants, and confusable-involvement signals) between
//! two strings. See src/axes.rs for the axis registry and src/verdict.rs for
//! the single-pair human verdict.

mod axes;
mod confusables_data;
mod distance;
mod verdict;

use axes::{
    build_panel, metric_value, parse_fields, validate_metric, PairContext, Panel, ALL_AXES,
};
use std::env;
use std::process::ExitCode;
use verdict::verdict;

/// A scored row carried through batch/list modes: the two strings + the panel.
type Row = (String, String, Panel);

/// Compute the panel for a pair.
fn score_pair(a: &str, b: &str) -> Panel {
    build_panel(&PairContext::new(a, b))
}

/// The selected axis keys to emit, in canonical order: all when None.
fn selected_keys(fields: Option<&[&'static str]>) -> Vec<&'static str> {
    match fields {
        None => ALL_AXES.iter().map(|ax| ax.key()).collect(),
        Some(list) => list.to_vec(),
    }
}

/// One JSONL record for a scored pair. `keys` names the two strings.
fn result_json(
    a: &str,
    b: &str,
    panel: &Panel,
    keys: (&str, &str),
    fields: Option<&[&'static str]>,
) -> String {
    let mut out = format!("{{\"{}\":{:?},\"{}\":{:?}", keys.0, a, keys.1, b);
    for k in selected_keys(fields) {
        if let Some(v) = panel.get(k) {
            out.push_str(&format!(",\"{}\":{}", k, v.to_json()));
        }
    }
    out.push('}');
    out
}

/// Human single-pair output: one padded row per selected axis, then a blank
/// line, then the verdict.
fn emit_human(
    a: &str,
    b: &str,
    panel: &Panel,
    fields: Option<&[&'static str]>,
    len_tolerance: f64,
) {
    for k in selected_keys(fields) {
        if let Some(v) = panel.get(k) {
            // Longest key is uts39_confusable_count (22); pad to 24.
            println!("{k:<24} {}", v.to_human());
        }
    }
    let la = a.chars().count();
    let lb = b.chars().count();
    let (cat, msg) = verdict(panel, la, lb, len_tolerance);
    println!("\n[{}] {}", cat.tag(), msg);
}

/// The metric distance of a row, or +inf if the metric key is missing/non-numeric
/// (cannot happen — the key is validated at parse time).
fn row_metric(panel: &Panel, metric: &str) -> f64 {
    metric_value(panel, metric).unwrap_or(f64::INFINITY)
}

/// Sort rows ascending by the active metric (most suspicious first) and
/// optionally keep only the first `top`. Stable: ties preserve input order.
fn sort_and_truncate(mut rows: Vec<Row>, metric: &str, top: Option<usize>) -> Vec<Row> {
    rows.sort_by(|x, y| {
        row_metric(&x.2, metric)
            .partial_cmp(&row_metric(&y.2, metric))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(n) = top {
        rows.truncate(n);
    }
    rows
}

/// Score `string` against one raw candidate line. Returns the scored row, or
/// None if blank or over threshold on the active metric.
fn score_candidate(string: &str, raw: &str, metric: &str, threshold: Option<f64>) -> Option<Row> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    let panel = score_pair(string, line);
    if let Some(t) = threshold {
        if row_metric(&panel, metric) > t {
            return None;
        }
    }
    Some((string.to_string(), line.to_string(), panel))
}

/// Score `string` against each line, collecting kept rows, sorting/truncating
/// when ranking is requested.
fn process_list<I: Iterator<Item = String>>(
    string: &str,
    lines: I,
    metric: &str,
    threshold: Option<f64>,
    sort: bool,
    top: Option<usize>,
) -> Vec<Row> {
    let mut rows: Vec<Row> = lines
        .filter_map(|raw| score_candidate(string, &raw, metric, threshold))
        .collect();
    if sort || top.is_some() {
        rows = sort_and_truncate(rows, metric, top);
    }
    rows
}

/// Batch-mode success: with a threshold, requires at least one match; without,
/// always succeeds.
fn batch_matched_ok(threshold: Option<f64>, matched: bool) -> bool {
    threshold.is_none() || matched
}

struct Opts {
    json: bool,
    threshold: Option<f64>,
    stdin: bool,
    string: Option<String>,
    list: Option<String>,
    sort: bool,
    top: Option<usize>,
    metric: &'static str,
    positionals: Vec<String>,
    fields: Option<Vec<&'static str>>,
    len_tolerance: f64,
}

fn print_usage() {
    eprintln!(
        "sqdist - typosquat / homoglyph string distance\n\n\
         USAGE:\n\
         \x20   sqdist [OPTIONS] <STRING_A> <STRING_B>      # single pair\n\
         \x20   sqdist [OPTIONS] --stdin                    # batch: pre-paired lines\n\
         \x20   sqdist [OPTIONS] --string <S> --list <FILE> # score <S> vs each line\n\n\
         OPTIONS:\n\
         \x20   -t, --threshold <F>     Alert when the --metric distance <= F. Single-pair:\n\
         \x20                           sets exit code. Batch: filters output; exit 1 if none match.\n\
         \x20   -m, --metric <AXIS>     Numeric axis for -t and --sort (default skeleton_damerau)\n\
         \x20       --fields <LIST>     Comma-separated axes to show (default: all). See AXES.\n\
         \x20       --len-tolerance <F> Max length-difference ratio for a spoof verdict (default 0.25)\n\
         \x20   -s, --stdin             Batch: read TAB/comma pairs from stdin, emit JSONL\n\
         \x20       --string <S>        (with --list) the single string to compare\n\
         \x20       --list <FILE>       (with --string) score <S> against each non-blank line\n\
         \x20       --sort              List mode: emit most-suspicious-first (buffers)\n\
         \x20       --top <N>           List mode: keep only the N closest (implies --sort)\n\
         \x20   -j, --json              Emit JSON (single-pair mode)\n\
         \x20   -v, --version           Print version and commit, then exit\n\
         \x20   -h, --help              This help\n\n\
         AXES:\n\
         \x20   equal                   the strings are byte-identical (bool)\n\
         \x20   levenshtein             min single-char insert/delete/substitute edits\n\
         \x20   damerau                 like levenshtein, but an adjacent swap counts as one edit\n\
         \x20   skeleton_levenshtein    levenshtein after reducing both to UTS#39 skeletons\n\
         \x20   skeleton_damerau        damerau after reducing both to UTS#39 skeletons\n\
         \x20                           (~0 when visually identical, incl. multi-char confusables)\n\
         \x20   uts39_confusable_count  # of aligned substitutions that are UTS#39-confusable (experimental)\n\
         \x20   uts39_skeleton_delta    damerau - skeleton_damerau; edits that vanish under\n\
         \x20                           skeletonization (experimental, may change)\n\
         \x20   confusable_only         true when the strings differ but share an identical skeleton\n\n\
         OUTPUT KEYS: single-pair/stdin use a,b; list mode uses input,match. Batch is JSONL.\n"
    );
}

fn parse_from(argv: Vec<String>) -> Result<Opts, String> {
    let mut args = argv.into_iter();
    let mut opts = Opts {
        json: false,
        threshold: None,
        stdin: false,
        string: None,
        list: None,
        sort: false,
        top: None,
        metric: "skeleton_damerau",
        positionals: Vec::new(),
        fields: None,
        len_tolerance: 0.25,
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-v" | "--version" => {
                println!(
                    "sqdist {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    env!("SQDIST_GIT_SHA")
                );
                std::process::exit(0);
            }
            "-j" | "--json" => opts.json = true,
            "-s" | "--stdin" => opts.stdin = true,
            "--sort" => opts.sort = true,
            "--string" => opts.string = Some(args.next().ok_or("--string needs a value")?),
            "--list" => opts.list = Some(args.next().ok_or("--list needs a value")?),
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
                opts.metric = validate_metric(&v)?;
            }
            "--fields" => {
                let v = args.next().ok_or("--fields needs a value")?;
                opts.fields = Some(parse_fields(&v)?);
            }
            "--len-tolerance" => {
                let v = args.next().ok_or("--len-tolerance needs a value")?;
                let t: f64 = v.parse().map_err(|_| "invalid --len-tolerance")?;
                if !(0.0..=1.0).contains(&t) {
                    return Err("--len-tolerance must be between 0.0 and 1.0".into());
                }
                opts.len_tolerance = t;
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
        let mut matched = false;
        if opts.sort || opts.top.is_some() {
            let rows = process_list(
                string,
                lines,
                opts.metric,
                opts.threshold,
                opts.sort,
                opts.top,
            );
            matched = !rows.is_empty();
            for (a, b, panel) in &rows {
                let _ = writeln!(
                    out,
                    "{}",
                    result_json(a, b, panel, ("input", "match"), opts.fields.as_deref())
                );
            }
        } else {
            for raw in lines {
                if let Some((a, b, panel)) =
                    score_candidate(string, &raw, opts.metric, opts.threshold)
                {
                    matched = true;
                    let _ = writeln!(
                        out,
                        "{}",
                        result_json(&a, &b, &panel, ("input", "match"), opts.fields.as_deref())
                    );
                }
            }
        }
        return if batch_matched_ok(opts.threshold, matched) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Stdin batch mode: pre-paired lines, a/b JSONL.
    if opts.stdin {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        let mut matched = false;
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
            let panel = score_pair(la, lb);
            if let Some(t) = opts.threshold {
                if row_metric(&panel, opts.metric) > t {
                    continue;
                }
            }
            matched = true;
            let _ = writeln!(
                out,
                "{}",
                result_json(la, lb, &panel, ("a", "b"), opts.fields.as_deref())
            );
        }
        return if batch_matched_ok(opts.threshold, matched) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Single-pair mode.
    let a = &opts.positionals[0];
    let b = &opts.positionals[1];
    let panel = score_pair(a, b);
    if opts.json {
        println!(
            "{}",
            result_json(a, b, &panel, ("a", "b"), opts.fields.as_deref())
        );
    } else {
        emit_human(a, b, &panel, opts.fields.as_deref(), opts.len_tolerance);
    }
    if let Some(t) = opts.threshold {
        return if row_metric(&panel, opts.metric) <= t {
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
    use axes::AxisValue;

    #[test]
    fn result_json_uses_given_keys_and_all_axes() {
        let panel = score_pair("paypal", "p\u{0430}ypal");
        let line = result_json("paypal", "p\u{0430}ypal", &panel, ("a", "b"), None);
        assert!(line.starts_with("{\"a\":\"paypal\""));
        assert!(line.contains("\"damerau\":"));
        assert!(line.contains("\"skeleton_damerau\":"));
        assert!(line.contains("\"uts39_confusable_count\":"));
        assert!(line.contains("\"uts39_skeleton_delta\":"));
        assert!(line.contains("\"confusable_only\":true"));
        assert!(!line.contains("homoglyph_damerau"));
        assert!(!line.contains("normalized"));

        let line2 = result_json("paypal", "p\u{0430}ypal", &panel, ("input", "match"), None);
        assert!(line2.starts_with("{\"input\":\"paypal\",\"match\":"));
    }

    #[test]
    fn result_json_respects_field_filter() {
        let panel = score_pair("GOOGLE", "GO0GLE");
        let only = parse_fields("damerau,confusable_only").unwrap();
        let line = result_json("GOOGLE", "GO0GLE", &panel, ("a", "b"), Some(&only));
        assert!(line.starts_with("{\"a\":\"GOOGLE\",\"b\":\"GO0GLE\""));
        assert!(line.contains("\"damerau\":"));
        assert!(line.contains("\"confusable_only\":"));
        assert!(!line.contains("\"levenshtein\":"));
        assert!(!line.contains("\"skeleton_damerau\":"));
    }

    #[test]
    fn sort_and_truncate_orders_and_caps() {
        let rows = vec![
            (
                "abc".to_string(),
                "abXYZ".to_string(),
                score_pair("abc", "abXYZ"),
            ),
            (
                "abc".to_string(),
                "abc".to_string(),
                score_pair("abc", "abc"),
            ),
            (
                "abc".to_string(),
                "abd".to_string(),
                score_pair("abc", "abd"),
            ),
        ];
        let sorted = sort_and_truncate(rows, "skeleton_damerau", None);
        assert_eq!(sorted[0].1, "abc"); // identical -> 0 first
        assert!(
            row_metric(&sorted[0].2, "skeleton_damerau")
                <= row_metric(&sorted[1].2, "skeleton_damerau")
        );
        assert!(
            row_metric(&sorted[1].2, "skeleton_damerau")
                <= row_metric(&sorted[2].2, "skeleton_damerau")
        );

        let rows2 = vec![
            (
                "abc".to_string(),
                "abd".to_string(),
                score_pair("abc", "abd"),
            ),
            (
                "abc".to_string(),
                "abc".to_string(),
                score_pair("abc", "abc"),
            ),
        ];
        let top1 = sort_and_truncate(rows2, "skeleton_damerau", Some(1));
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].1, "abc");
    }

    #[test]
    fn sort_is_stable_on_ties() {
        let rows = vec![
            (
                "x".to_string(),
                "first".to_string(),
                score_pair("x", "first"),
            ),
            (
                "x".to_string(),
                "secnd".to_string(),
                score_pair("x", "secnd"),
            ),
        ];
        let a = row_metric(&rows[0].2, "skeleton_damerau");
        let b = row_metric(&rows[1].2, "skeleton_damerau");
        assert!((a - b).abs() < 1e-9, "precondition: scores must tie");
        let sorted = sort_and_truncate(rows, "skeleton_damerau", None);
        assert_eq!(sorted[0].1, "first");
        assert_eq!(sorted[1].1, "secnd");
    }

    #[test]
    fn process_list_filters_sorts_caps() {
        let lines = vec![
            "paypal".to_string(),
            "p\u{0430}ypal".to_string(),
            "completely-different".to_string(),
        ];
        let out = process_list(
            "paypal",
            lines.clone().into_iter(),
            "skeleton_damerau",
            None,
            true,
            Some(2),
        );
        assert_eq!(out.len(), 2);
        assert!(row_metric(&out[0].2, "skeleton_damerau").abs() < 1e-9);
        assert!(row_metric(&out[1].2, "skeleton_damerau").abs() < 1e-9);

        let out2 = process_list(
            "paypal",
            lines.into_iter(),
            "skeleton_damerau",
            Some(0.0),
            false,
            None,
        );
        assert_eq!(out2.len(), 2);

        let out3 = process_list(
            "paypal",
            vec!["".to_string(), "  ".to_string(), "paypal".to_string()].into_iter(),
            "skeleton_damerau",
            None,
            false,
            None,
        );
        assert_eq!(out3.len(), 1);
    }

    #[test]
    fn score_candidate_trims_skips_and_thresholds() {
        assert!(score_candidate("paypal", "", "skeleton_damerau", None).is_none());
        assert!(score_candidate("paypal", "   ", "skeleton_damerau", None).is_none());

        let r = score_candidate("paypal", "  p\u{0430}ypal  ", "skeleton_damerau", None)
            .expect("should score");
        assert_eq!(r.0, "paypal");
        assert_eq!(r.1, "p\u{0430}ypal");
        assert_eq!(r.2.get("confusable_only"), Some(AxisValue::Bool(true)));

        assert!(score_candidate("paypal", "zzzzzz", "skeleton_damerau", Some(0.0)).is_none());
        assert!(
            score_candidate("paypal", "p\u{0430}ypal", "skeleton_damerau", Some(0.0)).is_some()
        );
    }

    #[test]
    fn batch_exit_codes() {
        assert!(batch_matched_ok(None, false));
        assert!(batch_matched_ok(None, true));
        assert!(batch_matched_ok(Some(0.5), true));
        assert!(!batch_matched_ok(Some(0.5), false));
    }

    #[test]
    fn git_sha_env_is_present() {
        assert!(!env!("SQDIST_GIT_SHA").is_empty());
    }

    #[test]
    fn parse_list_mode() {
        let o = parse_from(vec![
            "--string".into(),
            "paypal".into(),
            "--list".into(),
            "names.txt".into(),
            "--metric".into(),
            "skeleton_damerau".into(),
        ])
        .unwrap();
        assert_eq!(o.string.as_deref(), Some("paypal"));
        assert_eq!(o.list.as_deref(), Some("names.txt"));
        assert_eq!(o.metric, "skeleton_damerau");
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
        assert!(parse_from(vec!["a".into(), "b".into(), "--list".into(), "f".into()]).is_err());
        assert!(parse_from(vec!["--stdin".into(), "--list".into(), "f".into()]).is_err());
        assert!(parse_from(vec!["--list".into(), "f".into()]).is_err());
        assert!(parse_from(vec!["--string".into(), "x".into()]).is_err());
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
        assert_eq!(o.metric, "skeleton_damerau");
    }

    #[test]
    fn metric_rejects_bool_axis_flag() {
        assert!(parse_from(vec![
            "--metric".into(),
            "equal".into(),
            "a".into(),
            "b".into()
        ])
        .is_err());
        assert!(parse_from(vec![
            "--metric".into(),
            "confusable_only".into(),
            "a".into(),
            "b".into()
        ])
        .is_err());
    }

    #[test]
    fn hogl_weight_flag_removed() {
        assert!(parse_from(vec!["-w".into(), "0.1".into(), "a".into(), "b".into()]).is_err());
        assert!(parse_from(vec![
            "--hogl-weight".into(),
            "0.1".into(),
            "a".into(),
            "b".into()
        ])
        .is_err());
    }

    #[test]
    fn parse_fields_flag() {
        let o = parse_from(vec![
            "--fields".into(),
            "damerau,confusable_only".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
        assert_eq!(
            o.fields.as_deref(),
            Some(&["damerau", "confusable_only"][..])
        );
    }

    #[test]
    fn parse_fields_flag_rejects_unknown() {
        assert!(parse_from(vec![
            "--fields".into(),
            "damerau,nope".into(),
            "a".into(),
            "b".into(),
        ])
        .is_err());
    }

    #[test]
    fn parse_len_tolerance_default_and_override() {
        let d = parse_from(vec!["a".into(), "b".into()]).unwrap();
        assert!((d.len_tolerance - 0.25).abs() < 1e-9);
        let o = parse_from(vec![
            "--len-tolerance".into(),
            "0.4".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
        assert!((o.len_tolerance - 0.4).abs() < 1e-9);
    }

    #[test]
    fn parse_len_tolerance_rejects_out_of_range() {
        assert!(parse_from(vec![
            "--len-tolerance".into(),
            "5.0".into(),
            "a".into(),
            "b".into()
        ])
        .is_err());
        assert!(parse_from(vec![
            "--len-tolerance".into(),
            "-0.5".into(),
            "a".into(),
            "b".into()
        ])
        .is_err());
        assert!(parse_from(vec![
            "--len-tolerance".into(),
            "0.0".into(),
            "a".into(),
            "b".into()
        ])
        .is_ok());
        assert!(parse_from(vec![
            "--len-tolerance".into(),
            "1.0".into(),
            "a".into(),
            "b".into()
        ])
        .is_ok());
    }
}
