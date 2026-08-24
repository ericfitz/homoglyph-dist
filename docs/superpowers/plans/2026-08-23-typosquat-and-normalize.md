# Typosquat Profile and PyPI Normalize Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship v0.4.0 `--typosquat` (five-axis alert profile + four-way classification) and `--pypi` / `-n <file>` (PEP 503 identity-gate then score), without changing default JSON.

**Architecture:** A new `src/normalize.rs` owns `NormOp`, `apply_ops`, `parse_ops` (hand-rolled JSON subset, no new crates), `pypi_ops`, and `load_ops_file`. `src/verdict.rs` gains `TyposquatClass` + `classify_typosquat` beside the existing homoglyph `verdict()`. `src/main.rs` parses an ordered `Vec<NormStep>`, builds the op list in argv order, scores each pair through a `prepare_pair` helper, and emits opt-in JSON keys.

**Tech Stack:** Rust 2021, existing `unicode-security` only. No `serde`. `cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`.

## Global Constraints

- Version **0.4.0** (bump in the docs/version task, not earlier).
- **No new runtime dependencies.**
- Default `sqdist a b` JSON and spoof/benign verdict **unchanged** when none of `--typosquat` / `--pypi` / `-n` are set.
- `classification` is **not** an axis; do not add it to `ALL_AXES`.
- `--normalize` / `-n` is **always a filesystem path**. `--normalize pypi` opens a file named `pypi`. The PEP 503 preset is **only** `--pypi`.
- No inline JSON, no colon-delimited mini-language.
- `--typosquat` cannot combine with `-t` / `--threshold`.
- Under `--typosquat`, default `--metric` is `damerau` unless the user passed `-m`.
- Existing substring test `!line.contains("normalized")` is **only valid on default JSON**; `a_normalized` contains that substring.

---

## Context for the implementer (read before starting)

**Spec (source of truth):** `docs/superpowers/specs/2026-08-23-typosquat-and-normalize-design.md`

**Code to read:**
- `src/main.rs` — `Opts`, `parse_from`, `result_json`, `emit_human`, `score_pair`, `score_candidate`, `process_list`, `batch_matched_ok`, three modes in `main`.
- `src/verdict.rs` — homoglyph `Verdict` / `verdict()`. Do not change its tags or tests except where default-mode `[SAME PROJECT]` is handled **outside** `verdict()`.
- `src/axes.rs` — `parse_fields`, `ALL_AXES` order, `Panel::get`.
- `CLAUDE.md` — conventions.

**Per-pair scoring (implement exactly):**

1. Identifiers in output are always the **originals**.
2. If the op list is non-empty, `a_norm = apply_ops(a)`, `b_norm = apply_ops(b)`.
3. If originals are equal → score originals, class `identical`.
4. Else if ops ran and `a_norm == b_norm` → score **originals**, class `same_project`.
5. Else if ops ran → score **normalized** strings; else score originals.
6. `--typosquat` class: `confusable_only || damerau ≤ 1` and `max(scored char lens) ≥ 3` → `likely_typosquat`, else `unrelated` (after identical / same_project).
7. Batch/list `--typosquat`: emit only `likely_typosquat`; exit 1 if none.

**Working rules:** TDD. Before every commit: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Conventional Commits. Commit to the current branch. Do **not** push. Do **not** bump `Cargo.toml` version until Task 7.

---

## File map

| File | Responsibility |
|---|---|
| Create `src/normalize.rs` | `NormOp`, `apply_ops`, `parse_ops`, `pypi_ops`, `load_ops_file` |
| Modify `src/verdict.rs` | Add `TyposquatClass` + `classify_typosquat`; leave `verdict()` behavior unchanged |
| Modify `src/main.rs` | Flags, `prepare_pair`, JSON/human extras, batch filter, help |
| Modify `Cargo.toml` | version 0.4.0 (Task 7) |
| Modify `README.md`, `CLAUDE.md` | flags, classification, PEP 503 (Task 7) |

---

### Task 1: Normalizer apply engine (`NormOp` / `apply_ops` / `pypi_ops`)

**Files:**
- Create: `src/normalize.rs`
- Modify: `src/main.rs` (add `mod normalize;` only)

**Interfaces:**
- Consumes: nothing
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum NormOp {
      Lower,
      Map { charset: String, repl: String },
      Collapse { charset: String },
  }
  pub fn apply_ops(s: &str, ops: &[NormOp]) -> String;
  pub fn pypi_ops() -> Vec<NormOp>; // lower, map "._-" → "-", collapse "-"
  ```

- [ ] **Step 1: Write the failing tests in `src/normalize.rs`**

Create the module with tests first (the types can be empty stubs that fail). Put this file content as the starting point — tests included, implementation replaced by `unimplemented!()` until Step 3.

```rust
//! String normalizer: ordered ops for registry name identity (PEP 503 etc.).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormOp {
    Lower,
    Map { charset: String, repl: String },
    Collapse { charset: String },
}

pub fn apply_ops(s: &str, ops: &[NormOp]) -> String {
    let _ = (s, ops);
    unimplemented!("apply_ops")
}

pub fn pypi_ops() -> Vec<NormOp> {
    unimplemented!("pypi_ops")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(charset: &str, repl: &str) -> NormOp {
        NormOp::Map {
            charset: charset.into(),
            repl: repl.into(),
        }
    }
    fn collapse(charset: &str) -> NormOp {
        NormOp::Collapse {
            charset: charset.into(),
        }
    }

    #[test]
    fn lower_unicode() {
        assert_eq!(apply_ops("Friendly.Bard", &[NormOp::Lower]), "friendly.bard");
    }

    #[test]
    fn map_is_one_to_one_no_run_compression() {
        let ops = [map("._-", "-")];
        assert_eq!(apply_ops("my---package", &ops), "my---package");
        assert_eq!(apply_ops("my___package", &ops), "my---package");
        assert_eq!(apply_ops("my.-_package", &ops), "my---package");
    }

    #[test]
    fn collapse_same_char_only() {
        let ops = [collapse("-")];
        assert_eq!(apply_ops("my---package", &ops), "my-package");
        assert_eq!(apply_ops("my.-_package", &ops), "my.-_package");
        assert_eq!(apply_ops("my___package", &ops), "my___package");
    }

    #[test]
    fn collapse_charset_dot_underscore_hyphen_does_not_merge_mixed_run() {
        let ops = [collapse("._-")];
        assert_eq!(apply_ops("my.-_package", &ops), "my.-_package");
    }

    #[test]
    fn map_repl_may_be_empty_or_colon() {
        assert_eq!(apply_ops("a.b", &[map(".", "")]), "ab");
        assert_eq!(apply_ops("a_b", &[map("_", ":")]), "a:b");
    }

    #[test]
    fn empty_ops_is_identity() {
        assert_eq!(apply_ops("Foo_Bar", &[]), "Foo_Bar");
    }

    #[test]
    fn pypi_preset_matches_pep503() {
        let ops = pypi_ops();
        assert_eq!(apply_ops("my---package", &ops), "my-package");
        assert_eq!(apply_ops("my___package", &ops), "my-package");
        assert_eq!(apply_ops("my.-_package", &ops), "my-package");
        assert_eq!(apply_ops("requests_toolbelt", &ops), "requests-toolbelt");
        assert_eq!(apply_ops("Friendly.Bard", &ops), "friendly-bard");
        assert_eq!(apply_ops("FrIeNdLy-._.-bArD", &ops), "friendly-bard");
    }
}
```

Add `mod normalize;` to `src/main.rs` next to the other `mod` lines.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib apply_ops pypi_preset -- --nocapture` is wrong (this is a bin). Use:

```
cargo test map_is_one_to_one -- --nocapture
```

Expected: compile failure or panic `unimplemented!("apply_ops")`.

- [ ] **Step 3: Implement `apply_ops` and `pypi_ops`**

```rust
fn charset_has(charset: &str, c: char) -> bool {
    charset.chars().any(|x| x == c)
}

pub fn apply_ops(s: &str, ops: &[NormOp]) -> String {
    let mut cur = s.to_string();
    for op in ops {
        cur = match op {
            NormOp::Lower => cur.to_lowercase(),
            NormOp::Map { charset, repl } => {
                let mut out = String::new();
                for c in cur.chars() {
                    if charset_has(charset, c) {
                        out.push_str(repl);
                    } else {
                        out.push(c);
                    }
                }
                out
            }
            NormOp::Collapse { charset } => {
                let mut out = String::new();
                let mut prev: Option<char> = None;
                for c in cur.chars() {
                    if charset_has(charset, c) {
                        if prev == Some(c) {
                            continue;
                        }
                        prev = Some(c);
                        out.push(c);
                    } else {
                        prev = None;
                        out.push(c);
                    }
                }
                out
            }
        };
    }
    cur
}

pub fn pypi_ops() -> Vec<NormOp> {
    vec![
        NormOp::Lower,
        NormOp::Map {
            charset: "._-".into(),
            repl: "-".into(),
        },
        NormOp::Collapse {
            charset: "-".into(),
        },
    ]
}
```

- [ ] **Step 4: Run tests**

```
cargo test --bin sqdist normalize::
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: PASS, clippy/fmt clean. If fmt fails, run `cargo fmt`.

- [ ] **Step 5: Commit**

```bash
git add src/normalize.rs src/main.rs
git commit -m "feat: add registry name normalizer apply engine"
```

---

### Task 2: JSON op parser and file load

**Files:**
- Modify: `src/normalize.rs`

**Interfaces:**
- Consumes: `NormOp` from Task 1
- Produces:
  ```rust
  pub fn parse_ops(json: &str) -> Result<Vec<NormOp>, String>;
  pub fn load_ops_file(path: &str) -> Result<Vec<NormOp>, String>;
  ```

No new crates. Parse a **JSON array of arrays of strings** only (this is the entire rules schema). Support JSON string escapes at least: `\"`, `\\`, `\/`, `\n`, `\r`, `\t`, `\uXXXX`. Reject trailing non-whitespace, non-array root, non-array ops, non-string operands, unknown op, wrong arity, empty `CHARSET`. Empty root `[]` → `Ok(vec![])`.

- [ ] **Step 1: Write the failing tests** (append to `src/normalize.rs` tests)

```rust
    #[test]
    fn parse_ops_pypi_json() {
        let json = r#"[["lower"],["map","._-","-"],["collapse","-"]]"#;
        assert_eq!(parse_ops(json).unwrap(), pypi_ops());
    }

    #[test]
    fn parse_ops_colon_in_charset_and_repl() {
        let json = r#"[["map","._-:","-"],["collapse",":"]]"#;
        let ops = parse_ops(json).unwrap();
        assert_eq!(
            ops,
            vec![
                map("._-:", "-"),
                collapse(":"),
            ]
        );
        assert_eq!(apply_ops("foo:bar_baz", &ops), "foo-bar-baz");
    }

    #[test]
    fn parse_ops_empty_array_is_identity() {
        assert_eq!(parse_ops("[]").unwrap(), vec![]);
    }

    #[test]
    fn parse_ops_whitespace_ok() {
        let json = "[\n  [\"lower\"]\n]\n";
        assert_eq!(parse_ops(json).unwrap(), vec![NormOp::Lower]);
    }

    #[test]
    fn parse_ops_rejects_object_root() {
        assert!(parse_ops("{}").is_err());
    }

    #[test]
    fn parse_ops_rejects_unknown_op() {
        let e = parse_ops(r#"[["fold","._-","-"]]"#).unwrap_err();
        assert!(e.contains("unknown op"), "{e}");
        assert!(e.contains("fold"), "{e}");
    }

    #[test]
    fn parse_ops_rejects_empty_charset() {
        assert!(parse_ops(r#"[["map","","-"]]"#).is_err());
        assert!(parse_ops(r#"[["collapse",""]]"#).is_err());
    }

    #[test]
    fn parse_ops_rejects_wrong_arity() {
        assert!(parse_ops(r#"[["lower","x"]]"#).is_err());
        assert!(parse_ops(r#"[["map","._-"]]"#).is_err());
        assert!(parse_ops(r#"[["collapse","-","x"]]"#).is_err());
    }

    #[test]
    fn load_ops_file_reads_json() {
        let p = std::env::temp_dir().join(format!("sqdist-norm-{}.json", std::process::id()));
        std::fs::write(&p, r#"[["lower"]]"#).unwrap();
        let ops = load_ops_file(p.to_str().unwrap()).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(ops, vec![NormOp::Lower]);
    }

    #[test]
    fn load_ops_file_missing_path_errors() {
        let e = load_ops_file("/no/such/sqdist-normalize-rules.json").unwrap_err();
        assert!(e.contains("normalize") || e.contains("cannot") || e.contains("No such"), "{e}");
    }
```

- [ ] **Step 2: Run tests — expect compile fail (`parse_ops` missing)**

```
cargo test parse_ops_pypi_json
```

- [ ] **Step 3: Implement parser + loader**

Add `parse_ops` / `load_ops_file` to `src/normalize.rs`. A working subset parser:

```rust
pub fn parse_ops(json: &str) -> Result<Vec<NormOp>, String> {
    let mut p = Parser::new(json);
    p.skip_ws();
    let root = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err("normalize rules: trailing junk after JSON".into());
    }
    let JsonVal::Arr(ops) = root else {
        return Err("normalize rules: expected a JSON array of ops".into());
    };
    ops.into_iter().map(op_from_json).collect()
}

pub fn load_ops_file(path: &str) -> Result<Vec<NormOp>, String> {
    let meta = std::fs::metadata(path).map_err(|e| {
        format!("cannot read --normalize file {path:?}: {e}")
    })?;
    if !meta.is_file() {
        return Err(format!("--normalize {path:?} is not a file"));
    }
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read --normalize file {path:?}: {e}"))?;
    parse_ops(&data).map_err(|e| format!("{path}: {e}"))
}

enum JsonVal {
    Str(String),
    Arr(Vec<JsonVal>),
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
            pos: 0,
        }
    }
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn parse_value(&mut self) -> Result<JsonVal, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(JsonVal::Str(self.parse_string()?)),
            _ => Err("normalize rules: expected JSON string or array".into()),
        }
    }
    fn parse_array(&mut self) -> Result<JsonVal, String> {
        self.pos += 1; // [
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(JsonVal::Arr(items));
            }
            if !items.is_empty() {
                if self.peek() != Some(b',') {
                    return Err("normalize rules: expected comma in array".into());
                }
                self.pos += 1;
                self.skip_ws();
            }
            items.push(self.parse_value()?);
        }
    }
    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // "
        let mut out = String::new();
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => {
                            out.push('"');
                            self.pos += 1;
                        }
                        Some(b'\\') => {
                            out.push('\\');
                            self.pos += 1;
                        }
                        Some(b'/') => {
                            out.push('/');
                            self.pos += 1;
                        }
                        Some(b'n') => {
                            out.push('\n');
                            self.pos += 1;
                        }
                        Some(b'r') => {
                            out.push('\r');
                            self.pos += 1;
                        }
                        Some(b't') => {
                            out.push('\t');
                            self.pos += 1;
                        }
                        Some(b'u') => {
                            self.pos += 1;
                            let hex = self.bytes.get(self.pos..self.pos + 4).ok_or_else(|| {
                                "normalize rules: truncated \\u escape".to_string()
                            })?;
                            let hex = std::str::from_utf8(hex)
                                .map_err(|_| "normalize rules: bad \\u escape")?;
                            let cp = u32::from_str_radix(hex, 16)
                                .map_err(|_| "normalize rules: bad \\u escape")?;
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| "normalize rules: bad \\u escape".to_string())?;
                            out.push(ch);
                            self.pos += 4;
                        }
                        _ => return Err("normalize rules: bad string escape".into()),
                    }
                }
                c if c < 0x20 => return Err("normalize rules: unescaped control in string".into()),
                _ => {
                    let s = std::str::from_utf8(&self.bytes[self.pos..])
                        .map_err(|_| "normalize rules: invalid utf-8")?;
                    let ch = s.chars().next().unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        Err("normalize rules: unterminated string".into())
    }
}

fn as_str(v: JsonVal) -> Result<String, String> {
    match v {
        JsonVal::Str(s) => Ok(s),
        JsonVal::Arr(_) => Err("normalize rules: expected string".into()),
    }
}

fn op_from_json(v: JsonVal) -> Result<NormOp, String> {
    let JsonVal::Arr(items) = v else {
        return Err("normalize rules: each op must be a JSON array".into());
    };
    if items.is_empty() {
        return Err("normalize rules: empty op array".into());
    }
    let mut it = items.into_iter();
    let name = as_str(it.next().unwrap())?;
    match name.as_str() {
        "lower" => {
            if it.next().is_some() {
                return Err("normalize rules: lower takes no arguments".into());
            }
            Ok(NormOp::Lower)
        }
        "map" => {
            let charset = as_str(it.next().ok_or("normalize rules: map needs CHARSET")?)?;
            let repl = as_str(it.next().ok_or("normalize rules: map needs REPL")?)?;
            if it.next().is_some() {
                return Err("normalize rules: map takes exactly CHARSET and REPL".into());
            }
            if charset.is_empty() {
                return Err("normalize rules: empty CHARSET".into());
            }
            Ok(NormOp::Map { charset, repl })
        }
        "collapse" => {
            let charset = as_str(it.next().ok_or("normalize rules: collapse needs CHARSET")?)?;
            if it.next().is_some() {
                return Err("normalize rules: collapse takes exactly CHARSET".into());
            }
            if charset.is_empty() {
                return Err("normalize rules: empty CHARSET".into());
            }
            Ok(NormOp::Collapse { charset })
        }
        other => Err(format!("normalize rules: unknown op: {other}")),
    }
}
```

- [ ] **Step 4: Run tests**

```
cargo test --bin sqdist normalize::
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/normalize.rs
git commit -m "feat: parse normalize ops from JSON files"
```

---

### Task 3: `classify_typosquat`

**Files:**
- Modify: `src/verdict.rs`

**Interfaces:**
- Consumes: `Panel`, `AxisValue` (already imported)
- Produces:
  ```rust
  #[derive(Clone, Copy, PartialEq, Eq, Debug)]
  pub enum TyposquatClass {
      Identical,
      SameProject,
      LikelyTyposquat,
      Unrelated,
  }
  impl TyposquatClass {
      pub fn tag(self) -> &'static str;      // IDENTICAL / SAME PROJECT / LIKELY TYPOSQUAT / UNRELATED
      pub fn json_key(self) -> &'static str; // identical / same_project / likely_typosquat / unrelated
  }
  pub fn classify_typosquat(
      originals_equal: bool,
      same_project: bool,
      panel: &Panel,
      scored_len_a: usize,
      scored_len_b: usize,
  ) -> (TyposquatClass, String);
  ```

Do **not** change `verdict()` or existing `Verdict` variants.

Reason strings (stable enough to assert `contains`):
- identical: `The strings are identical.`
- same_project: `The names differ only by registry normalization (same project).`
- likely_typosquat && `confusable_only`: `Strings differ but share a confusable skeleton (visual lookalike).`
- likely_typosquat && damerau ≤ 1 (and not confusable_only): `1 Damerau edit.` If `keyboard_distance` is `AxisValue::Float(k)` and `k <= 0.0` (or `abs() < 1e-12`), append ` Keyboard distance 0 (no far-key substitutions).`
- unrelated: `No typosquat signal (damerau > 1 and not confusable-only).`

Classification order: identical → same_project → likely_typosquat (`confusable_only || damerau ≤ 1`, max scored len ≥ 3) → unrelated.

Reuse the existing private `int_axis` / `bool_axis` helpers.

- [ ] **Step 1: Write failing tests in `src/verdict.rs` `tests`**

Use the existing `panel(a, b)` helper already in that module.

```rust
    use super::TyposquatClass;

    fn class(a: &str, b: &str, same_project: bool) -> TyposquatClass {
        let p = panel(a, b);
        let scored_a = a.chars().count();
        let scored_b = b.chars().count();
        classify_typosquat(a == b, same_project, &p, scored_a, scored_b).0
    }

    #[test]
    fn typosquat_identical() {
        assert_eq!(class("lodash", "lodash", false), TyposquatClass::Identical);
    }

    #[test]
    fn typosquat_same_project_wins_over_damerau() {
        // Raw pair would be damerau=1; identity-gate must win.
        assert_eq!(class("foo_bar", "foo-bar", true), TyposquatClass::SameProject);
    }

    #[test]
    fn typosquat_lodahs_is_likely() {
        assert_eq!(class("lodash", "lodahs", false), TyposquatClass::LikelyTyposquat);
    }

    #[test]
    fn typosquat_1odash_visual() {
        assert_eq!(class("lodash", "1odash", false), TyposquatClass::LikelyTyposquat);
        let p = panel("lodash", "1odash");
        assert!(matches!(p.get("confusable_only"), Some(AxisValue::Bool(true))));
    }

    #[test]
    fn typosquat_rnicrosoft_despite_damerau_2() {
        assert_eq!(
            class("microsoft", "rnicrosoft", false),
            TyposquatClass::LikelyTyposquat
        );
        let p = panel("microsoft", "rnicrosoft");
        match p.get("damerau") {
            Some(AxisValue::Int(n)) => assert!(n >= 2, "damerau={n}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typosquat_go0gle() {
        assert_eq!(class("google", "go0gle", false), TyposquatClass::LikelyTyposquat);
    }

    #[test]
    fn typosquat_short_unrelated() {
        assert_eq!(class("ab", "ac", false), TyposquatClass::Unrelated);
    }

    #[test]
    fn typosquat_combosquat_unrelated() {
        assert_eq!(
            class("lodash", "lodash-utils", false),
            TyposquatClass::Unrelated
        );
    }
```

Add `use crate::axes::AxisValue;` in the test module if not already (verdict tests currently don't import AxisValue — add it).

- [ ] **Step 2: `cargo test typosquat_lodahs` — expect compile fail**

- [ ] **Step 3: Implement `TyposquatClass` + `classify_typosquat` in `src/verdict.rs`** (after `verdict()`, before `#[cfg(test)]`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TyposquatClass {
    Identical,
    SameProject,
    LikelyTyposquat,
    Unrelated,
}

impl TyposquatClass {
    pub fn tag(self) -> &'static str {
        match self {
            TyposquatClass::Identical => "IDENTICAL",
            TyposquatClass::SameProject => "SAME PROJECT",
            TyposquatClass::LikelyTyposquat => "LIKELY TYPOSQUAT",
            TyposquatClass::Unrelated => "UNRELATED",
        }
    }
    pub fn json_key(self) -> &'static str {
        match self {
            TyposquatClass::Identical => "identical",
            TyposquatClass::SameProject => "same_project",
            TyposquatClass::LikelyTyposquat => "likely_typosquat",
            TyposquatClass::Unrelated => "unrelated",
        }
    }
}

pub fn classify_typosquat(
    originals_equal: bool,
    same_project: bool,
    panel: &Panel,
    scored_len_a: usize,
    scored_len_b: usize,
) -> (TyposquatClass, String) {
    if originals_equal {
        return (
            TyposquatClass::Identical,
            "The strings are identical.".into(),
        );
    }
    if same_project {
        return (
            TyposquatClass::SameProject,
            "The names differ only by registry normalization (same project).".into(),
        );
    }
    let dam = int_axis(panel, "damerau");
    let confusable_only = bool_axis(panel, "confusable_only");
    let long = scored_len_a.max(scored_len_b);
    let likely = (confusable_only || dam <= 1) && long >= 3;
    if likely {
        let reason = if confusable_only {
            "Strings differ but share a confusable skeleton (visual lookalike).".into()
        } else {
            let mut r = "1 Damerau edit.".to_string();
            if let Some(AxisValue::Float(k)) = panel.get("keyboard_distance") {
                if k.abs() < 1e-12 {
                    r.push_str(" Keyboard distance 0 (no far-key substitutions).");
                }
            }
            r
        };
        return (TyposquatClass::LikelyTyposquat, reason);
    }
    (
        TyposquatClass::Unrelated,
        "No typosquat signal (damerau > 1 and not confusable-only).".into(),
    )
}
```

- [ ] **Step 4:** `cargo test typosquat_ ; cargo clippy --all-targets -- -D warnings; cargo fmt`

- [ ] **Step 5: Commit**

```bash
git add src/verdict.rs
git commit -m "feat: add typosquat classification tags"
```

---

### Task 4: CLI flags (`--typosquat`, `--pypi`, `-n`)

**Files:**
- Modify: `src/main.rs` (`Opts`, `parse_from`, `print_usage`)

**Interfaces:**
- Consumes: none of the scoring helpers yet
- Produces: parsed `Opts` fields:
  ```rust
  enum NormStep {
      Pypi,
      File(String),
  }
  // on Opts:
  typosquat: bool,
  norm_steps: Vec<NormStep>,
  metric_explicit: bool, // true iff user passed -m/--metric
  ```

After the parse loop (before mode checks):
- If `--typosquat` && `threshold.is_some()` → `Err("--typosquat cannot be combined with -t/--threshold")`
- If `--typosquat` && !`metric_explicit` → `metric = "damerau"`
- If `--typosquat` && `fields.is_none()` → `fields = Some(parse_fields("equal,damerau,skeleton_damerau,confusable_only,keyboard_distance").unwrap())`
- Duplicate `--pypi` → `Err("duplicate --pypi")`
- `-n` / `--normalize` requires a value; push `NormStep::File(path)` (even if path is `"pypi"`)

Initialize new fields in the `Opts { ... }` literal: `typosquat: false`, `norm_steps: Vec::new()`, `metric_explicit: false`.

On `-m`/`--metric` success, set `metric_explicit = true`.

Help text: add the three OPTIONS lines from the spec (verbatim meaning; wrap like existing `\x20   ` style).

- [ ] **Step 1: Write failing parse tests** in `src/main.rs` `tests`

```rust
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
        let o = parse_from(vec![
            "-n".into(),
            "pypi".into(),
            "a".into(),
            "b".into(),
        ])
        .unwrap();
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
```

These will fail to compile until `Opts` has the new fields.

- [ ] **Step 2: Confirm compile failure**

```
cargo test parse_typosquat_defaults
```

- [ ] **Step 3: Implement flags**

1. Add `enum NormStep { Pypi, File(String) }` (Debug + PartialEq for tests).
2. Extend `Opts` with `typosquat: bool`, `norm_steps: Vec<NormStep>`, `metric_explicit: bool`.
3. In `parse_from` init, set them false / empty / false.
4. Match arms:
   - `"--typosquat" => opts.typosquat = true`
   - `"--pypi"` → if `opts.norm_steps.iter().any(|s| matches!(s, NormStep::Pypi)) { return Err("duplicate --pypi".into()); }` then `push(NormStep::Pypi)`
   - `"-n" | "--normalize"` → `File(args.next().ok_or("--normalize needs a path")?)`
5. In `-m` arm, `opts.metric_explicit = true` after successful validate.
6. After the `while` loop, before list_mode checks:

```rust
    if opts.typosquat {
        if opts.threshold.is_some() {
            return Err(
                "--typosquat cannot be combined with -t/--threshold".into(),
            );
        }
        if !opts.metric_explicit {
            opts.metric = "damerau";
        }
        if opts.fields.is_none() {
            opts.fields = Some(
                parse_fields(
                    "equal,damerau,skeleton_damerau,confusable_only,keyboard_distance",
                )
                .expect("typosquat field list is valid"),
            );
        }
    }
```

7. `print_usage`: insert after `--len-tolerance` (keep `\x20   ` alignment):

```
         \x20   --typosquat             Package-typosquat profile: five axes, four-way\n\
         \x20                           classification, batch emits likely_typosquat only.\n\
         \x20                           Default metric damerau. Cannot combine with -t.\n\
         \x20   --pypi                  PEP 503 normalize (lower, map ._- → -, collapse -).\n\
         \x20                           Identity-gate same-project names; otherwise score\n\
         \x20                           normalized strings. Originals stay identifier keys.\n\
         \x20-n, --normalize <PATH>      Append normalize ops from a JSON file (ordered\n\
         \x20                           array of [op, …]). Repeatable. Not a preset name.\n\
```

Fix help wrapping if clippy/fmt complains about the ellipsis character; ASCII `...` is fine.

- [ ] **Step 4:** `cargo test parse_typosquat_ parse_pypi_ parse_normalize_ parse_duplicate_ parse_fields_overrides ; cargo clippy --all-targets -- -D warnings; cargo fmt`

Also run `cargo test parse_single_pair_still_works` — default metric must remain `skeleton_damerau`.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: parse --typosquat, --pypi, and --normalize path"
```

---

### Task 5: `prepare_pair`, JSON extras, single-pair human

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `apply_ops`, `pypi_ops`, `load_ops_file`, `NormStep`, `classify_typosquat`, `TyposquatClass`
- Produces: `prepare_pair`, extended `result_json` / `emit_human`, op-list build in `main`

```rust
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
    PairScore {
        a: a.to_string(),
        b: b.to_string(),
        a_norm,
        b_norm,
        panel: score_pair(sa, sb, cmap),
        scored_a: sa.to_string(),
        scored_b: sb.to_string(),
        identical,
        same_project,
    }
}
```

Extend `result_json`:

```rust
fn result_json(
    a: &str,
    b: &str,
    panel: &Panel,
    keys: (&str, &str),
    fields: Option<&[&'static str]>,
    norms: Option<(&str, &str)>,
    class: Option<(&str, &str)>, // (json_key, reason)
) -> String
```

After emitting `a`/`b` (or `input`/`match`), if `norms` is `Some((na, nb))` append `,"{k0}_normalized":{:?},"{k1}_normalized":{:?}` using `keys.0` / `keys.1`. Then axes. Then if `class` is Some, append `,"classification":"{ck}","reason":{:?}`.

Update **every existing** `result_json(...)` call to pass `None, None` as the last two args (keep default JSON tests passing, including `!line.contains("normalized")` on default output).

`emit_human` gains `ops_on: bool`, `norms: Option<(&str,&str)>`, `typosquat: bool`, `identical: bool`, `same_project: bool`. Before the axis rows: if `ops_on`, norms present, and at least one side differs from original, print `{ "normalized":<24} {na} / {nb}` (key padded to 24 like axes). After axes:
- if `typosquat`: `classify_typosquat(identical, same_project, panel, scored_a.chars().count(), scored_b.chars().count())` and print `[TAG] reason`
- else if `same_project`: print `[SAME PROJECT] The names differ only by registry normalization (same project).`
- else: existing `verdict(panel, orig_len_a, orig_len_b, len_tolerance)` — use **original** character counts for the old verdict (unchanged)

In `main`, after building `cmap`:

```rust
    let ops = match build_ops(&opts.norm_steps) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };
```

Single-pair path: `let scored = prepare_pair(a, b, &ops, &cmap);` then JSON/human from `scored`. Lengths for classify: `scored.scored_a.chars().count()`.

JSON class only if `opts.typosquat`. JSON norms only if `!ops.is_empty()` (always both keys, even when equal to originals).

- [ ] **Step 1: Write failing tests** (main.rs tests)

```rust
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
        assert!(line.contains("\"a_normalized\":\"foo-bar\"") || line.contains("\"a_normalized\":"));
        assert!(line.contains("a_normalized"));
        assert!(line.contains("b_normalized"));
        assert!(!line.contains("\"classification\""));
    }

    #[test]
    fn result_json_typosquat_adds_classification_after_axes() {
        let panel = score_pair("lodash", "lodahs", &cmap());
        let fields = parse_fields(
            "equal,damerau,skeleton_damerau,confusable_only,keyboard_distance",
        )
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
```

- [ ] **Step 2:** `cargo test result_json_pypi_adds` — compile fail on new arity / missing `prepare_pair`

- [ ] **Step 3: Implement signature change + helpers + single-pair `main` path**

**Must** update every `result_json` call in this file to the new arity in this task (otherwise the crate does not compile). List/stdin loops keep calling `score_pair` / `score_candidate` as today, but pass `None, None` for the new `result_json` args. Task 6 rewires those loops to `prepare_pair`.

Single-pair `main` (the block that currently does `let panel = score_pair(a, b, &cmap);`):

```rust
    let scored = prepare_pair(a, b, &ops, &cmap);
    let norms = match (scored.a_norm.as_deref(), scored.b_norm.as_deref()) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    let class = if opts.typosquat {
        let (c, reason) = verdict::classify_typosquat(
            scored.identical,
            scored.same_project,
            &scored.panel,
            scored.scored_a.chars().count(),
            scored.scored_b.chars().count(),
        );
        Some((c.json_key(), reason))
    } else {
        None
    };
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
                class.as_ref().map(|(k, r)| (*k, r.as_str())),
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
```

Lifetime of `reason` in `class`: keep `reason` in a `String` binding that lives across the print. Sketch:

```rust
    let class_owned: Option<( &'static str, String )> = if opts.typosquat {
        let (c, reason) = verdict::classify_typosquat(...);
        Some((c.json_key(), reason))
    } else {
        None
    };
    let class = class_owned
        .as_ref()
        .map(|(k, r)| (*k, r.as_str()));
```

Replace `emit_human` with:

```rust
fn emit_human(scored: &PairScore, fields: Option<&[&'static str]>, len_tolerance: f64, typosquat: bool, ops_on: bool) {
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
            scored.scored_a.chars().count(),
            scored.scored_b.chars().count(),
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
```

Import: `use verdict::{classify_typosquat, verdict};` or `verdict::classify_typosquat`.

Build `ops` at the start of `main` after `cmap` (needed for all three modes; list/stdin can ignore until Task 6). File errors → exit 2.

- [ ] **Step 4:** `cargo test result_json_ prepare_pair_ parse_ ; cargo test ; cargo clippy --all-targets -- -D warnings; cargo fmt`

Default `result_json_uses_given_keys_and_all_axes` must still pass (`!contains("normalized")` on default call with `None` norms).

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: score normalized pairs and emit opt-in JSON keys"
```

---

### Task 6: Batch/list `--typosquat` filter and exit codes

**Files:**
- Modify: `src/main.rs` (`score_candidate`, `process_list`, stdin loop, list loop, `batch_matched_ok`)

**Interfaces:**
- Consumes: `prepare_pair`, `classify_typosquat`, `opts.typosquat`, `ops`
- Produces: batch/list emit only `likely_typosquat` when `--typosquat`; exit 1 if zero such rows; without `--typosquat`, `-t` behavior unchanged; without `--typosquat` and without `-t`, emit all rows (including `same_project`)

Change `Row` from `(String, String, Panel)` to carry what JSON needs:

```rust
struct Row {
    a: String,
    b: String,
    a_norm: Option<String>,
    b_norm: Option<String>,
    panel: Panel,
}
```

Update `sort_and_truncate` to sort on `row.panel`.

`score_candidate` becomes:

```rust
fn score_candidate(
    string: &str,
    raw: &str,
    metric: &str,
    threshold: Option<f64>,
    cmap: &confusables::ConfusableMap,
    ops: &[normalize::NormOp],
    typosquat: bool,
) -> Option<Row>
```

- trim; skip empty
- `let s = prepare_pair(string, line, ops, cmap);`
- if `typosquat`: classify using scored lengths; keep only `TyposquatClass::LikelyTyposquat`
- else if `threshold`: keep if `row_metric(&s.panel, metric) <= t`
- return `Row { a: s.a, b: s.b, a_norm: s.a_norm, b_norm: s.b_norm, panel: s.panel }`

`process_list` threads `ops` and `typosquat`.

`batch_matched_ok`:

```rust
fn batch_matched_ok(threshold: Option<f64>, typosquat: bool, matched: bool) -> bool {
    if typosquat || threshold.is_some() {
        matched
    } else {
        true
    }
}
```

List and stdin emit paths: pass norms into `result_json` when `!ops.is_empty()`; pass classification when `typosquat` (batch `--typosquat` rows are always `likely_typosquat`, but still emit the keys). List keys `("input", "match")`.

- [ ] **Step 1: Failing tests** for filter helpers (unit-level, no process spawn)

```rust
    #[test]
    fn score_candidate_typosquat_keeps_lodahs() {
        let row = score_candidate(
            "lodash",
            "lodahs",
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
        let row = score_candidate(
            "lodash",
            "lodash-utils",
            "damerau",
            None,
            &cmap(),
            &[],
            true,
        );
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
```

- [ ] **Step 2:** `cargo test score_candidate_typosquat_keeps` — fail on arity

- [ ] **Step 3: Rewire `score_candidate` / `process_list` / both batch loops / `batch_matched_ok`**

Every `score_candidate` / `process_list` / `batch_matched_ok` call site in `main` must be updated. Grep `score_candidate(`, `process_list(`, `batch_matched_ok(`.

- [ ] **Step 4:** `cargo test ; cargo clippy --all-targets -- -D warnings; cargo fmt`

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: filter batch typosquat alerts and set exit codes"
```

---

### Task 7: Version 0.4.0, help already done, README + CLAUDE.md

**Files:**
- Modify: `Cargo.toml` (`version = "0.4.0"`)
- Modify: `README.md` — OPTIONS block, a short “Typosquat profile” subsection with the worked commands from the spec, note PEP 503 / `--pypi`, note `-n` is a file, note `--typosquat` cannot combine with `-t`, default metric under the profile is `damerau`
- Modify: `CLAUDE.md` — bump “currently 88” only if you recount after `cargo test`; document `--typosquat` / `--pypi` / `-n` in Modes; mention classification is not an axis; architecture list adds `src/normalize.rs`

Do not change default JSON examples in README except to add a *new* example for `--typosquat -j`.

- [ ] **Step 1: No new unit test required.** Run `cargo test` to get the live test count for CLAUDE.md.

- [ ] **Step 2: Bump version + docs**

`Cargo.toml`: `version = "0.4.0"`

README OPTIONS: insert the three flags as in help. Add after Usage (or after Watchlist):

```markdown
## Typosquat profile (package registries)

`--typosquat` emits five axes (`equal`, `damerau`, `skeleton_damerau`,
`confusable_only`, `keyboard_distance`), classifies each pair as
`identical` / `same_project` / `likely_typosquat` / `unrelated`, and in
batch/list mode prints **only** `likely_typosquat` rows (exit 1 if none).
It cannot be combined with `-t`. Default `--metric` becomes `damerau`.

`--pypi` applies PEP 503 (lowercase; map `._-` → `-`; collapse `-`). Names
that differ only by that rule are `same_project`, not squats. Otherwise
distances are on the normalized strings; JSON keeps the original identifiers
and adds `a_normalized` / `b_normalized` (or `input_normalized` /
`match_normalized`).

`--normalize` / `-n` is a **JSON file path**, not a preset name.
`--normalize pypi` reads a file named `pypi`. Repeatable; `--pypi` and `-n`
compose in argv order.

```sh
sqdist --typosquat lodash lodahs
sqdist --typosquat --string lodash --list new-packages.txt
sqdist --typosquat --pypi requests_toolbelt requests-toolbelt
sqdist --typosquat --pypi --string django --list new-pypi.txt
sqdist --typosquat -n ./rules.json --string foo --list names.txt
```

Rules file:

```json
[["lower"], ["map", "._-", "-"], ["collapse", "-"]]
```
```

CLAUDE.md: add `src/normalize.rs` to the architecture list; mention `--typosquat` classification and `--pypi` identity-gate in Modes / output contract; test count from `cargo test`.

- [ ] **Step 3:** `cargo test ; cargo clippy --all-targets -- -D warnings; cargo fmt --check`

Confirm `-v` would print `0.4.0` via `CARGO_PKG_VERSION` (the version test doesn't hardcode 0.3.0 today — do not add a hardcoded old version).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml README.md CLAUDE.md src/main.rs
git commit -m "docs: v0.4.0 typosquat profile and pypi normalize"
```

(`src/main.rs` only if help text was not finished in Task 4.)

---

## Spec coverage

| Spec item | Task |
|---|---|
| `apply_ops` map vs collapse vs pypi | 1 |
| JSON file ops, `[]`, colon in strings, schema errors | 2 |
| Four-way classification + reasons + min length 3 | 3 |
| `--typosquat` / `--pypi` / `-n` parse, argv order, `-n pypi` is a file, `-t` conflict, default metric/fields | 4 |
| Identity-gate scoring, `*_normalized`, classification JSON key order, human tags, default JSON unchanged, `[SAME PROJECT]` without `--typosquat` | 5 |
| Batch emit only `likely_typosquat`, exit 1 if none | 6 |
| v0.4.0, README, CLAUDE.md | 7 |
| No new crates | 2 (hand-rolled JSON) |
| `classification` not an axis | 4–5 (`--fields` still axis-only) |
