//! Minimal text KeyValues (VDF/ACF) parser. Steam's text files are small and
//! simple: quoted (or bare) tokens, `{ }` nesting, `//` comments, `\` escapes.

#[derive(Debug, Clone)]
pub enum Node {
    Str(String),
    Obj(Vec<(String, Node)>),
}

impl Node {
    /// Case-insensitive child lookup (Steam is inconsistent about key case).
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Obj(kids) => kids.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Node::Str(_) => None,
        }
    }

    /// Follow a path of keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Node> {
        keys.iter().try_fold(self, |n, k| n.get(k))
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            Node::Obj(_) => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Node::str)
    }

    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get_str(key).and_then(|s| s.trim().parse().ok())
    }

    pub fn children(&self) -> &[(String, Node)] {
        match self {
            Node::Obj(kids) => kids,
            Node::Str(_) => &[],
        }
    }
}

/// Parse a whole document. The result is an `Obj` holding the top-level pairs.
pub fn parse(src: &str) -> Node {
    let mut p = Parser { s: src.as_bytes(), i: 0 };
    Node::Obj(p.pairs())
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

enum Tok {
    Str(String),
    Open,
    Close,
}

impl Parser<'_> {
    fn pairs(&mut self) -> Vec<(String, Node)> {
        let mut out = Vec::new();
        loop {
            let key = match self.tok() {
                Some(Tok::Str(k)) => k,
                Some(Tok::Close) | None => return out,
                Some(Tok::Open) => continue, // malformed; skip
            };
            match self.tok() {
                Some(Tok::Str(v)) => out.push((key, Node::Str(v))),
                Some(Tok::Open) => {
                    let kids = self.pairs();
                    out.push((key, Node::Obj(kids)));
                }
                Some(Tok::Close) | None => return out,
            }
        }
    }

    fn tok(&mut self) -> Option<Tok> {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'{' => Some(Tok::Open),
            b'}' => Some(Tok::Close),
            b'"' => {
                let mut buf = Vec::new();
                while let Some(&c) = self.s.get(self.i) {
                    self.i += 1;
                    match c {
                        b'"' => break,
                        b'\\' => {
                            let e = self.s.get(self.i).copied().unwrap_or(b'\\');
                            self.i += 1;
                            buf.push(match e {
                                b'n' => b'\n',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                        _ => buf.push(c),
                    }
                }
                Some(Tok::Str(String::from_utf8_lossy(&buf).into_owned()))
            }
            _ => {
                let start = self.i - 1;
                while self.i < self.s.len()
                    && !self.s[self.i].is_ascii_whitespace()
                    && !b"{}\"".contains(&self.s[self.i])
                {
                    self.i += 1;
                }
                Some(Tok::Str(String::from_utf8_lossy(&self.s[start..self.i]).into_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_and_escapes() {
        let n = parse(
            r#"// c
"AppState" { "appid" "10" "name" "Counter \"X\"" "InstalledDepots" { "11" { "size" "5" } } }"#,
        );
        let a = n.get("appstate").unwrap();
        assert_eq!(a.get_u64("AppID"), Some(10));
        assert_eq!(a.get_str("name"), Some("Counter \"X\""));
        assert_eq!(a.path(&["InstalledDepots", "11", "size"]).and_then(Node::str), Some("5"));
    }
}
