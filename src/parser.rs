//! Error-tolerant recursive-descent parser producing a lossless green tree.
//!
//! Always produces a valid tree — errors go to a side channel. The central
//! invariant is: `parse(s).syntax().text() == s` for every input.

use rowan::GreenNodeBuilder;

use crate::lexer::{Token, lex_with};
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

impl ParseError {
    /// Compute 1-based line and column from the byte offset and source text.
    #[must_use]
    pub fn line_col(&self, source: &str) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for (i, ch) in source.char_indices() {
            if i >= self.offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// Format the error with source context, showing line/column and a caret.
    #[must_use]
    pub fn display(&self, source: &str) -> String {
        use std::fmt::Write;
        let (line, col) = self.line_col(source);
        let source_line = source.split('\n').nth(line - 1).unwrap_or("");
        let mut out = String::new();
        let _ = writeln!(out, "INI parse error at line {line}, column {col}");
        let _ = writeln!(out, "  |");
        let _ = writeln!(out, "{line:>3} | {source_line}");
        let _ = writeln!(out, "  | {}^", " ".repeat(col - 1));
        let _ = write!(out, "  = {}", self.message);
        out
    }
}

/// Options for configuring the parser.
#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// When `true`, keys without a `=` or `:` separator are accepted without
    /// producing an error. Their [`Entry::value()`](crate::ast::Entry::value)
    /// returns `None` (as opposed to `Some("")` for `key =`).
    ///
    /// This supports MySQL-style flag keys:
    ///
    /// ```ini
    /// [mysqldump]
    /// quick
    /// quote-names
    /// max_allowed_packet = 64M
    /// ```
    pub allow_no_value: bool,

    /// When `true`, a `;`/`#` marker on an entry line that is preceded by
    /// whitespace (and is not at the start of the value) begins a trailing
    /// inline comment instead of being part of the value. The comment is
    /// exposed via [`Entry::inline_comment()`](crate::ast::Entry::inline_comment).
    ///
    /// The rule is whitespace-adjacency, which preserves markers that are not
    /// whitespace-separated and markers at the value start:
    ///
    /// ```ini
    /// retain = 1            ; comment, value is "1"
    /// url    = http://x/#f  ; comment, value is "http://x/#f"
    /// conn   = a=1;b=2      ; comment, value is "a=1;b=2"
    /// color  = #fff         ; comment, value is "#fff"
    /// ```
    ///
    /// Disabled by default: the safest behavior for a lossless parser is to
    /// leave value bytes uninterpreted.
    pub inline_comments: bool,
}

/// Parse an INI source string.
#[must_use]
pub fn parse(input: &str) -> Parse {
    parse_with(input, &ParseOptions::default())
}

/// Parse an INI source string with custom options.
#[must_use]
pub fn parse_with(input: &str, options: &ParseOptions) -> Parse {
    let tokens = lex_with(input, options.inline_comments);
    let mut p = Parser {
        tokens,
        cursor: 0,
        builder: GreenNodeBuilder::new(),
        errors: Vec::new(),
        options,
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
    options: &'a ParseOptions,
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
            _ => {
                if !self.options.allow_no_value {
                    self.error("expected '=' or ':'");
                }
            }
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
        // A trailing inline comment (only emitted by the lexer when
        // `inline_comments` is enabled) belongs to the entry, before its newline.
        if self.peek() == Some(SyntaxKind::COMMENT) {
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

    #[test]
    fn utf8_bom_round_trips_without_errors() {
        let input = "\u{FEFF}[author]\nE-MAIL = u@gogs.io\n";
        let p = parse(input);
        assert_round_trip(input);
        assert!(p.errors().is_empty(), "BOM should not cause errors");
    }

    #[test]
    fn backslash_continuation_round_trips() {
        assert_round_trip("[s]\nk = hello \\\nworld\n");
        assert_round_trip("[s]\nk = a \\\nb \\\nc\nnext = val\n");
    }

    #[test]
    fn green_accessor() {
        let p = parse("[s]\nk=v\n");
        assert!(p.green().children().len() > 0);
    }

    #[test]
    fn unexpected_token_at_root() {
        // A stray `]` at root level triggers the error path.
        let p = parse("]\n[s]\nk=v\n");
        assert!(!p.errors().is_empty());
        assert_round_trip("]\n[s]\nk=v\n");
    }

    #[test]
    fn error_line_col() {
        let src = "[s]\nk=v\n[unclosed\n";
        let p = parse(src);
        let err = &p.errors()[0];
        let (line, col) = err.line_col(src);
        assert_eq!(line, 3);
        assert_eq!(col, 10); // after "[unclosed" (9 chars), expecting ']'
    }

    #[test]
    fn error_display_format() {
        let src = "[s]\nk=v\n[unclosed\n";
        let p = parse(src);
        let err = &p.errors()[0];
        let rendered = err.display(src);
        assert!(rendered.contains("line 3, column 10"));
        assert!(rendered.contains("[unclosed"));
        assert!(rendered.contains('^'));
        assert!(rendered.contains("expected ']'"));
    }

    fn inline() -> ParseOptions {
        ParseOptions {
            inline_comments: true,
            ..Default::default()
        }
    }

    #[test]
    fn inline_comment_round_trips() {
        let input = "[s]\nk = 1   ; note\nj = 2\n";
        let p = parse_with(input, &inline());
        assert_eq!(p.syntax().text().to_string(), input);
        assert!(p.errors().is_empty(), "errors: {:?}", p.errors());
    }

    #[test]
    fn inline_comment_round_trips_real_world() {
        let input = "[boot]\nBootproject.RetainMismatch.Init=1   ; HANDLES MISMATCHES\n";
        let p = parse_with(input, &inline());
        assert_eq!(p.syntax().text().to_string(), input);
        assert!(p.errors().is_empty(), "errors: {:?}", p.errors());
    }

    #[test]
    fn inline_comment_default_off_round_trips_into_value() {
        // With the option off, the marker stays in the value but still round-trips.
        let input = "[s]\nk = 1   ; note\n";
        let p = parse(input);
        assert_eq!(p.syntax().text().to_string(), input);
        assert!(p.errors().is_empty());
    }
}
