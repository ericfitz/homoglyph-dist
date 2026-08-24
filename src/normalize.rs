//! String normalizer: ordered ops for registry name identity (PEP 503 etc.).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormOp {
    Lower,
    Map { charset: String, repl: String },
    Collapse { charset: String },
}

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
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("cannot read --normalize file {path:?}: {e}"))?;
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
        assert_eq!(
            apply_ops("Friendly.Bard", &[NormOp::Lower]),
            "friendly.bard"
        );
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

    #[test]
    fn parse_ops_pypi_json() {
        let json = r#"[["lower"],["map","._-","-"],["collapse","-"]]"#;
        assert_eq!(parse_ops(json).unwrap(), pypi_ops());
    }

    #[test]
    fn parse_ops_colon_in_charset_and_repl() {
        let json = r#"[["map","._-:","-"],["collapse",":"]]"#;
        let ops = parse_ops(json).unwrap();
        assert_eq!(ops, vec![map("._-:", "-"), collapse(":"),]);
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
        assert!(
            e.contains("normalize") || e.contains("cannot") || e.contains("No such"),
            "{e}"
        );
    }
}
