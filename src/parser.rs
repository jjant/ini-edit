//! Error-tolerant recursive-descent parser producing a lossless green tree.
//!
//! Always produces a valid tree — errors go to a side channel. The central
//! invariant is: `parse(s).syntax().text() == s` for every input.

use rowan::GreenNodeBuilder;

use crate::lexer::{Token, lex};
use crate::syntax_kind::{SyntaxKind, SyntaxNode};

/// Result of parsing an INI source string.
#[derive(Debug, Clone)]
pub struct Parse {
    green: rowan::GreenNode,
    errors: Vec<ParseError>,
}

impl Parse {
    /// The lossless syntax tree root.
    #[must_use]
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Underlying green tree (cheap to clone).
    #[must_use]
    pub fn green(&self) -> &rowan::GreenNode {
        &self.green
    }

    /// Recoverable errors. Even when non-empty the tree still round-trips.
    #[must_use]
    pub fn errors(&self) -> &[ParseError] {
        &self.errors
    }
}

/// A non-fatal parse error with byte offset for diagnostic rendering.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// Human-readable description.
    pub message: String,
    /// Byte offset in the original source.
    pub offset: usize,
}

/// Parse an INI source string.
#[must_use]
pub fn parse(input: &str) -> Parse {
    let tokens = lex(input);
    let mut p = Parser {
        tokens,
        cursor: 0,
        builder: GreenNodeBuilder::new(),
        errors: Vec::new(),
    };
    p.parse_root();
    Parse {
        green: p.builder.finish(),
        errors: p.errors,
    }
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    cursor: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<ParseError>,
}

impl Parser<'_> {
    fn at_end(&self) -> bool {
        self.cursor >= self.tokens.len()
    }

    fn peek(&self) -> Option<SyntaxKind> {
        self.tokens.get(self.cursor).map(|t| t.kind)
    }

    fn bump(&mut self) {
        let tok = self.tokens[self.cursor];
        self.builder.token(tok.kind.into(), tok.text);
        self.cursor += 1;
    }

    fn error(&mut self, msg: impl Into<String>) {
        let offset = self.tokens[..self.cursor]
            .iter()
            .map(|t| t.text.len())
            .sum();
        self.errors.push(ParseError {
            message: msg.into(),
            offset,
        });
    }

    fn parse_root(&mut self) {
        self.builder.start_node(SyntaxKind::ROOT.into());
        while !self.at_end() {
            match self.peek().unwrap() {
                k if k.is_trivia() => self.bump(),
                SyntaxKind::L_BRACK => self.parse_section(),
                SyntaxKind::IDENT => self.parse_entry(),
                other => {
                    self.error(format!("unexpected token: {other:?}"));
                    self.bump();
                }
            }
        }
        self.builder.finish_node();
    }

    fn parse_section(&mut self) {
        self.builder.start_node(SyntaxKind::SECTION.into());
        self.parse_section_header();
        while !self.at_end() {
            match self.peek().unwrap() {
                k if k.is_trivia() => self.bump(),
                SyntaxKind::L_BRACK => break,
                SyntaxKind::IDENT => self.parse_entry(),
                other => {
                    self.error(format!("unexpected token in section: {other:?}"));
                    self.bump();
                }
            }
        }
        self.builder.finish_node();
    }

    fn parse_section_header(&mut self) {
        self.builder.start_node(SyntaxKind::SECTION_HEADER.into());
        if self.peek() == Some(SyntaxKind::L_BRACK) {
            self.bump();
        }
        if self.peek() == Some(SyntaxKind::WHITESPACE) {
            self.bump();
        }
        if self.peek() == Some(SyntaxKind::IDENT) {
            self.bump();
        } else {
            self.error("expected section name");
        }
        if self.peek() == Some(SyntaxKind::WHITESPACE) {
            self.bump();
        }
        if self.peek() == Some(SyntaxKind::R_BRACK) {
            self.bump();
        } else {
            self.error("expected ']'");
        }
        self.builder.finish_node();
    }

    fn parse_entry(&mut self) {
        self.builder.start_node(SyntaxKind::ENTRY.into());

        self.builder.start_node(SyntaxKind::KEY.into());
        if self.peek() == Some(SyntaxKind::IDENT) {
            self.bump();
        }
        self.builder.finish_node();

        if self.peek() == Some(SyntaxKind::WHITESPACE) {
            self.bump();
        }
        match self.peek() {
            Some(SyntaxKind::EQ | SyntaxKind::COLON) => self.bump(),
            _ => self.error("expected '=' or ':'"),
        }
        if self.peek() == Some(SyntaxKind::WHITESPACE) {
            self.bump();
        }

        self.builder.start_node(SyntaxKind::VALUE.into());
        if self.peek() == Some(SyntaxKind::VALUE_TEXT) {
            self.bump();
        }
        self.builder.finish_node();

        if self.peek() == Some(SyntaxKind::WHITESPACE) {
            self.bump();
        }
        if self.peek() == Some(SyntaxKind::NEWLINE) {
            self.bump();
        }

        self.builder.finish_node();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_round_trip(input: &str) {
        let p = parse(input);
        assert_eq!(p.syntax().text().to_string(), input);
    }

    #[test]
    fn empty() {
        assert_round_trip("");
    }

    #[test]
    fn simple_kv() {
        assert_round_trip("key=value\n");
    }

    #[test]
    fn section() {
        assert_round_trip("[s]\nk=v\n");
    }

    #[test]
    fn multiple_sections() {
        assert_round_trip("; comment\n[a]\nx = 1\n\n[b]\ny = 2\n");
    }

    #[test]
    fn preamble_entries() {
        assert_round_trip("g=1\n[s]\nk=v\n");
    }

    #[test]
    fn weird_whitespace() {
        assert_round_trip("  [ s ]  \n  k  =  v  \n");
    }

    #[test]
    fn malformed_still_round_trips() {
        assert_round_trip("[unclosed\n");
        assert_round_trip("[]\n");
    }

    #[test]
    fn errors_reported() {
        let p = parse("[unclosed\n");
        assert!(!p.errors().is_empty());
    }

    #[test]
    fn root_kind() {
        assert_eq!(parse("").syntax().kind(), SyntaxKind::ROOT);
    }
}
