//! sqdist — string distance for typosquatting / homoglyph detection.
//!
//! Computes a panel of independent similarity axes (Levenshtein, Damerau,
//! their UTS#39-skeleton variants, and confusable-involvement signals) between
//! two strings. See src/axes.rs for the axis registry and src/verdict.rs for
//! the single-pair human verdict.

mod axes;
mod confusables;
mod confusables_data;
mod digraph_data;
mod distance;
mod flowcrypt_data;
mod keyboard;
mod normalize;
mod verdict;

use axes::{
    build_panel, metric_value, parse_fields, validate_metric, PairContext, Panel, ALL_AXES,
};
use std::env;
use std::process::ExitCode;
use verdict::{classify_typosquat, verdict, TyposquatClass};

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// A scored row carried through batch/list modes: originals, optional norms, panel.
struct Row {
    a: String,
    b: String,
    a_norm: Option<String>,
    b_norm: Option<String>,
    panel: Panel,
}

/// Compute the panel for a pair.
fn score_pair(a: &str, b: &str, cmap: &confusables::ConfusableMap) -> Panel {
    build_panel(&PairContext::new(a, b, cmap))
}

struct PairScore {
    a: String,
    b: String,
    a_norm: Option<String>,
    b_norm: Option<String>,
    panel: Panel,
    scored_a: String,
    scored_b: String,
    identical: bool,
    same_project: bool,
}

fn build_ops(steps: &[NormStep]) -> Result<Vec<normalize::NormOp>, String> {
    let mut ops = Vec::new();
    for step in steps {
        match step {
            NormStep::Pypi => ops.extend(normalize::pypi_ops()),
            NormStep::File(path) => ops.extend(normalize::load_ops_file(path)?),
        }
    }
    Ok(ops)
}

fn prepare_pair(
    a: &str,
    b: &str,
    ops: &[normalize::NormOp],
    cmap: &confusables::ConfusableMap,
) -> PairScore {
    let on = !ops.is_empty();
    let a_norm = on.then(|| normalize::apply_ops(a, ops));
    let b_norm = on.then(|| normalize::apply_ops(b, ops));
    let identical = a == b;
    let same_project = on && !identical && a_norm.as_deref() == b_norm.as_deref();
    let (sa, sb): (&str, &str) = if on && !identical && !same_project {
        (a_norm.as_deref().unwrap(), b_norm.as_deref().unwrap())
    } else {
        (a, b)
    };
    let panel = score_pair(sa, sb, cmap);
    let scored_a = sa.to_string();
    let scored_b = sb.to_string();
    PairScore {
        a: a.to_string(),
        b: b.to_string(),
        a_norm,
        b_norm,
        panel,
        scored_a,
        scored_b,
        identical,
        same_project,
    }
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
    norms: Option<(&str, &str)>,
    class: Option<(&str, &str)>, // (json_key, reason)
) -> String {
    let mut out = format!("{{\"{}\":{:?},\"{}\":{:?}", keys.0, a, keys.1, b);
    if let Some((na, nb)) = norms {
        out.push_str(&format!(
            ",\"{}_normalized\":{:?},\"{}_normalized\":{:?}",
            keys.0, na, keys.1, nb
        ));
    }
    for k in selected_keys(fields) {
        if let Some(v) = panel.get(k) {
            out.push_str(&format!(",\"{}\":{}", k, v.to_json()));
        }
    }
    if let Some((ck, reason)) = class {
        out.push_str(&format!(
            ",\"classification\":\"{ck}\",\"reason\":{reason:?}"
        ));
    }
    out.push('}');
    out
}

/// Human single-pair output: optional normalized line, one padded row per
/// selected axis, then a blank line and the verdict / classification.
fn emit_human(
    scored: &PairScore,
    fields: Option<&[&'static str]>,
    len_tolerance: f64,
    typosquat: bool,
    ops_on: bool,
) {
    if ops_on {
        if let (Some(na), Some(nb)) = (scored.a_norm.as_deref(), scored.b_norm.as_deref()) {
            if na != scored.a || nb != scored.b {
                println!("{: <24} {na} / {nb}", "normalized");
            }
        }
    }
    for k in selected_keys(fields) {
        if let Some(v) = scored.panel.get(k) {
            println!("{k:<24} {}", v.to_human());
        }
    }
    if typosquat {
        let (cat, msg) = classify_typosquat(
            scored.identical,
            scored.same_project,
            &scored.panel,
            &scored.scored_a,
            &scored.scored_b,
        );
        println!("\n[{}] {msg}", cat.tag());
    } else if scored.same_project {
        println!(
            "\n[SAME PROJECT] The names differ only by registry normalization (same project)."
        );
    } else {
        let la = scored.a.chars().count();
        let lb = scored.b.chars().count();
        let (cat, msg) = verdict(&scored.panel, la, lb, len_tolerance);
        println!("\n[{}] {msg}", cat.tag());
    }
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
        row_metric(&x.panel, metric)
            .partial_cmp(&row_metric(&y.panel, metric))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(n) = top {
        rows.truncate(n);
    }
    rows
}

/// Keep a prepared pair as a batch row, or drop it under `--typosquat` /
/// `-t` filters.
fn keep_scored(s: PairScore, metric: &str, threshold: Option<f64>, typosquat: bool) -> Option<Row> {
    if typosquat {
        let (cat, _) = classify_typosquat(
            s.identical,
            s.same_project,
            &s.panel,
            &s.scored_a,
            &s.scored_b,
        );
        if !matches!(
            cat,
            TyposquatClass::LikelyTyposquat | TyposquatClass::PossibleCombosquat
        ) {
            return None;
        }
    } else if let Some(t) = threshold {
        if row_metric(&s.panel, metric) > t {
            return None;
        }
    }
    Some(Row {
        a: s.a,
        b: s.b,
        a_norm: s.a_norm,
        b_norm: s.b_norm,
        panel: s.panel,
    })
}

/// Score `string` against one raw candidate line. Returns the scored row, or
/// None if blank, not a likely typosquat (when filtering), or over threshold.
fn score_candidate(
    string: &str,
    raw: &str,
    metric: &str,
    threshold: Option<f64>,
    cmap: &confusables::ConfusableMap,
    ops: &[normalize::NormOp],
    typosquat: bool,
) -> Option<Row> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    keep_scored(
        prepare_pair(string, line, ops, cmap),
        metric,
        threshold,
        typosquat,
    )
}

/// Score `string` against each line, collecting kept rows, sorting/truncating
/// when ranking is requested.
#[allow(clippy::too_many_arguments)] // threads ops/typosquat; tests call this directly
fn process_list<I: Iterator<Item = String>>(
    string: &str,
    lines: I,
    metric: &str,
    threshold: Option<f64>,
    sort: bool,
    top: Option<usize>,
    cmap: &confusables::ConfusableMap,
    ops: &[normalize::NormOp],
    typosquat: bool,
) -> Vec<Row> {
    let mut rows: Vec<Row> = lines
        .filter_map(|raw| score_candidate(string, &raw, metric, threshold, cmap, ops, typosquat))
        .collect();
    if sort || top.is_some() {
        rows = sort_and_truncate(rows, metric, top);
    }
    rows
}

/// Batch-mode success: `--typosquat` or `-t` requires at least one match;
/// otherwise always succeeds.
fn batch_matched_ok(threshold: Option<f64>, typosquat: bool, matched: bool) -> bool {
    if typosquat || threshold.is_some() {
        matched
    } else {
        true
    }
}

/// Reconstruct scored lengths / identity-gate flags from a kept row.
fn class_from_row(row: &Row, ops_on: bool) -> (TyposquatClass, String) {
    let identical = row.a == row.b;
    let same_project = ops_on && !identical && row.a_norm.as_deref() == row.b_norm.as_deref();
    let (sa, sb): (&str, &str) = if ops_on && !identical && !same_project {
        (
            row.a_norm.as_deref().unwrap(),
            row.b_norm.as_deref().unwrap(),
        )
    } else {
        (row.a.as_str(), row.b.as_str())
    };
    classify_typosquat(identical, same_project, &row.panel, sa, sb)
}

/// JSONL line for a batch/list row. Norms when ops ran; class when `--typosquat`.
fn emit_row_json(
    row: &Row,
    keys: (&str, &str),
    fields: Option<&[&'static str]>,
    ops: &[normalize::NormOp],
    typosquat: bool,
) -> String {
    let norms = if !ops.is_empty() {
        match (row.a_norm.as_deref(), row.b_norm.as_deref()) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        }
    } else {
        None
    };
    let class_owned: Option<(&'static str, String)> = if typosquat {
        let (c, reason) = class_from_row(row, !ops.is_empty());
        Some((c.json_key(), reason))
    } else {
        None
    };
    let class = class_owned.as_ref().map(|(k, r)| (*k, r.as_str()));
    result_json(&row.a, &row.b, &row.panel, keys, fields, norms, class)
}

#[derive(Debug, PartialEq)]
enum NormStep {
    Pypi,
    File(String),
}

#[derive(Debug)]
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
    sources: confusables::Sources,
    typosquat: bool,
    norm_steps: Vec<NormStep>,
    metric_explicit: bool,
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
         \x20       --confusables <LIST> Confusable sources for skeletons: uts39,flowcrypt,digraph (default uts39)\n\
         \x20       --len-tolerance <F> Max length-difference ratio for a spoof verdict (default 0.25)\n\
         \x20   --typosquat             Package-typosquat profile: five axes, classification\n\
         \x20                           (incl. possible_combosquat). Batch emits alerts only.\n\
         \x20                           Default metric damerau. Cannot combine with -t.\n\
         \x20   --pypi                  PEP 503 normalize (lower, map ._- → -, collapse -).\n\
         \x20                           Identity-gate same-project names; otherwise score\n\
         \x20                           normalized strings. Originals stay identifier keys.\n\
         \x20-n, --normalize <PATH>      Append normalize ops from a JSON file (ordered\n\
         \x20                           array of [op, ...]). Repeatable. Not a preset name.\n\
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
         \x20   confusable_only         true when the strings differ but share an identical skeleton\n\
         \x20   script_restriction      UTS#39 restriction level 0-5 (higher = more mixed-script/suspicious)\n\
         \x20   keyboard_distance       mean QWERTY key distance over substitutions, 0-1 (n/a if non-ASCII)\n\n\
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
        sources: confusables::Sources::default(),
        typosquat: false,
        norm_steps: Vec::new(),
        metric_explicit: false,
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
                println!("  data: {}", confusables_data::CONFUSABLES_PROVENANCE);
                println!("  data: {}", flowcrypt_data::FLOWCRYPT_PROVENANCE);
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
                opts.metric_explicit = true;
            }
            "--typosquat" => opts.typosquat = true,
            "--pypi" => {
                if opts.norm_steps.iter().any(|s| matches!(s, NormStep::Pypi)) {
                    return Err("duplicate --pypi".into());
                }
                opts.norm_steps.push(NormStep::Pypi);
            }
            "-n" | "--normalize" => {
                let v = args.next().ok_or("--normalize needs a path")?;
                opts.norm_steps.push(NormStep::File(v));
            }
            "--fields" => {
                let v = args.next().ok_or("--fields needs a value")?;
                opts.fields = Some(parse_fields(&v)?);
            }
            "--confusables" => {
                let v = args.next().ok_or("--confusables needs a value")?;
                opts.sources = confusables::parse_sources(&v)?;
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

    if opts.typosquat {
        if opts.threshold.is_some() {
            return Err("--typosquat cannot be combined with -t/--threshold".into());
        }
        if !opts.metric_explicit {
            opts.metric = "damerau";
        }
        if opts.fields.is_none() {
            opts.fields = Some(
                parse_fields("equal,damerau,skeleton_damerau,confusable_only,keyboard_distance")
                    .expect("typosquat field list is valid"),
            );
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
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    let opts = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };

    use std::io::{self, BufRead, Write};

    let cmap = if opts.sources == confusables::Sources::default() {
        confusables::ConfusableMap::uts39()
    } else {
        confusables::ConfusableMap::from_sources(&opts.sources)
    };

    let ops = match build_ops(&opts.norm_steps) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };

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
                &cmap,
                &ops,
                opts.typosquat,
            );
            matched = !rows.is_empty();
            for row in &rows {
                let _ = writeln!(
                    out,
                    "{}",
                    emit_row_json(
                        row,
                        ("input", "match"),
                        opts.fields.as_deref(),
                        &ops,
                        opts.typosquat,
                    )
                );
            }
        } else {
            for raw in lines {
                if let Some(row) = score_candidate(
                    string,
                    &raw,
                    opts.metric,
                    opts.threshold,
                    &cmap,
                    &ops,
                    opts.typosquat,
                ) {
                    matched = true;
                    let _ = writeln!(
                        out,
                        "{}",
                        emit_row_json(
                            &row,
                            ("input", "match"),
                            opts.fields.as_deref(),
                            &ops,
                            opts.typosquat,
                        )
                    );
                }
            }
        }
        return if batch_matched_ok(opts.threshold, opts.typosquat, matched) {
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
            let s = prepare_pair(la, lb, &ops, &cmap);
            if let Some(row) = keep_scored(s, opts.metric, opts.threshold, opts.typosquat) {
                matched = true;
                let _ = writeln!(
                    out,
                    "{}",
                    emit_row_json(
                        &row,
                        ("a", "b"),
                        opts.fields.as_deref(),
                        &ops,
                        opts.typosquat,
                    )
                );
            }
        }
        return if batch_matched_ok(opts.threshold, opts.typosquat, matched) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    // Single-pair mode.
    let a = &opts.positionals[0];
    let b = &opts.positionals[1];
    let scored = prepare_pair(a, b, &ops, &cmap);
    let norms = match (scored.a_norm.as_deref(), scored.b_norm.as_deref()) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    let class_owned: Option<(&'static str, String)> = if opts.typosquat {
        let (c, reason) = classify_typosquat(
            scored.identical,
            scored.same_project,
            &scored.panel,
            &scored.scored_a,
            &scored.scored_b,
        );
        Some((c.json_key(), reason))
    } else {
        None
    };
    let class = class_owned.as_ref().map(|(k, r)| (*k, r.as_str()));
    if opts.json {
        println!(
            "{}",
            result_json(
                &scored.a,
                &scored.b,
                &scored.panel,
                ("a", "b"),
                opts.fields.as_deref(),
                norms,
                class,
            )
        );
    } else {
        emit_human(
            &scored,
            opts.fields.as_deref(),
            opts.len_tolerance,
            opts.typosquat,
            !ops.is_empty(),
        );
    }
    if let Some(t) = opts.threshold {
        return if row_metric(&scored.panel, opts.metric) <= t {
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

    fn cmap() -> confusables::ConfusableMap {
        confusables::ConfusableMap::uts39()
    }

    fn test_row(a: &str, b: &str) -> Row {
        Row {
            a: a.to_string(),
            b: b.to_string(),
            a_norm: None,
            b_norm: None,
            panel: score_pair(a, b, &cmap()),
        }
    }

    #[test]
    fn result_json_uses_given_keys_and_all_axes() {
        let panel = score_pair("paypal", "p\u{0430}ypal", &cmap());
        let line = result_json(
            "paypal",
            "p\u{0430}ypal",
            &panel,
            ("a", "b"),
            None,
            None,
            None,
        );
        assert!(line.starts_with("{\"a\":\"paypal\""));
        assert!(line.contains("\"damerau\":"));
        assert!(line.contains("\"skeleton_damerau\":"));
        assert!(line.contains("\"uts39_confusable_count\":"));
        assert!(line.contains("\"uts39_skeleton_delta\":"));
        assert!(line.contains("\"confusable_only\":true"));
        assert!(!line.contains("homoglyph_damerau"));
        assert!(!line.contains("normalized"));

        let line2 = result_json(
            "paypal",
            "p\u{0430}ypal",
            &panel,
            ("input", "match"),
            None,
            None,
            None,
        );
        assert!(line2.starts_with("{\"input\":\"paypal\",\"match\":"));
    }

    #[test]
    fn result_json_respects_field_filter() {
        let panel = score_pair("GOOGLE", "GO0GLE", &cmap());
        let only = parse_fields("damerau,confusable_only").unwrap();
        let line = result_json(
            "GOOGLE",
            "GO0GLE",
            &panel,
            ("a", "b"),
            Some(&only),
            None,
            None,
        );
        assert!(line.starts_with("{\"a\":\"GOOGLE\",\"b\":\"GO0GLE\""));
        assert!(line.contains("\"damerau\":"));
        assert!(line.contains("\"confusable_only\":"));
        assert!(!line.contains("\"levenshtein\":"));
        assert!(!line.contains("\"skeleton_damerau\":"));
    }

    #[test]
    fn sort_and_truncate_orders_and_caps() {
        let rows = vec![
            test_row("abc", "abXYZ"),
            test_row("abc", "abc"),
            test_row("abc", "abd"),
        ];
        let sorted = sort_and_truncate(rows, "skeleton_damerau", None);
        assert_eq!(sorted[0].b, "abc"); // identical -> 0 first
        assert!(
            row_metric(&sorted[0].panel, "skeleton_damerau")
                <= row_metric(&sorted[1].panel, "skeleton_damerau")
        );
        assert!(
            row_metric(&sorted[1].panel, "skeleton_damerau")
                <= row_metric(&sorted[2].panel, "skeleton_damerau")
        );

        let rows2 = vec![test_row("abc", "abd"), test_row("abc", "abc")];
        let top1 = sort_and_truncate(rows2, "skeleton_damerau", Some(1));
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].b, "abc");
    }

    #[test]
    fn sort_is_stable_on_ties() {
        let rows = vec![test_row("x", "first"), test_row("x", "secnd")];
        let a = row_metric(&rows[0].panel, "skeleton_damerau");
        let b = row_metric(&rows[1].panel, "skeleton_damerau");
        assert!((a - b).abs() < 1e-9, "precondition: scores must tie");
        let sorted = sort_and_truncate(rows, "skeleton_damerau", None);
        assert_eq!(sorted[0].b, "first");
        assert_eq!(sorted[1].b, "secnd");
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
            &cmap(),
            &[],
            false,
        );
        assert_eq!(out.len(), 2);
        assert!(row_metric(&out[0].panel, "skeleton_damerau").abs() < 1e-9);
        assert!(row_metric(&out[1].panel, "skeleton_damerau").abs() < 1e-9);

        let out2 = process_list(
            "paypal",
            lines.into_iter(),
            "skeleton_damerau",
            Some(0.0),
            false,
            None,
            &cmap(),
            &[],
            false,
        );
        assert_eq!(out2.len(), 2);

        let out3 = process_list(
            "paypal",
            vec!["".to_string(), "  ".to_string(), "paypal".to_string()].into_iter(),
            "skeleton_damerau",
            None,
            false,
            None,
            &cmap(),
            &[],
            false,
        );
        assert_eq!(out3.len(), 1);
    }

    #[test]
    fn score_candidate_trims_skips_and_thresholds() {
        assert!(
            score_candidate("paypal", "", "skeleton_damerau", None, &cmap(), &[], false).is_none()
        );
        assert!(score_candidate(
            "paypal",
            "   ",
            "skeleton_damerau",
            None,
            &cmap(),
            &[],
            false
        )
        .is_none());

        let r = score_candidate(
            "paypal",
            "  p\u{0430}ypal  ",
            "skeleton_damerau",
            None,
            &cmap(),
            &[],
            false,
        )
        .expect("should score");
        assert_eq!(r.a, "paypal");
        assert_eq!(r.b, "p\u{0430}ypal");
        assert_eq!(r.panel.get("confusable_only"), Some(AxisValue::Bool(true)));

        assert!(score_candidate(
            "paypal",
            "zzzzzz",
            "skeleton_damerau",
            Some(0.0),
            &cmap(),
            &[],
            false
        )
        .is_none());
        assert!(score_candidate(
            "paypal",
            "p\u{0430}ypal",
            "skeleton_damerau",
            Some(0.0),
            &cmap(),
            &[],
            false
        )
        .is_some());
    }

    #[test]
    fn batch_exit_codes() {
        assert!(batch_matched_ok(None, false, false));
        assert!(batch_matched_ok(None, false, true));
        assert!(batch_matched_ok(Some(0.5), false, true));
        assert!(!batch_matched_ok(Some(0.5), false, false));
    }

    #[test]
    fn score_candidate_typosquat_keeps_lodahs() {
        let row = score_candidate("lodash", "lodahs", "damerau", None, &cmap(), &[], true);
        assert!(row.is_some());
    }

    #[test]
    fn score_candidate_typosquat_keeps_combosquat() {
        let row = score_candidate(
            "lodash",
            "lodash-utils",
            "damerau",
            None,
            &cmap(),
            &[],
            true,
        );
        assert!(row.is_some());
    }

    #[test]
    fn score_candidate_typosquat_drops_unrelated() {
        let row = score_candidate("lodash", "xylophone", "damerau", None, &cmap(), &[], true);
        assert!(row.is_none());
    }

    #[test]
    fn score_candidate_typosquat_pypi_drops_same_project() {
        let row = score_candidate(
            "foo_bar",
            "foo-bar",
            "damerau",
            None,
            &cmap(),
            &normalize::pypi_ops(),
            true,
        );
        assert!(row.is_none());
    }

    #[test]
    fn score_candidate_pypi_without_typosquat_keeps_same_project() {
        let row = score_candidate(
            "foo_bar",
            "foo-bar",
            "skeleton_damerau",
            None,
            &cmap(),
            &normalize::pypi_ops(),
            false,
        );
        assert!(row.is_some());
    }

    #[test]
    fn batch_ok_typosquat_requires_match() {
        assert!(!batch_matched_ok(None, true, false));
        assert!(batch_matched_ok(None, true, true));
        assert!(batch_matched_ok(None, false, false));
        assert!(!batch_matched_ok(Some(1.0), false, false));
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
    fn na_metric_value_is_infinity() {
        // keyboard_distance is NA for a non-ASCII pair; row_metric falls back to
        // +inf so the row never matches a finite -t and sorts last.
        let panel = score_pair("paypal", "p\u{0430}ypal", &cmap());
        assert!(row_metric(&panel, "keyboard_distance").is_infinite());
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

    #[test]
    fn version_includes_confusables_provenance() {
        let p = confusables_data::CONFUSABLES_PROVENANCE;
        assert!(p.contains("UTS#39"), "provenance names the standard: {p}");
        assert!(p.contains("17.0.0"), "provenance names the version: {p}");
    }

    #[test]
    fn version_includes_flowcrypt_provenance() {
        let p = flowcrypt_data::FLOWCRYPT_PROVENANCE;
        assert!(p.contains("FlowCrypt"), "names the source: {p}");
        assert!(p.contains("retrieved"), "carries a retrieval date: {p}");
        assert!(
            !p.contains("unknown"),
            "commit must be resolved, not 'unknown': {p}"
        );
    }

    #[test]
    fn parse_confusables_flag() {
        let o = parse_from(vec![
            "--confusables".into(),
            "uts39,digraph".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
        assert!(o.sources.digraph && !o.sources.flowcrypt);
    }

    #[test]
    fn parse_confusables_default_is_uts39() {
        let o = parse_from(vec!["a".into(), "b".into()]).unwrap();
        assert_eq!(o.sources, confusables::Sources::default());
    }

    #[test]
    fn parse_confusables_rejects_unknown() {
        assert!(parse_from(vec![
            "--confusables".into(),
            "uts39,nope".into(),
            "a".into(),
            "b".into(),
        ])
        .is_err());
    }

    #[test]
    fn parse_typosquat_defaults_metric_and_fields() {
        let o = parse_from(vec!["--typosquat".into(), "a".into(), "b".into()]).unwrap();
        assert!(o.typosquat);
        assert_eq!(o.metric, "damerau");
        assert_eq!(
            o.fields.as_deref(),
            Some(
                &[
                    "equal",
                    "damerau",
                    "skeleton_damerau",
                    "confusable_only",
                    "keyboard_distance"
                ][..]
            )
        );
    }

    #[test]
    fn parse_typosquat_metric_override_kept() {
        let o = parse_from(vec![
            "--typosquat".into(),
            "-m".into(),
            "skeleton_damerau".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
        assert_eq!(o.metric, "skeleton_damerau");
    }

    #[test]
    fn parse_typosquat_rejects_threshold() {
        let e = parse_from(vec![
            "--typosquat".into(),
            "-t".into(),
            "1".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap_err();
        assert!(e.contains("--typosquat"), "{e}");
        assert!(e.contains("-t"), "{e}");
    }

    #[test]
    fn parse_pypi_and_normalize_order() {
        let o = parse_from(vec![
            "--pypi".into(),
            "-n".into(),
            "extra.json".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
        assert_eq!(o.norm_steps.len(), 2);
        assert!(matches!(o.norm_steps[0], NormStep::Pypi));
        match &o.norm_steps[1] {
            NormStep::File(p) => assert_eq!(p, "extra.json"),
            other => panic!("{other:?}"),
        }

        let o2 = parse_from(vec![
            "--normalize".into(),
            "extra.json".into(),
            "--pypi".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
        match &o2.norm_steps[0] {
            NormStep::File(p) => assert_eq!(p, "extra.json"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(o2.norm_steps[1], NormStep::Pypi));
    }

    #[test]
    fn parse_normalize_pypi_is_a_filename() {
        let o = parse_from(vec!["-n".into(), "pypi".into(), "a".into(), "b".into()]).unwrap();
        match &o.norm_steps[..] {
            [NormStep::File(p)] => assert_eq!(p, "pypi"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_duplicate_pypi_errors() {
        assert!(parse_from(vec![
            "--pypi".into(),
            "--pypi".into(),
            "a".into(),
            "b".into()
        ])
        .is_err());
    }

    #[test]
    fn parse_fields_overrides_typosquat_emit_set() {
        let o = parse_from(vec![
            "--typosquat".into(),
            "--fields".into(),
            "damerau".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
        assert_eq!(o.fields.as_deref(), Some(&["damerau"][..]));
    }

    #[test]
    fn result_json_pypi_adds_normalized_keys() {
        let panel = score_pair("foo-bar", "foo-bar", &cmap());
        let line = result_json(
            "foo_bar",
            "foo-bar",
            &panel,
            ("a", "b"),
            None,
            Some(("foo-bar", "foo-bar")),
            None,
        );
        assert!(
            line.contains("\"a_normalized\":\"foo-bar\"") || line.contains("\"a_normalized\":")
        );
        assert!(line.contains("a_normalized"));
        assert!(line.contains("b_normalized"));
        assert!(!line.contains("\"classification\""));
    }

    #[test]
    fn result_json_typosquat_adds_classification_after_axes() {
        let panel = score_pair("lodash", "lodahs", &cmap());
        let fields =
            parse_fields("equal,damerau,skeleton_damerau,confusable_only,keyboard_distance")
                .unwrap();
        let line = result_json(
            "lodash",
            "lodahs",
            &panel,
            ("a", "b"),
            Some(&fields),
            None,
            Some(("likely_typosquat", "1 Damerau edit.")),
        );
        assert!(line.contains("\"classification\":\"likely_typosquat\""));
        assert!(line.contains("\"reason\":"));
        assert!(!line.contains("\"levenshtein\":"));
        let class_at = line.find("\"classification\"").unwrap();
        let dam_at = line.find("\"damerau\"").unwrap();
        assert!(dam_at < class_at, "classification after axes");
    }

    #[test]
    fn result_json_list_normalized_key_names() {
        let panel = score_pair("a", "a", &cmap());
        let line = result_json(
            "A",
            "a",
            &panel,
            ("input", "match"),
            None,
            Some(("a", "a")),
            None,
        );
        assert!(line.contains("input_normalized"));
        assert!(line.contains("match_normalized"));
    }

    #[test]
    fn prepare_pair_same_project_scores_originals() {
        let ops = normalize::pypi_ops();
        let s = prepare_pair("foo_bar", "foo-bar", &ops, &cmap());
        assert!(s.same_project);
        assert!(!s.identical);
        assert_eq!(s.a, "foo_bar");
        assert_eq!(s.a_norm.as_deref(), Some("foo-bar"));
        // Panel on originals → damerau 1, equal false
        assert_eq!(s.panel.get("equal"), Some(axes::AxisValue::Bool(false)));
        assert_eq!(s.panel.get("damerau"), Some(axes::AxisValue::Int(1)));
    }

    #[test]
    fn prepare_pair_django_case_scores_normalized() {
        let ops = normalize::pypi_ops();
        let s = prepare_pair("Django", "djangoo", &ops, &cmap());
        assert!(!s.same_project);
        assert_eq!(s.scored_a, "django");
        assert_eq!(s.scored_b, "djangoo");
        assert_eq!(s.panel.get("damerau"), Some(axes::AxisValue::Int(1)));
    }
}
