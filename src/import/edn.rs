//! A minimal EDN reader — just enough to read Logseq whiteboard files
//! (`whiteboards/*.edn`). Handles maps, vectors, lists, strings, keywords,
//! symbols, numbers, `true`/`false`/`nil`, `,` as whitespace, `;` comments, and
//! `#`-dispatch (`#_` discard, `#{…}` set, `#tag value` — the tag is dropped and
//! the value kept). Not a complete EDN implementation; unknown forms parse as
//! symbols rather than erroring.

/// A parsed EDN value. Keyword/Symbol hold the text without the leading `:`.
#[derive(Debug, Clone, PartialEq)]
pub enum Edn {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Keyword(String),
    Symbol(String),
    Vec(Vec<Edn>),
    List(Vec<Edn>),
    Map(Vec<(Edn, Edn)>),
}

impl Edn {
    /// The value for keyword `key` in a map (`key` without the leading `:`).
    pub fn get(&self, key: &str) -> Option<&Edn> {
        match self {
            Edn::Map(pairs) => pairs.iter().find_map(|(k, v)| match k {
                Edn::Keyword(kw) if kw == key => Some(v),
                _ => None,
            }),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Edn::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Edn::Float(f) => Some(*f),
            Edn::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Elements of a vector or list.
    pub fn as_seq(&self) -> Option<&[Edn]> {
        match self {
            Edn::Vec(v) | Edn::List(v) => Some(v),
            _ => None,
        }
    }
}

/// Parse a single top-level EDN value (the rest of the input is ignored).
pub fn parse(s: &str) -> Option<Edn> {
    Parser {
        chars: s.chars().collect(),
        pos: 0,
    }
    .value()
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// Skip whitespace, commas (EDN treats `,` as whitespace), and `;` comments.
    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() || c == ',' => self.pos += 1,
                Some(';') => {
                    while let Some(c) = self.bump() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn value(&mut self) -> Option<Edn> {
        self.skip_ws();
        match self.peek()? {
            '{' => self.seq('}').map(Edn::map_pairs),
            '[' => self.seq(']').map(Edn::Vec),
            '(' => self.seq(')').map(Edn::List),
            '"' => self.string(),
            ':' => {
                self.bump();
                Some(Edn::Keyword(self.token()))
            }
            '#' => self.dispatch(),
            _ => Some(self.atom()),
        }
    }

    /// Read elements until `close`. For `}` the caller pairs them into a map.
    fn seq(&mut self, close: char) -> Option<Vec<Edn>> {
        self.bump(); // opening delimiter
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(c) if c == close => {
                    self.bump();
                    return Some(items);
                }
                None => return None,
                _ => items.push(self.value()?),
            }
        }
    }

    fn dispatch(&mut self) -> Option<Edn> {
        self.bump(); // #
        match self.peek()? {
            '{' => self.seq('}').map(Edn::Vec), // set → treat as a vector
            '_' => {
                self.bump();
                self.value()?; // discard the next form…
                self.value() // …and return the one after it
            }
            _ => {
                self.token(); // tag (e.g. uuid, inst) — dropped
                self.value() // keep the tagged value
            }
        }
    }

    fn string(&mut self) -> Option<Edn> {
        self.bump(); // opening quote
        let mut s = String::new();
        while let Some(c) = self.bump() {
            match c {
                '"' => return Some(Edn::Str(s)),
                '\\' => match self.bump()? {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    other => s.push(other),
                },
                _ => s.push(c),
            }
        }
        None // unterminated
    }

    fn atom(&mut self) -> Edn {
        let tok = self.token();
        match tok.as_str() {
            "true" => Edn::Bool(true),
            "false" => Edn::Bool(false),
            "nil" => Edn::Nil,
            _ => {
                if let Ok(i) = tok.parse::<i64>() {
                    Edn::Int(i)
                } else if let Ok(f) = tok.parse::<f64>() {
                    Edn::Float(f)
                } else {
                    Edn::Symbol(tok)
                }
            }
        }
    }

    /// Read a bare token up to the next delimiter or whitespace.
    fn token(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, ',' | ';' | '{' | '}' | '[' | ']' | '(' | ')' | '"')
            {
                break;
            }
            s.push(c);
            self.pos += 1;
        }
        s
    }
}

impl Edn {
    /// Pair a flat element list into a map (`{:k v :k2 v2}`); a dangling final
    /// key is dropped.
    fn map_pairs(items: Vec<Edn>) -> Edn {
        let mut pairs = Vec::with_capacity(items.len() / 2);
        let mut it = items.into_iter();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            pairs.push((k, v));
        }
        Edn::Map(pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_whiteboard_shape() {
        let e = parse(
            r#"{:blocks ({:block/properties
                {:ls-type :whiteboard-shape
                 :logseq.tldraw.shape
                 {:type "text" :point [3577.7 53.8] :size [619 68]
                  :fontSize 48 :text "Hi \"there\"" :nonce 1664366266514}}})}"#,
        )
        .unwrap();
        let blocks = e.get("blocks").unwrap().as_seq().unwrap();
        let shape = blocks[0]
            .get("block/properties")
            .unwrap()
            .get("logseq.tldraw.shape")
            .unwrap();
        assert_eq!(shape.get("type").unwrap().as_str(), Some("text"));
        assert_eq!(shape.get("text").unwrap().as_str(), Some("Hi \"there\""));
        assert_eq!(shape.get("fontSize").unwrap().as_f64(), Some(48.0));
        let point = shape.get("point").unwrap().as_seq().unwrap();
        assert_eq!(point[0].as_f64(), Some(3577.7));
        assert_eq!(
            shape.get("size").unwrap().as_seq().unwrap()[1].as_f64(),
            Some(68.0)
        );
    }

    #[test]
    fn handles_comments_commas_and_dispatch() {
        let e = parse(
            r#"; a comment
            {:a 1, :id #uuid "abc", :keep #_ :dropped :real, :flag true, :n nil}"#,
        )
        .unwrap();
        assert_eq!(e.get("a").unwrap(), &Edn::Int(1));
        assert_eq!(e.get("id").unwrap().as_str(), Some("abc")); // tag dropped, value kept
        assert_eq!(e.get("keep").unwrap(), &Edn::Keyword("real".into())); // #_ skipped :dropped
        assert_eq!(e.get("flag").unwrap(), &Edn::Bool(true));
        assert_eq!(e.get("n").unwrap(), &Edn::Nil);
    }
}
