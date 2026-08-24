//! String normalizer: ordered ops for registry name identity (PEP 503 etc.).

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // TODO(task-5): remove once used by prepare_pair
pub enum NormOp {
    Lower,
    Map { charset: String, repl: String },
    Collapse { charset: String },
}

#[allow(dead_code)] // TODO(task-5): remove once apply_ops is wired
fn charset_has(charset: &str, c: char) -> bool {
    charset.chars().any(|x| x == c)
}

#[allow(dead_code)] // TODO(task-5): remove once used by prepare_pair
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

#[allow(dead_code)] // TODO(task-5): remove once used by prepare_pair / --pypi
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
}
