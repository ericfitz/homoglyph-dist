# Skeletonization + Watchlist Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add full UTS#39 skeletonization (catches multi-char confusables like `rn`↔`m`) and a `--string`/`--list` watchlist mode to `sqdist`, with `main()` refactored into thin I/O over pure, unit-tested functions.

**Architecture:** All code lives in `src/main.rs` (single-binary project, no new files). Pure functions (`skeleton`, extended `score_pair`, `metric_value`, `sort_and_truncate`, `result_json`) are unit-tested in the `#[cfg(test)]` module; `main()` only parses args, reads stdin/files, and writes. Skeleton-based metrics are added alongside the existing per-char homoglyph metrics, both emitted. The `-m/--metric` flag selects which distance drives `-t` and `--sort`.

**Tech Stack:** Rust 2021, std only (no new dependencies). `cargo test` / `cargo build --release`. Confusables data already embedded in `src/confusables_data.rs`.

**Reference spec:** `docs/superpowers/specs/2026-05-22-skeleton-and-watchlist-design.md`

**Conventions for every task:** run `cargo test` (all tests) and `cargo clippy` before each commit. Commit messages use Conventional Commits and end with the `Co-Authored-By` trailer the repo uses.

---

## File structure

| File | Responsibility | Change |
|---|---|---|
| `src/main.rs` | everything: metrics, skeleton, scoring, arg parsing, I/O modes, tests | modified throughout |
| `src/confusables_data.rs` | embedded UTS#39 table | unchanged (read-only) |
| `README.md` | user docs | rewrite limitation section, document new flags/fields |
| `CLAUDE.md` | agent docs | update output contract + mode matrix |

Task order builds bottom-up: skeleton primitive → scores struct → metric selection → sort/truncate → serializer → arg parsing → I/O wiring → docs. Each task is independently testable and committed.

---

## Task 1: `skeleton()` function

**Files:**
- Modify: `src/main.rs` (add function after `confusable`, ~line 31; add tests in the `#[cfg(test)]` module ~line 272)

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `src/main.rs`:

```rust
    #[test]
    fn skeleton_maps_multichar() {
        // UTS#39: the skeleton of 'm' is "rn", so "microsoft" and "rnicrosoft"
        // share a skeleton.
        assert_eq!(skeleton("microsoft"), skeleton("rnicrosoft"));
        assert_eq!(skeleton("microsoft"), "rnicrosoft");
    }

    #[test]
    fn skeleton_is_idempotent() {
        for s in ["microsoft", "paypal", "vvallet", "g\u{43E}\u{43E}gle", "abc123"] {
            assert_eq!(skeleton(&skeleton(s)), skeleton(s), "not idempotent for {s}");
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test skeleton 2>&1 | tail -20`
Expected: compile error — `cannot find function 'skeleton' in this scope`.

- [ ] **Step 3: Implement `skeleton()`**

Add immediately after the `confusable` function (after line 31) in `src/main.rs`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test skeleton 2>&1 | tail -20`
Expected: `skeleton_maps_multichar`, `skeleton_is_idempotent`, `skeleton_collapses_homoglyphs` all PASS.

Note: if `skeleton_is_idempotent` fails, it means some table entry's target re-skeletonizes to something different. The spec asserts this should not happen for the embedded v17.0.0 data; if it does, STOP and report — do not add a recursive loop without revisiting the spec.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: add UTS#39 skeleton() for multi-char confusable detection

skeleton(s) maps each code point through the confusables table and
concatenates, so the skeleton of 'm' is "rn" and "microsoft"/"rnicrosoft"
share a skeleton. Single non-recursive pass; idempotent on the v17.0.0 table.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Extend `Scores` with skeleton fields + redefine `confusable_only`

**Files:**
- Modify: `src/main.rs` — `Scores` struct (lines 104-110), `score_pair` (lines 112-129), existing `digit_letter_confusable`/`full_homoglyph_word_near_zero` tests if affected

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
    #[test]
    fn skeleton_damerau_catches_multichar_spoof() {
        let s = score_pair("rnicrosoft", "microsoft", 0.1);
        // Per-char metric can't align "rn" to "m", so it costs real edits.
        assert!(s.hogl > 1.0, "homoglyph_damerau should be > 1, got {}", s.hogl);
        // Skeleton metric sees identical skeletons => zero.
        assert!(s.skel.abs() < 1e-9, "skeleton_damerau should be ~0, got {}", s.skel);
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test 2>&1 | tail -25`
Expected: compile errors — no field `skel`, `skel_norm` on `Scores`.

- [ ] **Step 3: Extend `Scores` and `score_pair`**

Replace the `Scores` struct (lines 104-110) with:

```rust
struct Scores {
    lev: u64,
    dam: u64,
    hogl: f64,      // per-char weighted Damerau (single-char confusables)
    skel: f64,      // Damerau on full skeletons (multi-char aware)
    norm: f64,      // hogl / max(len)
    skel_norm: f64, // skel / max(skeleton len)
    confusable_only: bool,
}
```

Replace `score_pair` (lines 112-129) with:

```rust
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
```

- [ ] **Step 4: Check whether existing tests need updating**

Run: `cargo test 2>&1 | tail -30`
Expected: the three new tests PASS. The existing six tests in the module call `levenshtein`/`damerau`/`confusable` directly (not `score_pair`), EXCEPT none assert on `Scores` fields — verify by reading the test bodies. If any existing test fails due to the `confusable_only` redefinition, update its expectation to match `a != b && skeleton(a) == skeleton(b)` and note the change in the commit. Do NOT weaken a test to make it pass without confirming the new value is correct per the spec.

- [ ] **Step 5: Run full suite**

Run: `cargo test 2>&1 | tail -15`
Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: add skeleton_damerau/skeleton_normalized scores, redefine confusable_only

score_pair now also computes Damerau distance on full skeletons (multi-char
aware) and a skeleton-normalized score. confusable_only is redefined as
(a != b && skeleton(a) == skeleton(b)), dropping the equal-length requirement
so cross-length spoofs like rnicrosoft/microsoft are flagged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `Metric` enum + `metric_value()`

**Files:**
- Modify: `src/main.rs` — add `Metric` enum and `metric_value` near `score_pair`; tests in module

- [ ] **Step 1: Write the failing test**

Add to `mod tests`:

```rust
    #[test]
    fn metric_value_selects_field() {
        let s = score_pair("rnicrosoft", "microsoft", 0.1);
        assert_eq!(metric_value(&s, Metric::Homoglyph), s.hogl);
        assert_eq!(metric_value(&s, Metric::Skeleton), s.skel);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test metric_value 2>&1 | tail -15`
Expected: compile error — `cannot find type 'Metric'` / `function 'metric_value'`.

- [ ] **Step 3: Implement `Metric` and `metric_value`**

Add directly above the `Scores` struct (before line 104):

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test metric_value 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: add Metric enum and metric_value() for threshold/sort selection

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `sort_and_truncate()`

**Files:**
- Modify: `src/main.rs` — add function; tests in module

- [ ] **Step 1: Write the failing test**

Add to `mod tests`:

```rust
    #[test]
    fn sort_and_truncate_orders_and_caps() {
        // Build results with known skeleton distances by pairing against "abc".
        let pairs = vec![
            ("abc".to_string(), "abXYZ".to_string(), score_pair("abc", "abXYZ", 0.1)),
            ("abc".to_string(), "abc".to_string(), score_pair("abc", "abc", 0.1)),
            ("abc".to_string(), "abd".to_string(), score_pair("abc", "abd", 0.1)),
        ];
        let sorted = sort_and_truncate(pairs, Metric::Skeleton, None);
        // Ascending by skeleton distance: identical (0) first.
        assert_eq!(sorted[0].1, "abc");
        assert!(metric_value(&sorted[0].2, Metric::Skeleton)
            <= metric_value(&sorted[1].2, Metric::Skeleton));
        assert!(metric_value(&sorted[1].2, Metric::Skeleton)
            <= metric_value(&sorted[2].2, Metric::Skeleton));

        // --top caps the output length.
        let pairs2 = vec![
            ("abc".to_string(), "abd".to_string(), score_pair("abc", "abd", 0.1)),
            ("abc".to_string(), "abc".to_string(), score_pair("abc", "abc", 0.1)),
        ];
        let top1 = sort_and_truncate(pairs2, Metric::Skeleton, Some(1));
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].1, "abc"); // closest kept
    }

    #[test]
    fn sort_is_stable_on_ties() {
        // Equal scores must preserve input order.
        let pairs = vec![
            ("x".to_string(), "first".to_string(), score_pair("x", "first", 0.1)),
            ("x".to_string(), "secnd".to_string(), score_pair("x", "secnd", 0.1)),
        ];
        // Both 5-char non-confusable => same skeleton distance.
        let a = metric_value(&pairs[0].2, Metric::Skeleton);
        let b = metric_value(&pairs[1].2, Metric::Skeleton);
        assert!((a - b).abs() < 1e-9, "precondition: scores must tie");
        let sorted = sort_and_truncate(pairs, Metric::Skeleton, None);
        assert_eq!(sorted[0].1, "first");
        assert_eq!(sorted[1].1, "secnd");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test sort 2>&1 | tail -15`
Expected: compile error — `cannot find function 'sort_and_truncate'`.

- [ ] **Step 3: Implement `sort_and_truncate`**

Add after `metric_value`:

```rust
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
```

Note: `Vec::sort_by` is guaranteed stable, satisfying the tie requirement.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test sort 2>&1 | tail -10`
Expected: `sort_and_truncate_orders_and_caps` and `sort_is_stable_on_ties` PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: add sort_and_truncate() for watchlist --sort/--top

Stable ascending sort by the active metric with optional top-N truncation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `result_json()` shared serializer

**Files:**
- Modify: `src/main.rs` — add serializer, refactor `emit` to use it; tests in module

- [ ] **Step 1: Write the failing test**

Add to `mod tests`:

```rust
    #[test]
    fn result_json_uses_given_keys_and_all_fields() {
        let s = score_pair("paypal", "p\u{0430}ypal", 0.1);
        let line = result_json("paypal", "p\u{0430}ypal", &s, ("a", "b"));
        assert!(line.starts_with("{\"a\":\"paypal\""));
        assert!(line.contains("\"homoglyph_damerau\":"));
        assert!(line.contains("\"skeleton_damerau\":"));
        assert!(line.contains("\"normalized\":"));
        assert!(line.contains("\"skeleton_normalized\":"));
        assert!(line.contains("\"confusable_only\":true"));

        // File-mode keys.
        let line2 = result_json("paypal", "p\u{0430}ypal", &s, ("input", "match"));
        assert!(line2.starts_with("{\"input\":\"paypal\",\"match\":"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test result_json 2>&1 | tail -15`
Expected: compile error — `cannot find function 'result_json'`.

- [ ] **Step 3: Implement `result_json` and refactor `emit`**

Add this function (place it just above `emit`, ~line 131):

```rust
/// One JSONL record for a scored pair, using the given key names for the two
/// strings (e.g. ("a","b") for single-pair/stdin, ("input","match") for --list).
fn result_json(a: &str, b: &str, s: &Scores, keys: (&str, &str)) -> String {
    format!(
        "{{\"{}\":{:?},\"{}\":{:?},\"levenshtein\":{},\"damerau\":{},\
         \"homoglyph_damerau\":{},\"skeleton_damerau\":{},\
         \"normalized\":{:.4},\"skeleton_normalized\":{:.4},\"confusable_only\":{}}}",
        keys.0, a, keys.1, b, s.lev, s.dam, s.hogl, s.skel, s.norm, s.skel_norm, s.confusable_only
    )
}
```

Replace `emit` (lines 131-144) with a version that reuses `result_json` for the JSON branch and prints the new fields in the human branch:

```rust
fn emit(a: &str, b: &str, s: &Scores, json: bool) {
    if json {
        println!("{}", result_json(a, b, s, ("a", "b")));
    } else {
        println!("levenshtein          {}", s.lev);
        println!("damerau              {}", s.dam);
        println!("homoglyph_damerau    {}", s.hogl);
        println!("skeleton_damerau     {}", s.skel);
        println!("normalized           {:.4}", s.norm);
        println!("skeleton_normalized  {:.4}", s.skel_norm);
        println!("confusable_only      {}", s.confusable_only);
    }
}
```

- [ ] **Step 4: Run test + full suite**

Run: `cargo test 2>&1 | tail -15`
Expected: `result_json_uses_given_keys_and_all_fields` PASS; all other tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: add result_json() serializer, emit skeleton fields in all output

One serializer parameterized by key names ((a,b) or (input,match)) used by
single-pair, stdin, and list modes. Human output gains skeleton_damerau and
skeleton_normalized.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Extend `Opts` + arg parsing for `--string`/`--list`/`--sort`/`--top`/`--metric`

**Files:**
- Modify: `src/main.rs` — `Opts` struct (lines 146-151), `parse_args` (lines 169-212), `print_usage` (lines 153-167); tests in module

- [ ] **Step 1: Write the failing tests**

The current `parse_args` reads `env::args()` directly, which is untestable. This task introduces a testable `parse_from(args: Vec<String>)` and makes `parse_args` a thin wrapper. Add to `mod tests`:

```rust
    #[test]
    fn parse_list_mode() {
        let o = parse_from(vec![
            "--string".into(), "paypal".into(),
            "--list".into(), "names.txt".into(),
            "--metric".into(), "skeleton".into(),
        ]).unwrap();
        assert_eq!(o.string.as_deref(), Some("paypal"));
        assert_eq!(o.list.as_deref(), Some("names.txt"));
        assert_eq!(o.metric, Metric::Skeleton);
        assert!(o.positionals.is_empty());
    }

    #[test]
    fn parse_top_implies_sort() {
        let o = parse_from(vec![
            "--string".into(), "x".into(), "--list".into(), "f".into(),
            "--top".into(), "5".into(),
        ]).unwrap();
        assert_eq!(o.top, Some(5));
        assert!(o.sort);
    }

    #[test]
    fn parse_rejects_mode_conflicts() {
        // positionals + --list
        assert!(parse_from(vec![
            "a".into(), "b".into(), "--list".into(), "f".into(),
        ]).is_err());
        // --stdin + --list
        assert!(parse_from(vec![
            "--stdin".into(), "--list".into(), "f".into(),
        ]).is_err());
        // --list without --string
        assert!(parse_from(vec!["--list".into(), "f".into()]).is_err());
        // --string without --list
        assert!(parse_from(vec!["--string".into(), "x".into()]).is_err());
        // bad metric
        assert!(parse_from(vec![
            "--string".into(), "x".into(), "--list".into(), "f".into(),
            "--metric".into(), "bogus".into(),
        ]).is_err());
    }

    #[test]
    fn parse_single_pair_still_works() {
        let o = parse_from(vec!["paypal".into(), "p\u{0430}ypal".into()]).unwrap();
        assert_eq!(o.positionals.len(), 2);
        assert!(o.list.is_none());
        assert!(!o.stdin);
        assert_eq!(o.metric, Metric::Skeleton); // default
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test parse_ 2>&1 | tail -20`
Expected: compile errors — `Opts` has no `string`/`list`/`sort`/`top`/`metric`/`positionals`; no `parse_from`.

- [ ] **Step 3: Rewrite `Opts` and split `parse_args` into `parse_from`**

Replace the `Opts` struct (lines 146-151) with:

```rust
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
```

Replace `parse_args` (lines 169-212) with `parse_from` plus a thin `parse_args` wrapper:

```rust
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
                    other => return Err(format!("invalid --metric: {other} (use homoglyph|skeleton)")),
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
        return Err(format!("expected 2 string arguments, got {}", opts.positionals.len()));
    }
    Ok(opts)
}

fn parse_args() -> Result<Opts, String> {
    parse_from(env::args().skip(1).collect())
}
```

- [ ] **Step 4: Update `print_usage`**

Replace `print_usage` (lines 153-167) with:

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test parse_ 2>&1 | tail -20`
Expected: all four `parse_*` tests PASS.

Note: `main()` will not compile yet because it uses the old `parse_args` return shape `(String, String, Opts)`. That is fixed in Task 7. To keep this task's commit compiling, ALSO apply the minimal `main()` change in Step 6 below before committing.

- [ ] **Step 6: Adjust `main()` signature use to compile (full rewrite in Task 7)**

In `main()` (lines 214-222), change the destructuring to match the new `parse_args` return type. Replace:

```rust
    let (a, b, opts) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };
```

with:

```rust
    let opts = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };
    // Single-pair operands (list/stdin modes ignore these).
    let a = opts.positionals.first().cloned().unwrap_or_default();
    let b = opts.positionals.get(1).cloned().unwrap_or_default();
```

This keeps the existing stdin/single-pair bodies working unchanged for now (they reference `a`, `b`, `opts.*`). Run `cargo build 2>&1 | tail -15` — Expected: builds (warnings about unused `opts.string` etc. are fine).

- [ ] **Step 7: Run full suite + clippy**

Run: `cargo test 2>&1 | tail -15 && cargo clippy 2>&1 | tail -15`
Expected: all tests PASS; clippy clean (or only pre-existing warnings).

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: parse --string/--list/--sort/--top/--metric with mode validation

Split arg parsing into testable parse_from(Vec<String>); add list-mode flags,
the --metric selector (default skeleton), and mode-conflict validation. Update
usage text. main() adapted to the new Opts shape; list-mode wiring follows.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Wire list mode + metric-driven threshold into `main()`

**Files:**
- Modify: `src/main.rs` — `main()` (lines 214-270), add a `run_list` helper; tests in module for the helper

- [ ] **Step 1: Write the failing test for the list-processing helper**

This task adds `process_list(string, lines, hogl_weight, metric, threshold, sort, top) -> Vec<(String,String,Scores)>` — pure over an iterator of lines so it is unit-testable without files. Add to `mod tests`:

```rust
    #[test]
    fn process_list_filters_sorts_caps() {
        let lines = vec![
            "paypal".to_string(),     // identical -> skel 0
            "p\u{0430}ypal".to_string(), // homoglyph -> skel 0, confusable_only
            "completely-different".to_string(),
        ];
        // No threshold, sort by skeleton, top 2: the two zero-distance lines.
        let out = process_list("paypal", lines.clone().into_iter(), 0.1,
                               Metric::Skeleton, None, true, Some(2));
        assert_eq!(out.len(), 2);
        assert!(metric_value(&out[0].2, Metric::Skeleton).abs() < 1e-9);
        assert!(metric_value(&out[1].2, Metric::Skeleton).abs() < 1e-9);

        // Threshold filters: only skeleton distance <= 0.0 kept (the 2 matches).
        let out2: Vec<_> = process_list("paypal", lines.into_iter(), 0.1,
                                        Metric::Skeleton, Some(0.0), false, None);
        assert_eq!(out2.len(), 2);

        // Blank lines are skipped.
        let out3 = process_list("paypal",
            vec!["".to_string(), "  ".to_string(), "paypal".to_string()].into_iter(),
            0.1, Metric::Skeleton, None, false, None);
        assert_eq!(out3.len(), 1);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test process_list 2>&1 | tail -15`
Expected: compile error — `cannot find function 'process_list'`.

- [ ] **Step 3: Implement `process_list`**

Add after `sort_and_truncate`:

```rust
/// Score `string` against each non-blank line, applying threshold filtering
/// (against `metric`) while streaming-collecting, then optionally sort/truncate.
fn process_list<I: Iterator<Item = String>>(
    string: &str,
    lines: I,
    hogl_weight: f64,
    metric: Metric,
    threshold: Option<f64>,
    sort: bool,
    top: Option<usize>,
) -> Vec<(String, String, Scores)> {
    let mut results: Vec<(String, String, Scores)> = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let s = score_pair(string, line, hogl_weight);
        if let Some(t) = threshold {
            if metric_value(&s, metric) > t {
                continue;
            }
        }
        results.push((string.to_string(), line.to_string(), s));
    }
    if sort || top.is_some() {
        results = sort_and_truncate(results, metric, top);
    }
    results
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test process_list 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Rewrite `main()` to dispatch all three modes**

Replace the entire `main()` (lines 214-270) with:

```rust
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
        let lines = io::BufReader::new(file)
            .lines()
            .map_while(Result::ok);
        let results = process_list(
            string, lines, opts.hogl_weight, opts.metric,
            opts.threshold, opts.sort, opts.top,
        );
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        for (a, b, s) in &results {
            let _ = writeln!(out, "{}", result_json(a, b, s, ("input", "match")));
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
            let mut parts = line.splitn(2, |c| c == '\t' || c == ',');
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
            let _ = writeln!(out, "{}", result_json(la, lb, &s, ("a", "b")));
        }
        return ExitCode::SUCCESS;
    }

    // Single-pair mode.
    let a = &opts.positionals[0];
    let b = &opts.positionals[1];
    let s = score_pair(a, b, opts.hogl_weight);
    emit(a, b, &s, opts.json);
    if let Some(t) = opts.threshold {
        return if metric_value(&s, opts.metric) <= t {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    ExitCode::SUCCESS
}
```

Note: `opts.positionals[0]`/`[1]` are safe here — `parse_from` guarantees exactly 2 positionals when neither list nor stdin mode is active.

- [ ] **Step 6: Run full suite + clippy + build**

Run: `cargo test 2>&1 | tail -15 && cargo clippy 2>&1 | tail -15 && cargo build --release 2>&1 | tail -5`
Expected: all tests PASS; clippy clean; release build succeeds.

- [ ] **Step 7: Manual smoke test of all three modes**

```bash
# single pair: multi-char spoof now caught by skeleton metric
./target/release/sqdist rnicrosoft microsoft
# Expected: skeleton_damerau 0, confusable_only true, homoglyph_damerau > 1

# list mode with sort + top
printf 'paypal\np\xd0\xb0ypal\nunrelated\n' > /tmp/cand.txt
./target/release/sqdist --string paypal --list /tmp/cand.txt --sort --top 2
# Expected: 2 JSONL lines with "input"/"match" keys, the two zero-skeleton ones first

# threshold on skeleton metric
./target/release/sqdist --string paypal --list /tmp/cand.txt -t 0.0
# Expected: only the confusable matches (skeleton distance 0)
rm -f /tmp/cand.txt
```

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: wire --list watchlist mode and metric-driven thresholds into main

main() dispatches single-pair, --stdin, and --list modes. List mode scores
--string against each non-blank file line via process_list(), emits input/match
JSONL, and honors --sort/--top. Threshold checks now use the --metric distance
across all modes.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Rewrite the "Known limitation: multi-character homoglyphs" section**

Open `README.md`, find the section beginning `## Known limitation: multi-character homoglyphs` (currently ~line 78). Replace the entire section (through the line ending `...visually confusable.`) with:

```markdown
## Multi-character homoglyphs

Multi-character visual confusables ARE detected via full UTS#39
skeletonization — each string is reduced to its skeleton (every code point
mapped through the confusables table and concatenated) before measuring
distance:

- `rn` ↔ `m` (`rnicrosoft` vs `microsoft`)
- `vv` ↔ `w`
- `cl` ↔ `d`

These surface in the `skeleton_damerau` field (≈0 for a pure multi-char spoof)
and set `confusable_only` to `true`, even when the strings differ in length.
The `homoglyph_damerau` field uses per-character weighting and does NOT collapse
multi-char sequences, so comparing the two fields distinguishes "a few homoglyph
substitutions" from "fully visually confusable".

Leetspeak substitutions (`3`→`e`, `4`→`a`) are deliberately NOT treated as
homoglyphs because UTS #39 does not consider them visually confusable.
```

- [ ] **Step 2: Update the OPTIONS block and output-fields list**

In the `## Usage` section, replace the OPTIONS code block with one matching the new `print_usage` (include `--metric`, `--string`, `--list`, `--sort`, `--top`). In `## Output fields`, add `skeleton_damerau` and `skeleton_normalized` with one-line descriptions, and note that list mode uses `input`/`match` keys instead of `a`/`b`. (Use the field descriptions from the spec's Feature 1 table.)

- [ ] **Step 3: Add a watchlist usage example**

Under the `### Batch / pipeline` heading in `README.md`, add a new subsection.
The exact markdown to insert (a fenced `sh` block sits inside it — use real
triple-backtick fences):

> `### Watchlist mode (one string vs. a file)`
>
> Score a single name against every line of a candidates file — closer to how
> you'd screen registry/Artifactory package names against a known-good name.
> Open a `sh` code fence containing these two commands, then close it:
>
> - `# emit JSONL (input/match keys), most-suspicious first, top 10`
> - `sqdist --string paypal --list candidates.txt --sort --top 10`
> - (blank line)
> - `# alert-only: skeleton distance at/under the threshold`
> - `sqdist --string paypal --list candidates.txt -t 0.5`

- [ ] **Step 4: Verify the doc examples are accurate**

Build and run each README command against a scratch file to confirm output shape matches what the docs claim.

```bash
cargo build --release
printf 'paypal\np\xd0\xb0ypal\nfoobar\n' > /tmp/wl.txt
./target/release/sqdist --string paypal --list /tmp/wl.txt --sort --top 10
rm -f /tmp/wl.txt
```
Expected: JSONL lines with `input`/`match` keys; no discrepancy with the README text.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs: document skeletonization and --string/--list watchlist mode

Rewrite the multi-char limitation section (now solved via skeletonization),
document the new fields and flags, and add a watchlist usage example.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update the limitation framing and output contract**

In `CLAUDE.md`, find the section `### Known limitation — do not treat as a bug` and replace it with a short note that multi-char skeletonization IS now implemented (`skeleton()` builds the full skeleton; `skeleton_damerau`/`confusable_only` surface multi-char spoofs), keeping the note that leetspeak is intentionally excluded.

In the `## Output contract` section, update the field list to: `levenshtein, damerau, homoglyph_damerau, skeleton_damerau, normalized, skeleton_normalized, confusable_only`. Add that list mode (`--string`/`--list`) uses `input`/`match` keys, and that `-m/--metric` (default `skeleton`) selects the distance for `-t` and `--sort`.

- [ ] **Step 2: Add a mode-matrix note**

Under `## Architecture`, add a brief line documenting the three modes and their pairing/keys (single pair → positionals/`a`,`b`; `--stdin` → pre-paired/`a`,`b`; `--list` → one-vs-many/`input`,`match`), and that `main()` is a thin wrapper over `process_list`/`score_pair`/`sort_and_truncate`/`result_json`.

- [ ] **Step 3: Verify CLAUDE.md has no stale claims**

Run: `rg -n 'single code point|not.*caught|multi-char' CLAUDE.md`
Expected: confirm no remaining text claims multi-char confusables are uncaught. Fix any that remain.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "$(cat <<'EOF'
docs: update CLAUDE.md for skeletonization and list mode

Multi-char skeletonization is now implemented; update the limitation note, the
output-field contract (skeleton fields, input/match keys, --metric), and the
mode matrix.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Full test + lint + release build**

Run:
```bash
cargo test 2>&1 | tail -20
cargo clippy --all-targets 2>&1 | tail -20
cargo fmt --check 2>&1 | tail -5
cargo build --release 2>&1 | tail -5
```
Expected: all tests PASS; clippy clean; `fmt --check` clean (run `cargo fmt` and amend if not); release build succeeds.

- [ ] **Step 2: Confirm the headline behaviors end-to-end**

```bash
./target/release/sqdist rnicrosoft microsoft        # skeleton_damerau 0, confusable_only true
./target/release/sqdist paypal p$(printf '\xd0\xb0')ypal  # confusable_only true
printf 'paypal\nfoobar\n' > /tmp/v.txt
./target/release/sqdist --string paypal --list /tmp/v.txt --metric skeleton --sort
rm -f /tmp/v.txt
```
Expected outputs as annotated; no panics; exit codes sane.

- [ ] **Step 3: Push**

```bash
git push origin main
```
Expected: all task commits pushed to `origin/main`.

---

## Notes for the implementer

- **No new dependencies.** Everything uses std.
- **TDD throughout:** each task writes the failing test first, then the minimal code.
- **The existing six tests** call `levenshtein`/`damerau`/`confusable` directly and should keep passing; only `confusable_only`-related expectations could shift (Task 2, Step 4) — verify, don't blindly edit.
- **`confusables_data.rs` is generated** — never edit it by hand.
- If `skeleton_is_idempotent` fails (Task 1), STOP and report rather than adding recursion; it would indicate a spec assumption broke against the data.
