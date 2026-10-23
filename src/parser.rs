//! Error-tolerant recursive-descent parser producing a lossless green tree.
//!
//! Always produces a valid tree — errors go to a side channel. The central
//! invariant is: `parse(s).syntax().text() == s` for every input.

use rowan::GreenNodeBuilder;

use crate::lexer::{BOM, Token, lex_with, line_break_len, line_len};
use crate::syntax_kind::{SyntaxKind, SyntaxNode};

/// Parse an INI source string.
///
/// Every [`ParseOptions`] flag is off, as in `ParseOptions::default()`.
#[must_use]
pub fn parse(input: &str) -> Parse {
    // Spelled out so that a change to the derived defaults cannot silently
    // change what this function accepts.
    let options = ParseOptions {
        allow_no_value: false,
        inline_comments: false,
    };
    parse_with(input, &options)
}

/// Parse an INI source string with custom options.
#[must_use]
pub fn parse_with(input: &str, options: &ParseOptions) -> Parse {
    let tokens = lex_with(input, options.inline_comments);
    let mut parser = Parser {
        tokens,
        cursor: 0,
        offset: 0,
        builder: GreenNodeBuilder::new(),
        errors: Vec::new(),
        options,
    };
    parser.parse_root();
    Parse {
        green: parser.builder.finish(),
        errors: parser.errors,
    }
}

/// Options for configuring the parser.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// A bare key accepted by [`allow_no_value`](Self::allow_no_value) can
    /// carry one too: `quick  # dump rows one at a time`.
    ///
    /// Disabled by default: the safest behavior for a lossless parser is to
    /// leave value bytes uninterpreted.
    pub inline_comments: bool,
}

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
    ///
    /// Columns count characters. A leading UTF-8 BOM is not a column.
    #[must_use]
    pub fn line_col(&self, source: &str) -> (usize, usize) {
        let mut line = 1;
        let mut column = 1;
        let mut previous_was_cr = false;
        let start = bom_len(source);
        for (index, character) in source[start..].char_indices() {
            if start + index >= self.offset {
                break;
            }
            match character {
                '\r' => {
                    line += 1;
                    column = 1;
                    previous_was_cr = true;
                }
                '\n' if previous_was_cr => {
                    previous_was_cr = false;
                }
                '\n' => {
                    line += 1;
                    column = 1;
                }
                _ => {
                    column += 1;
                    previous_was_cr = false;
                }
            }
        }
        (line, column)
    }

    /// Format the error with source context, showing line/column and a caret.
    #[must_use]
    pub fn display(&self, source: &str) -> String {
        use std::fmt::Write;
        let (line, column) = self.line_col(source);
        let source_line = source_line(source, line);
        let mut out = String::new();
        let _ = writeln!(out, "INI parse error at line {line}, column {column}");
        let _ = writeln!(out, "  |");
        let _ = writeln!(out, "{line:>3} | {source_line}");
        let _ = writeln!(out, "  | {}^", " ".repeat(column - 1));
        let _ = write!(out, "  = {}", self.message);
        out
    }
}

/// Byte length of a leading UTF-8 BOM. It identifies the encoding and is not
/// part of the first line, so diagnostics neither count nor display it.
fn bom_len(source: &str) -> usize {
    if source.starts_with(BOM) {
        BOM.len()
    } else {
        0
    }
}

/// The text of the 1-based line `target_line` of `source`, without its line
/// break. A leading BOM is not part of the first line.
fn source_line(source: &str, target_line: usize) -> &str {
    // display() obtains target_line from line_col() over this same source, so
    // the requested line always exists, including the empty line after a final
    // newline.
    let mut rest = &source[bom_len(source)..];
    for _ in 1..target_line {
        let line_break_start = line_len(rest);
        rest = &rest[line_break_start + line_break_len(&rest[line_break_start..])..];
    }
    &rest[..line_len(rest)]
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    cursor: usize,
    /// Byte offset of the next token, maintained once as tokens are consumed.
    offset: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<ParseError>,
    options: &'a ParseOptions,
}

/// The kind of physical line beginning at the parser cursor, determined by
/// looking past optional leading whitespace.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// Whitespace only (or empty) — a blank line.
    Blank,
    /// A full-line comment.
    Comment,
    /// A `[section]` header line.
    Section,
    /// A `key = value` entry line.
    Entry,
    /// A line starting with an unexpected token.
    Error,
}

impl Parser<'_> {
    fn parse_root(&mut self) {
        self.builder.start_node(SyntaxKind::ROOT.into());
        // The BOM identifies the document, not its first line. Keeping it at
        // the root also leaves just the indentation for line classification.
        if self.tokens.first().is_some_and(|token| token.text == BOM) {
            self.bump();
        }
        // Lines before the first header belong to the root. Every later line
        // belongs to the section of the header above it: the line loop stops
        // only at a header or at the end, so each iteration starts a section.
        self.parse_lines_until_header();
        while !self.at_end() {
            self.parse_section();
        }
        self.builder.finish_node();
    }

    /// Parse a section header line and the lines up to the next header.
    fn parse_section(&mut self) {
        self.builder.start_node(SyntaxKind::SECTION.into());
        self.parse_section_header();
        self.parse_lines_until_header();
        self.builder.finish_node();
    }

    /// Parse one node per line up to the next section header or the end of
    /// the input. The root and every section share this one line dispatch.
    fn parse_lines_until_header(&mut self) {
        while !self.at_end() {
            match self.line_kind() {
                LineKind::Section => break,
                LineKind::Blank => self.parse_blank_line(),
                LineKind::Comment => self.parse_comment_line(),
                LineKind::Entry => self.parse_entry(),
                LineKind::Error => self.parse_error_line(),
            }
        }
    }

    /// Parse a `[name]` header as a self-contained line node owning its leading
    /// indentation, any trailing content, and the terminating newline.
    fn parse_section_header(&mut self) {
        self.builder.start_node(SyntaxKind::SECTION_HEADER.into());
        self.skip(SyntaxKind::WHITESPACE);
        self.expect_token(SyntaxKind::L_BRACK);
        self.skip(SyntaxKind::WHITESPACE);
        if self.peek() == Some(SyntaxKind::IDENT) {
            self.bump();
        } else {
            self.error("expected section name");
        }
        self.skip(SyntaxKind::WHITESPACE);
        if self.peek() == Some(SyntaxKind::R_BRACK) {
            self.bump();
        } else {
            self.error("expected ']'");
        }
        // Retain malformed trailing text, but report it at its own offset.
        self.skip(SyntaxKind::WHITESPACE);
        if self.peek() == Some(SyntaxKind::LEX_ERROR) {
            self.error("unexpected text after section header");
        }
        // The header keeps a trailing comment or text and its line break.
        self.bump_rest_of_line();
        self.builder.finish_node();
    }

    /// Parse a `key = value` entry as a line node owning its leading
    /// indentation, optional trailing inline comment, and terminating newline.
    fn parse_entry(&mut self) {
        self.builder.start_node(SyntaxKind::ENTRY.into());
        self.skip(SyntaxKind::WHITESPACE);

        self.builder.start_node(SyntaxKind::KEY.into());
        self.expect_token(SyntaxKind::IDENT);
        self.builder.finish_node();

        // Whitespace before a bare key's inline comment follows the empty
        // value, as it does after any other value.
        if self.tokens.get(self.cursor + 1).map(|token| token.kind) != Some(SyntaxKind::COMMENT) {
            self.skip(SyntaxKind::WHITESPACE);
        }
        match self.peek() {
            Some(SyntaxKind::EQ | SyntaxKind::COLON) => {
                self.bump();
                self.skip(SyntaxKind::WHITESPACE);
            }
            _ => {
                if !self.options.allow_no_value {
                    self.error("expected '=' or ':'");
                }
            }
        }
        // Text after a key without a separator (`key junk`) is lexed as one
        // error token. Keep it on the entry's line, before its empty value, so
        // assigning a value replaces it.
        if self.peek() == Some(SyntaxKind::LEX_ERROR) {
            self.error(format!("unexpected token: {:?}", SyntaxKind::LEX_ERROR));
            self.bump();
        }

        self.builder.start_node(SyntaxKind::VALUE.into());
        self.skip(SyntaxKind::VALUE_TEXT);
        self.builder.finish_node();

        self.skip(SyntaxKind::WHITESPACE);
        // A trailing inline comment (only emitted by the lexer when
        // `inline_comments` is enabled) belongs to the entry, before its newline.
        self.skip(SyntaxKind::COMMENT);
        self.skip(SyntaxKind::NEWLINE);

        self.builder.finish_node();
    }

    /// Parse a full-line comment as a line node owning its newline.
    fn parse_comment_line(&mut self) {
        self.builder.start_node(SyntaxKind::COMMENT_LINE.into());
        self.skip(SyntaxKind::WHITESPACE);
        self.expect_token(SyntaxKind::COMMENT);
        self.skip(SyntaxKind::NEWLINE);
        self.builder.finish_node();
    }

    /// Parse a blank line (whitespace and/or a newline) as a line node.
    fn parse_blank_line(&mut self) {
        self.builder.start_node(SyntaxKind::BLANK_LINE.into());
        self.skip(SyntaxKind::WHITESPACE);
        self.skip(SyntaxKind::NEWLINE);
        self.builder.finish_node();
    }

    /// Parse a line that begins with an unexpected token as a line node.
    /// Records one error and keeps the whole physical line, so the tree still
    /// round-trips and editors can address it like any other line.
    fn parse_error_line(&mut self) {
        self.builder.start_node(SyntaxKind::ERROR_LINE.into());
        self.skip(SyntaxKind::WHITESPACE);
        let kind = self.peek().unwrap_or(SyntaxKind::LEX_ERROR);
        self.error(format!("unexpected token: {kind:?}"));
        self.bump_rest_of_line();
        self.builder.finish_node();
    }

    /// Classify the line at the cursor by peeking past optional leading
    /// whitespace, without consuming anything.
    fn line_kind(&self) -> LineKind {
        let mut index = self.cursor;
        if self.tokens.get(index).map(|token| token.kind) == Some(SyntaxKind::WHITESPACE) {
            index += 1;
        }
        match self.tokens.get(index).map(|token| token.kind) {
            None | Some(SyntaxKind::NEWLINE) => LineKind::Blank,
            Some(SyntaxKind::COMMENT) => LineKind::Comment,
            Some(SyntaxKind::L_BRACK) => LineKind::Section,
            Some(SyntaxKind::IDENT) => LineKind::Entry,
            Some(_) => LineKind::Error,
        }
    }

    fn error(&mut self, message: impl Into<String>) {
        // Re-scanning the consumed prefix for each diagnostic makes a file
        // with one error per line quadratic in its input size.
        let error = ParseError {
            message: message.into(),
            offset: self.offset,
        };
        self.errors.push(error);
    }

    fn at_end(&self) -> bool {
        self.cursor >= self.tokens.len()
    }

    fn peek(&self) -> Option<SyntaxKind> {
        self.tokens.get(self.cursor).map(|token| token.kind)
    }

    fn bump(&mut self) {
        let token = self.tokens[self.cursor];
        self.builder.token(token.kind.into(), token.text);
        self.cursor += 1;
        self.offset += token.text.len();
    }

    /// Bump the current token if it is `kind`. For optional tokens, such as
    /// indentation or a line break.
    fn skip(&mut self, kind: SyntaxKind) {
        if self.peek() == Some(kind) {
            self.bump();
        }
    }

    /// Bump the current token, which the line dispatch guarantees is `kind`:
    /// `line_kind` classified the line by this token.
    fn expect_token(&mut self, kind: SyntaxKind) {
        // No custom message: the pinned MC/DC toolchain reports the arguments
        // of a formatted message, which run only on failure, as uncovered.
        assert_eq!(self.peek(), Some(kind));
        self.bump();
    }

    /// Bump tokens up to and including the next newline (or end of input).
    fn bump_rest_of_line(&mut self) {
        while let Some(kind) = self.peek() {
            self.bump();
            if kind == SyntaxKind::NEWLINE {
                break;
            }
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::ast::{AstNode, File};
    use crate::editor::Editor;

    #[test]
    fn junk_after_a_section_header_is_reported_without_losing_text() {
        for prefix in ["[s]", " \t[λ]", "\u{FEFF}\t[s]"] {
            for whitespace in ["", " ", "\t "] {
                for junk in ["extra", "]", "key=value", "λ"] {
                    for ending in ["\n", "\r\n", "\r", ""] {
                        let source = format!("{prefix}{whitespace}{junk}{ending}");
                        let parsed = parse(&source);
                        assert_eq!(parsed.syntax().text().to_string(), source);
                        assert_eq!(parsed.errors().len(), 1, "{source:?}");
                        assert_eq!(
                            parsed.errors()[0].message,
                            "unexpected text after section header"
                        );
                        assert_eq!(
                            parsed.errors()[0].offset,
                            prefix.len() + whitespace.len(),
                            "{source:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn header_comments_remain_valid_and_junk_does_not_hide_following_entries() {
        for tail in ["", " ", "; note", " # note"] {
            let source = format!("[s]{tail}\nk=value\n");
            assert!(parse(&source).errors().is_empty(), "{source:?}");
        }

        let parsed = parse("[s] junk\nk=value\n[t] extra\nother=ok\n");
        assert_eq!(parsed.errors().len(), 2);
        let file = File::cast(parsed.syntax()).unwrap();
        let sections: Vec<_> = file.sections().collect();
        assert_eq!(sections.len(), 2);
        assert_eq!(
            sections[0].entries().next().unwrap().value().as_deref(),
            Some("value")
        );
        assert_eq!(
            sections[1].entries().next().unwrap().value().as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn malformed_lines_are_single_line_nodes_with_unchanged_diagnostics() {
        const SEPARATOR: &str = "expected '=' or ':'";
        const TOKEN: &str = "unexpected token: LEX_ERROR";
        // Each case is a line, its node kind, its diagnostics with
        // allow_no_value off and then on, and the column of its errors.
        let cases = [
            (
                "=bad",
                SyntaxKind::ERROR_LINE,
                &[TOKEN][..],
                &[TOKEN][..],
                0,
            ),
            (":", SyntaxKind::ERROR_LINE, &[TOKEN], &[TOKEN], 0),
            (
                "key junk",
                SyntaxKind::ENTRY,
                &[SEPARATOR, TOKEN],
                &[TOKEN],
                4,
            ),
            (
                "key\t\tjunk",
                SyntaxKind::ENTRY,
                &[SEPARATOR, TOKEN],
                &[TOKEN],
                5,
            ),
            (
                "my key = value",
                SyntaxKind::ENTRY,
                &[SEPARATOR, TOKEN],
                &[TOKEN],
                3,
            ),
            (
                "!include /etc/λ",
                SyntaxKind::ENTRY,
                &[SEPARATOR, TOKEN],
                &[TOKEN],
                9,
            ),
        ];
        for (line, kind, strict, lenient, column) in cases {
            for header in ["", "[s]\n", "[s]\r"] {
                for indent in ["", " \t"] {
                    for ending in ["\n", "\r\n", "\r", ""] {
                        for (allow_no_value, inline_comments) in
                            [(false, false), (false, true), (true, false), (true, true)]
                        {
                            let options = ParseOptions {
                                allow_no_value,
                                inline_comments,
                            };
                            let next = if ending.is_empty() { "" } else { "next=1\n" };
                            let source = format!("{header}{indent}{line}{ending}{next}");
                            let parsed = parse_with(&source, &options);
                            assert_eq!(parsed.syntax().text().to_string(), source);

                            let parent = if header.is_empty() {
                                parsed.syntax()
                            } else {
                                parsed.syntax().first_child().unwrap()
                            };
                            assert!(
                                parent
                                    .children_with_tokens()
                                    .all(|element| element.as_node().is_some()),
                                "{source:?}: every physical line must be one node"
                            );
                            let lines: Vec<_> = parent.children().collect();
                            let offset = usize::from(!header.is_empty());
                            assert_eq!(lines.len(), offset + 1 + usize::from(!next.is_empty()));
                            assert_eq!(lines[offset].kind(), kind, "{source:?}");
                            assert_eq!(
                                lines[offset].text().to_string(),
                                format!("{indent}{line}{ending}")
                            );

                            let expected = if allow_no_value { lenient } else { strict };
                            let messages: Vec<_> = parsed
                                .errors()
                                .iter()
                                .map(|error| error.message.as_str())
                                .collect();
                            assert_eq!(messages, expected, "{source:?}");
                            for error in parsed.errors() {
                                assert_eq!(error.offset, header.len() + indent.len() + column);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bare_keys_accept_inline_comments() {
        for header in ["", "[s]\n"] {
            for indent in ["", " \t"] {
                for gap in [" ", "\t", " \t "] {
                    for comment in [";", "# note", "; λ ; x"] {
                        for ending in ["\n", "\r\n", "\r", ""] {
                            for allow_no_value in [false, true] {
                                let options = ParseOptions {
                                    allow_no_value,
                                    inline_comments: true,
                                };
                                let next = if ending.is_empty() { "" } else { "next=1\n" };
                                let source =
                                    format!("{header}{indent}flag{gap}{comment}{ending}{next}");
                                let parsed = parse_with(&source, &options);
                                assert_eq!(parsed.syntax().text().to_string(), source);

                                let file = File::cast(parsed.syntax()).unwrap();
                                let entry = match file.sections().next() {
                                    Some(section) => section.entries().next(),
                                    None => file.preamble_entries().next(),
                                }
                                .unwrap();
                                assert_eq!(entry.key().as_deref(), Some("flag"));
                                assert_eq!(entry.value(), None);
                                assert_eq!(entry.inline_comment().as_deref(), Some(comment));
                                // The gap follows the empty value, like any
                                // whitespace before an inline comment.
                                let kinds: Vec<_> = entry
                                    .syntax()
                                    .children_with_tokens()
                                    .map(|element| element.kind())
                                    .collect();
                                let leading = usize::from(!indent.is_empty());
                                assert_eq!(
                                    kinds[leading..leading + 4],
                                    [
                                        SyntaxKind::KEY,
                                        SyntaxKind::VALUE,
                                        SyntaxKind::WHITESPACE,
                                        SyntaxKind::COMMENT
                                    ],
                                    "{source:?}"
                                );

                                let errors: Vec<_> = parsed
                                    .errors()
                                    .iter()
                                    .map(|error| (error.message.as_str(), error.offset))
                                    .collect();
                                if allow_no_value {
                                    assert!(errors.is_empty(), "{source:?}: {errors:?}");
                                } else {
                                    let offset = header.len() + indent.len() + "flag".len();
                                    assert_eq!(errors, [("expected '=' or ':'", offset)]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bom_does_not_hide_the_first_indented_line() {
        for indent in ["", " ", "\t", " \t"] {
            for newline in ["\n", "\r\n", "\r", ""] {
                for line in ["[section]", "key = value", "; comment", "# comment", ""] {
                    let source = format!("\u{FEFF}{indent}{line}{newline}");
                    let parsed = parse(&source);
                    assert_eq!(parsed.syntax().text().to_string(), source);
                    assert!(
                        parsed.errors().is_empty(),
                        "{source:?}: {:?}",
                        parsed.errors()
                    );
                    let file = File::cast(parsed.syntax()).unwrap();
                    match line {
                        "[section]" => assert_eq!(
                            file.sections().next().unwrap().name().as_deref(),
                            Some("section")
                        ),
                        "key = value" => assert_eq!(
                            file.preamble_entries().next().unwrap().value().as_deref(),
                            Some("value")
                        ),
                        _ => {}
                    }
                }
            }
        }
    }

    #[test]
    fn bom_belongs_to_the_document_when_removing_the_first_section() {
        for indent in ["", " \t"] {
            let source = format!("\u{FEFF}{indent}[first]\na=1\n[second]\nb=2\n");
            let editor = Editor::new(&source);
            editor.section("first").remove();
            assert_eq!(editor.finish(), "\u{FEFF}[second]\nb=2\n");
        }
    }

    #[test]
    fn bom_is_not_removed_with_a_preamble_entry() {
        let root = parse("\u{FEFF}key=value\n[section]\n")
            .syntax()
            .clone_for_update();
        File::cast(root.clone())
            .unwrap()
            .preamble_entries()
            .next()
            .unwrap()
            .syntax()
            .detach();
        assert_eq!(root.text().to_string(), "\u{FEFF}[section]\n");
    }

    #[test]
    fn parse_spells_out_the_default_options() {
        // parse() writes its options out instead of calling Default. Pin the
        // derived Default to the same values, so neither can drift alone.
        assert_eq!(
            ParseOptions::default(),
            ParseOptions {
                allow_no_value: false,
                inline_comments: false,
            }
        );
        // Each option changes how one of these lines parses.
        let source = "flag ; note\nkey = value ; note\n";
        let implicit = parse(source);
        let explicit = parse_with(source, &ParseOptions::default());
        assert_eq!(implicit.green(), explicit.green());
        let diagnostics = |parsed: &Parse| -> Vec<(String, usize)> {
            parsed
                .errors()
                .iter()
                .map(|error| (error.message.clone(), error.offset))
                .collect()
        };
        assert_eq!(diagnostics(&implicit), diagnostics(&explicit));
        assert_eq!(implicit.errors().len(), 2);
    }

    fn assert_round_trip(input: &str) {
        let parsed = parse(input);
        assert_eq!(parsed.syntax().text().to_string(), input);
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
        assert_round_trip("[unterminated");
        assert_round_trip("[]\n");
    }

    #[test]
    fn errors_reported() {
        let parsed = parse("[unclosed\n");
        assert!(!parsed.errors().is_empty());
    }

    #[test]
    fn root_kind() {
        assert_eq!(parse("").syntax().kind(), SyntaxKind::ROOT);
    }

    #[test]
    fn utf8_bom_round_trips_without_errors() {
        let input = "\u{FEFF}[author]\nE-MAIL = u@gogs.io\n";
        let parsed = parse(input);
        assert_round_trip(input);
        assert!(parsed.errors().is_empty(), "BOM should not cause errors");
    }

    #[test]
    fn backslash_continuation_round_trips() {
        assert_round_trip("[s]\nk = hello \\\nworld\n");
        assert_round_trip("[s]\nk = a \\\nb \\\nc\nnext = val\n");
    }

    #[test]
    fn green_accessor() {
        let parsed = parse("[s]\nk=v\n");
        assert!(parsed.green().children().len() > 0);
    }

    #[test]
    fn unexpected_token_at_root() {
        // A stray `]` at root level triggers the error path.
        let parsed = parse("]\n[s]\nk=v\n");
        assert!(!parsed.errors().is_empty());
        assert_round_trip("]\n[s]\nk=v\n");
    }

    #[test]
    fn error_line_col() {
        let source = "[s]\nk=v\n[unclosed\n";
        let parsed = parse(source);
        let error = &parsed.errors()[0];
        let (line, column) = error.line_col(source);
        assert_eq!(line, 3);
        // The error is after "[unclosed" (9 characters), where ']' is expected.
        assert_eq!(column, 10);
    }

    #[test]
    fn error_line_col_supports_all_line_endings() {
        for source in [
            "[s]\nk=v\n[unclosed\n",
            "[s]\r\nk=v\r\n[unclosed\r\n",
            "[s]\rk=v\r[unclosed\r",
        ] {
            let parsed = parse(source);
            let error = &parsed.errors()[0];
            assert_eq!(error.line_col(source), (3, 10), "source: {source:?}");
            let rendered = error.display(source);
            assert!(rendered.contains("line 3, column 10"), "{rendered}");
            assert!(rendered.contains("  3 | [unclosed"), "{rendered}");
        }
    }

    #[test]
    fn a_bom_is_not_an_error_column() {
        // Each expected column is the number of characters before the error
        // plus one. A BOM is zero-width, so neither the reported column nor
        // the caret may count it.
        let lines = [("=bad", 1), (" \t=bad", 3), ("[λ] junk", 5), ("\t[日本", 5)];
        for bom in ["", "\u{FEFF}"] {
            for (line, column) in lines {
                for newline in ["\n", "\r\n", "\r", ""] {
                    let caret = format!("\n  | {}^\n", " ".repeat(column - 1));

                    let source = format!("{bom}{line}{newline}");
                    let error = parse(&source).errors()[0].clone();
                    assert_eq!(error.line_col(&source), (1, column), "{source:?}");
                    let rendered = error.display(&source);
                    assert!(
                        rendered.contains(&format!("\n  1 | {line}\n")),
                        "{rendered}"
                    );
                    assert!(rendered.contains(&caret), "{rendered}");

                    // The document marker never affected later lines.
                    let source = format!("{bom}[s]\r\n{line}{newline}");
                    let error = parse(&source).errors()[0].clone();
                    assert_eq!(error.line_col(&source), (2, column), "{source:?}");
                    let rendered = error.display(&source);
                    assert!(
                        rendered.contains(&format!("\n  2 | {line}\n")),
                        "{rendered}"
                    );
                    assert!(rendered.contains(&caret), "{rendered}");
                }
            }
        }
    }

    #[test]
    fn only_a_leading_bom_is_skipped() {
        // U+FEFF after the start of the document is an ordinary character.
        for source in ["k\u{FEFF}\n", "\u{FEFF}k\u{FEFF}\n"] {
            let error = ParseError {
                message: "problem".to_string(),
                offset: source.len() - 1,
            };
            assert_eq!(error.line_col(source), (1, 3), "{source:?}");
            assert!(error.display(source).contains("\n  1 | k\u{FEFF}\n"));
        }
        let error = ParseError {
            message: "problem".to_string(),
            offset: 3,
        };
        assert_eq!(error.line_col("\u{FEFF}"), (1, 1));
        assert!(error.display("\u{FEFF}").contains("\n  1 | \n  | ^\n"));
    }

    #[test]
    fn source_line_handles_final_line() {
        assert_eq!(source_line("first\nlast", 2), "last");
    }

    #[test]
    fn parse_error_records_offset_after_leading_whitespace() {
        let parsed = parse("  =bad\n");
        assert_eq!(parsed.errors()[0].offset, 2);
    }

    #[test]
    fn error_offsets_follow_consumed_bytes_including_unicode_and_continuations() {
        // Expected offsets come from the source, independently of the token
        // cursor. This also checks errors at EOF and two errors at one offset.
        for newline in ["\n", "\r\n", "\r"] {
            let prefix = format!(
                "\u{FEFF}; café{newline}[valid]{newline}key = 日本 \\{newline}continued{newline}"
            );
            let source = format!("{prefix}  =bad{newline}[{newline}裸");
            let parsed = parse(&source);
            assert_eq!(parsed.syntax().text().to_string(), source);
            let offsets: Vec<_> = parsed.errors().iter().map(|error| error.offset).collect();
            let missing_name = source.rfind('[').unwrap() + 1;
            assert_eq!(
                offsets,
                [prefix.len() + 2, missing_name, missing_name, source.len()]
            );
        }
    }

    #[test]
    fn error_heavy_input_preserves_every_diagnostic() {
        let line = " \t=invalid 日本\r\n";
        let source = line.repeat(1024);
        let parsed = parse(&source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.errors().len(), 1024);
        for (index, error) in parsed.errors().iter().enumerate() {
            assert_eq!(error.offset, index * line.len() + 2);
            assert_eq!(error.message, "unexpected token: LEX_ERROR");
        }
    }

    #[test]
    fn error_display_handles_final_and_missing_source_lines() {
        let final_line = ParseError {
            message: "problem".to_string(),
            offset: 4,
        };
        let rendered = final_line.display("one\nlast");
        assert!(rendered.contains("  2 | last"), "{rendered}");

        let past_end = ParseError {
            message: "problem".to_string(),
            offset: usize::MAX,
        };
        let rendered = past_end.display("one\n");
        assert!(rendered.contains("  2 | "), "{rendered}");
    }

    #[test]
    fn error_display_format() {
        let source = "[s]\nk=v\n[unclosed\n";
        let parsed = parse(source);
        let error = &parsed.errors()[0];
        let rendered = error.display(source);
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
        let parsed = parse_with(input, &inline());
        assert_eq!(parsed.syntax().text().to_string(), input);
        assert!(parsed.errors().is_empty(), "errors: {:?}", parsed.errors());
    }

    #[test]
    fn inline_comment_round_trips_real_world() {
        let input = "[boot]\nBootproject.RetainMismatch.Init=1   ; HANDLES MISMATCHES\n";
        let parsed = parse_with(input, &inline());
        assert_eq!(parsed.syntax().text().to_string(), input);
        assert!(parsed.errors().is_empty(), "errors: {:?}", parsed.errors());
    }

    #[test]
    fn inline_comment_default_off_round_trips_into_value() {
        // With the option off, the marker stays in the value but still round-trips.
        let input = "[s]\nk = 1   ; note\n";
        let parsed = parse(input);
        assert_eq!(parsed.syntax().text().to_string(), input);
        assert!(parsed.errors().is_empty());
    }

    // The tests below check the line-node model: every logical line is one
    // node, which owns its line break.

    #[test]
    fn produces_line_nodes() {
        let parsed = parse("; top\n[s]\nk = v\n\n; tail\n");
        let root = parsed.syntax();
        let root_kinds: Vec<_> = root.children().map(|node| node.kind()).collect();
        assert_eq!(
            root_kinds,
            vec![SyntaxKind::COMMENT_LINE, SyntaxKind::SECTION]
        );
        let section = root
            .children()
            .find(|node| node.kind() == SyntaxKind::SECTION)
            .unwrap();
        let body: Vec<_> = section.children().map(|node| node.kind()).collect();
        assert_eq!(
            body,
            vec![
                SyntaxKind::SECTION_HEADER,
                SyntaxKind::ENTRY,
                SyntaxKind::BLANK_LINE,
                SyntaxKind::COMMENT_LINE,
            ]
        );
    }

    #[test]
    fn header_owns_newline_and_entry_captures_indent() {
        let parsed = parse("[s]\n\tk = v\n");
        let section = parsed.syntax().first_child().unwrap();
        let header = section.first_child().unwrap();
        assert_eq!(header.kind(), SyntaxKind::SECTION_HEADER);
        assert_eq!(header.last_token().unwrap().kind(), SyntaxKind::NEWLINE);

        let entry = section
            .children()
            .find(|node| node.kind() == SyntaxKind::ENTRY)
            .unwrap();
        let first = entry.first_token().unwrap();
        assert_eq!(first.kind(), SyntaxKind::WHITESPACE);
        assert_eq!(first.text(), "\t");
        assert_eq!(entry.last_token().unwrap().kind(), SyntaxKind::NEWLINE);
    }

    #[test]
    fn blank_lines_are_individual_nodes() {
        // Three consecutive newlines == three blank-line nodes.
        let parsed = parse("[s]\n\n\n\n");
        let section = parsed.syntax().first_child().unwrap();
        let blanks = section
            .children()
            .filter(|node| node.kind() == SyntaxKind::BLANK_LINE)
            .count();
        assert_eq!(blanks, 3);
    }

    #[test]
    fn error_line_inside_section_round_trips() {
        // A line starting with `=` lexes to a LEX_ERROR token, so the section
        // holds an error line.
        let input = "[s]\n=bad\nk = v\n";
        let parsed = parse(input);
        assert_eq!(parsed.syntax().text().to_string(), input);
        assert!(!parsed.errors().is_empty());
    }

    #[test]
    fn error_line_at_root_round_trips() {
        // Same, but in the preamble, where the root holds the error line.
        let input = "=oops\n[s]\nk = v\n";
        let parsed = parse(input);
        assert_eq!(parsed.syntax().text().to_string(), input);
        assert!(!parsed.errors().is_empty());
    }
}
