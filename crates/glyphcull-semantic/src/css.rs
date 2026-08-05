//! A strict CSS subset parser (SPEC/DESIGN: the compiler owns CSS translation;
//! the runtime never sees CSS).
//!
//! Supported grammar (v1, strict — malformed input is rejected with a position):
//!
//! ```text
//! stylesheet := rule*
//! rule       := selector_list '{' declaration* '}'
//! selector_list := complex (',' complex)*
//! complex    := simple+            (descendant chain: last matches the node,
//!                                    earlier ones match ancestors)
//! simple     := ['#'id]? ['.'class]* [tag]?   (at most one tag; classes in order)
//! declaration:= name ':' value ';'
//! ```
//!
//! Comments (`/* */`), whitespace, and string/identifier values are handled
//! generically; the *interpretation* of declarations happens in glyphcull-chunk's
//! style resolver (relative units are resolved against the parent's computed
//! style, per the CSS cascade).

use std::fmt;

/// A single compound selector: at most one tag, one id, and any number of classes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleSelector {
    /// Element tag (lowercase), if any.
    pub tag: Option<String>,
    /// Element id, if any.
    pub id: Option<String>,
    /// Classes, in document order.
    pub classes: Vec<String>,
}

impl SimpleSelector {
    /// The selector specificity (id > class > tag), per CSS.
    #[must_use]
    pub fn specificity(&self) -> (usize, usize, usize) {
        (
            usize::from(self.id.is_some()),
            self.classes.len(),
            usize::from(self.tag.is_some()),
        )
    }
}

/// A complex selector: a descendant chain (last element matches the node; the
/// earlier elements must match ancestors, innermost first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexSelector {
    /// The chain, from outermost to innermost (last matches the node itself).
    pub chain: Vec<SimpleSelector>,
}

/// One declaration: raw name/value, interpreted by the style resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Property name (lowercase).
    pub name: String,
    /// Raw value text (trimmed).
    pub value: String,
}

/// One CSS rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssRule {
    /// The matching selectors (specificity + order decide the cascade).
    pub selectors: Vec<ComplexSelector>,
    /// Declarations in source order.
    pub declarations: Vec<Declaration>,
}

/// A parsed stylesheet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stylesheet {
    /// Rules in source order.
    pub rules: Vec<CssRule>,
}

/// CSS parsing errors, with the byte position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssError {
    /// Byte offset of the error.
    pub offset: usize,
    /// Human-readable message.
    pub message: String,
}

impl fmt::Display for CssError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CSS error at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for CssError {}

/// The maximum accepted stylesheet length (defensive).
pub const MAX_STYLESHEET_LEN: usize = 1 << 20;

/// Parse a stylesheet.
pub fn parse_stylesheet(text: &str) -> Result<Stylesheet, CssError> {
    if text.len() > MAX_STYLESHEET_LEN {
        return Err(CssError {
            offset: MAX_STYLESHEET_LEN,
            message: "stylesheet too large".to_string(),
        });
    }
    let mut p = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let mut rules = Vec::new();
    loop {
        p.skip_ws_and_comments()?;
        if p.pos >= p.bytes.len() {
            break;
        }
        let rule = p.parse_rule()?;
        rules.push(rule);
    }
    Ok(Stylesheet { rules })
}

/// Parse a declaration list (as found in `style="..."` attributes): a sequence
/// of `name: value;` pairs without a selector/rule wrapper.
pub fn parse_declarations(text: &str) -> Result<Vec<Declaration>, CssError> {
    if text.len() > MAX_STYLESHEET_LEN {
        return Err(CssError {
            offset: MAX_STYLESHEET_LEN,
            message: "stylesheet too large".to_string(),
        });
    }
    let mut p = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let mut out = Vec::new();
    loop {
        p.skip_ws_and_comments()?;
        match p.peek() {
            None => break,
            Some(b';') => {
                p.pos += 1;
            }
            Some(_) => out.push(p.parse_declaration()?),
        }
    }
    Ok(out)
}

/// A position-tracking scanner over the stylesheet bytes.
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, message: impl Into<String>) -> CssError {
        CssError {
            offset: self.pos,
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws_and_comments(&mut self) -> Result<(), CssError> {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'\x0C') => {
                    self.pos += 1;
                }
                Some(b'/') if self.bytes.get(self.pos + 1) == Some(&b'*') => {
                    self.pos += 2;
                    while !(self.bytes.get(self.pos) == Some(&b'*')
                        && self.bytes.get(self.pos + 1) == Some(&b'/'))
                    {
                        self.pos += 1;
                        if self.pos >= self.bytes.len() {
                            return Err(self.err("unterminated comment"));
                        }
                    }
                    self.pos += 2;
                }
                _ => return Ok(()),
            }
        }
    }

    fn parse_rule(&mut self) -> Result<CssRule, CssError> {
        let selectors = self.parse_selector_list()?;
        self.skip_ws_and_comments()?;
        if self.bump() != Some(b'{') {
            return Err(self.err("expected '{'"));
        }
        let mut declarations = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            match self.peek() {
                None => return Err(self.err("unterminated rule block")),
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                Some(b';') => {
                    self.pos += 1; // stray semicolon: tolerated
                }
                Some(_) => {
                    declarations.push(self.parse_declaration()?);
                }
            }
        }
        Ok(CssRule {
            selectors,
            declarations,
        })
    }

    fn parse_selector_list(&mut self) -> Result<Vec<ComplexSelector>, CssError> {
        let mut out = Vec::new();
        loop {
            out.push(self.parse_complex_selector()?);
            self.skip_ws_and_comments()?;
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        Ok(out)
    }

    fn parse_complex_selector(&mut self) -> Result<ComplexSelector, CssError> {
        let mut chain = Vec::new();
        loop {
            chain.push(self.parse_simple_selector()?);
            self.skip_ws_and_comments()?;
            // A descendant combinator is implied by whitespace followed by
            // another simple selector start (#, ., or a letter).
            match self.peek() {
                Some(b'#') | Some(b'.') | Some(b'a'..=b'z') | Some(b'A'..=b'Z') => continue,
                _ => break,
            }
        }
        if chain.is_empty() {
            return Err(self.err("empty selector"));
        }
        Ok(ComplexSelector { chain })
    }

    fn parse_simple_selector(&mut self) -> Result<SimpleSelector, CssError> {
        let mut sel = SimpleSelector::default();
        loop {
            match self.peek() {
                Some(b'#') => {
                    if sel.id.is_some() {
                        return Err(self.err("duplicate id in selector"));
                    }
                    self.pos += 1;
                    sel.id = Some(self.parse_ident()?);
                }
                Some(b'.') => {
                    self.pos += 1;
                    sel.classes.push(self.parse_ident()?);
                }
                Some(b'a'..=b'z') | Some(b'A'..=b'Z') => {
                    if sel.tag.is_some() {
                        return Err(self.err("duplicate tag in selector"));
                    }
                    sel.tag = Some(self.parse_ident()?.to_lowercase());
                }
                _ => break,
            }
        }
        Ok(sel)
    }

    /// Parse an identifier: letters, digits, '-', '_' (must not start with a digit).
    fn parse_ident(&mut self) -> Result<String, CssError> {
        let start = self.pos;
        let mut out = String::new();
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                out.push(char::from(b));
                self.pos += 1;
            } else {
                break;
            }
        }
        if out.is_empty() || out.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) {
            self.pos = start;
            return Err(self.err("expected identifier"));
        }
        Ok(out)
    }

    fn parse_declaration(&mut self) -> Result<Declaration, CssError> {
        let name = self.parse_ident()?.to_lowercase();
        self.skip_ws_and_comments()?;
        if self.bump() != Some(b':') {
            return Err(self.err("expected ':' after property name"));
        }
        self.skip_ws_and_comments()?;
        // Value runs until an unquoted ';' or '}'.
        let start = self.pos;
        let mut in_string = false;
        let mut escaped = false;
        while let Some(b) = self.peek() {
            if escaped {
                escaped = false;
                self.pos += 1;
                continue;
            }
            match b {
                b'\\' => {
                    escaped = true;
                    self.pos += 1;
                }
                b'"' | b'\'' => {
                    in_string = !in_string;
                    self.pos += 1;
                }
                b';' if !in_string => break,
                b'}' if !in_string => break,
                _ => self.pos += 1,
            }
        }
        let value = std::str::from_utf8(
            self.bytes
                .get(start..self.pos)
                .ok_or_else(|| self.err("invalid value range"))?,
        )
        .map_err(|_| self.err("invalid UTF-8 in value"))?
        .trim()
        .to_string();
        if value.is_empty() {
            return Err(self.err("empty declaration value"));
        }
        Ok(Declaration { name, value })
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_stylesheet, SimpleSelector};

    #[test]
    fn simple_rules() {
        let css = "p { color: #ff0000; }\n.note { font-size: 14px; }";
        let sheet = parse_stylesheet(css).expect("parse");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selectors[0].chain.len(), 1);
        assert_eq!(
            sheet.rules[0].selectors[0].chain[0].tag.as_deref(),
            Some("p")
        );
        assert_eq!(sheet.rules[0].declarations[0].value, "#ff0000");
        assert_eq!(
            sheet.rules[1].selectors[0].chain[0].classes,
            vec!["note".to_string()]
        );
    }

    #[test]
    fn compound_and_descendant_selectors() {
        let css = "div.note#x em { font-style: italic; }";
        let sheet = parse_stylesheet(css).expect("parse");
        let chain = &sheet.rules[0].selectors[0].chain;
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].tag.as_deref(), Some("div"));
        assert_eq!(chain[0].classes, vec!["note".to_string()]);
        assert_eq!(chain[0].id.as_deref(), Some("x"));
        assert_eq!(chain[1].tag.as_deref(), Some("em"));
    }

    #[test]
    fn selector_lists() {
        let css = "h1, h2, h3 { color: blue; }";
        let sheet = parse_stylesheet(css).expect("parse");
        assert_eq!(sheet.rules[0].selectors.len(), 3);
    }

    #[test]
    fn comments_and_whitespace() {
        let css = "/* header */\np { /* inner */ color: red; }\n";
        let sheet = parse_stylesheet(css).expect("parse");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations[0].value, "red");
    }

    #[test]
    fn quoted_values() {
        let css = "p { font-family: \"Noto Sans\", sans-serif; }";
        let sheet = parse_stylesheet(css).expect("parse");
        assert_eq!(
            sheet.rules[0].declarations[0].value,
            "\"Noto Sans\", sans-serif"
        );
    }

    #[test]
    fn specificity_order() {
        let mut s1 = SimpleSelector {
            tag: Some("p".into()),
            id: None,
            classes: vec![],
        };
        let s2 = SimpleSelector {
            tag: None,
            id: Some("x".into()),
            classes: vec![],
        };
        assert!(s1.specificity() < s2.specificity());
        s1.classes.push("a".into());
        assert!(s1.specificity() > (0, 0, 1));
    }

    #[test]
    fn malformed_rejected() {
        assert!(parse_stylesheet("p { color: red").is_err()); // unterminated
        assert!(parse_stylesheet("p color: red; }").is_err()); // missing '{'
        assert!(parse_stylesheet("/* unterminated").is_err());
        assert!(parse_stylesheet("p { color }").is_err()); // missing ':'
    }

    #[test]
    fn deterministic() {
        let css = "p { color: #010203; }\n";
        assert_eq!(
            parse_stylesheet(css).expect("a"),
            parse_stylesheet(css).expect("b")
        );
    }
}
