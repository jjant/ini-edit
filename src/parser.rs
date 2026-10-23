//! Error-tolerant recursive-descent parser producing a lossless green tree.
//!
//! Any input of at most [`MAX_INPUT_LEN`] bytes produces a valid tree —
//! errors go to a side channel. The central invariant is:
//! `parse(text).syntax().text() == text` for every such input. A longer input
//! panics; see [`parse_with`].

use rowan::GreenNodeBuilder;

use crate::MAX_INPUT_LEN;
use crate::lexer::{BOM, Token, lex_with, line_break_len, line_len};
use crate::syntax_kind::{SyntaxKind, SyntaxNode};

/// Parse an INI source string.
///
/// Every [`ParseOptions`] flag is off, as in `ParseOptions::default()`.
///
/// # Panics
///
/// Panics if `input` is longer than [`MAX_INPUT_LEN`] bytes, like
/// [`parse_with`].
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
///
/// # Panics
///
/// Panics if `input` is longer than [`MAX_INPUT_LEN`] bytes (`u32::MAX`), the
/// most text a `rowan` tree can measure. The check comes before any lexing,
/// so a longer input fails at once, the same way in debug and release builds.
#[must_use]
pub fn parse_with(input: &str, options: &ParseOptions) -> Parse {
    // The precondition, before any lexing. rowan measures text in u32, so a
    // longer input would be lexed in full and then overflow inside rowan in
    // debug builds, or wrap the tree's length in release builds. A caller
    // can break it, and `parse_panics_one_byte_past_max_input_len` does, so
    // unlike the internal checks it carries a message.
    assert!(
        fits_text_size(input.len()),
        "ini-edit parses at most MAX_INPUT_LEN ({MAX_INPUT_LEN}) bytes, got {}",
        input.len()
    );
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
    // Paired with the lexer's checks: every token was consumed once, in
    // order, and the offsets cover the input.
    assert_eq!(parser.cursor, parser.tokens.len());
    assert_eq!(parser.offset, input.len());
    // Diagnostics are in source order (see `Parser::error`), so the last one
    // bounds them all.
    assert!(
        parser
            .errors
            .last()
            .is_none_or(|error| error.offset <= input.len())
    );
    let green = parser.builder.finish();
    // The tree covers the input. Paired with the precondition, which keeps
    // its length within the u32 that rowan measures it in.
    assert_eq!(usize::from(green.text_len()), input.len());
    // O(n) checks, so debug builds only.
    #[cfg(all(debug_assertions, not(miri)))]
    {
        let root = SyntaxNode::new_root(green.clone());
        // The central invariant: the tree reproduces the input.
        assert_eq!(root.text(), input);
        // Every physical line is one node that owns only its own line break,
        // as editor line indices require. The editor checks it again when
        // it hands out its tree.
        crate::editor::debug_assert_line_nodes(&root);
    }
    Parse {
        green,
        errors: parser.errors,
    }
}

/// Whether rowan can measure a text of `len` bytes: its `TextSize` is a
/// `u32`, so this means `len <= MAX_INPUT_LEN`. Kept out of the assertion so
/// that tests can check both sides of the limit without allocating, and so
/// that cargo-mutants, which does not mutate assertion conditions, mutates
/// it.
///
/// Not written as that comparison: on 32-bit targets `MAX_INPUT_LEN` is
/// `usize::MAX`, so it is always true, and clippy's deny-by-default
/// `absurd_extreme_comparisons` rejects it.
fn fits_text_size(len: usize) -> bool {
    u32::try_from(len).is_ok()
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

    /// Recoverable errors, in source order and at most one per offset. Even
    /// when non-empty the tree still round-trips.
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
    #[expect(
        clippy::missing_panics_doc,
        reason = "line and column start at 1 and only grow or return to 1"
    )]
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
        // Both count from 1. Paired with `source_line` and `display`, which
        // rely on it.
        assert!(line >= 1);
        assert!(column >= 1);
        (line, column)
    }

    /// Format the error with source context, showing line/column and a caret.
    ///
    /// The caret sits under the reported column. Every gutter is as wide as
    /// the line number, and the padding before the caret repeats each tab of
    /// the source line, which a terminal expands to the same tab stop in both
    /// rows, and turns every other character into a space. Columns count
    /// characters, not terminal cells, so characters that are not one cell
    /// wide (wide East Asian, zero-width or combining) shift the caret by the
    /// difference, unless a tab between them and the column realigns both rows.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "line_col counts from 1, and the line holds every character before the column"
    )]
    pub fn display(&self, source: &str) -> String {
        let (line, column) = self.line_col(source);
        // The padding below is the `column - 1` characters before the column.
        assert!(column >= 1);
        let source_line = source_line(source, line);
        let padding: String = source_line
            .chars()
            .take(column - 1)
            .map(|character| if character == '\t' { '\t' } else { ' ' })
            .collect();
        // Paired with `line_col`: the line holds every character before the
        // column, so the caret does not fall short of it. The padding is
        // ASCII, so its length counts characters.
        assert_eq!(padding.len(), column - 1);
        let number = line.to_string();
        let gutter = " ".repeat(number.len());
        format!(
            "INI parse error at line {line}, column {column}\n\
             {gutter} |\n\
             {number} | {source_line}\n\
             {gutter} | {padding}^\n\
             {gutter} = {message}",
            message = self.message,
        )
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
    // Lines count from 1, as `line_col` reports them. Line 0 would silently
    // name the first line.
    assert!(target_line >= 1);
    let mut rest = &source[bom_len(source)..];
    for _ in 1..target_line {
        let line_break_start = line_len(rest);
        let newline_len = line_break_len(&rest[line_break_start..]);
        // display() obtains target_line from line_col() over this same
        // source, so the requested line always exists, including the empty
        // line after a final newline: every earlier line has a line break.
        assert!(newline_len > 0);
        rest = &rest[line_break_start + newline_len..];
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
            let section_start = self.cursor;
            self.parse_section();
            // A section owns at least its header's `[`, so the loop runs at
            // most once per token.
            assert!(self.cursor > section_start);
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
            let line_start = self.cursor;
            match self.line_kind() {
                LineKind::Section => break,
                LineKind::Blank => self.parse_blank_line(),
                LineKind::Comment => self.parse_comment_line(),
                LineKind::Entry => self.parse_entry(),
                LineKind::Error => self.parse_error_line(),
            }
            // Every line consumes a token, so the loop runs at most once per
            // token.
            assert!(self.cursor > line_start);
            // Paired with the lexer's check: every line node but the last
            // ends with its own line break.
            if !self.at_end() {
                assert_eq!(self.tokens[self.cursor - 1].kind, SyntaxKind::NEWLINE);
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
        let named = self.peek() == Some(SyntaxKind::IDENT);
        self.skip(SyntaxKind::IDENT);
        self.skip(SyntaxKind::WHITESPACE);
        let closed = self.peek() == Some(SyntaxKind::R_BRACK);
        // One diagnostic per position. The lexer emits whitespace after the
        // `[` as one token, so without a name the `]` or the line break comes
        // next: a header missing both lacks them at one offset.
        let missing = match (named, closed) {
            (true, true) => None,
            (false, true) => Some("expected section name"),
            (true, false) => Some("expected ']'"),
            (false, false) => Some("expected section name and ']'"),
        };
        if let Some(message) = missing {
            self.error(message);
        }
        if closed {
            self.bump();
        } else if !self.at_end() {
            // Negative space: without a `]`, the lexer runs the name to the
            // line break, so nothing else is on the line.
            assert_eq!(self.peek(), Some(SyntaxKind::NEWLINE));
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
        if let Some(SyntaxKind::EQ | SyntaxKind::COLON) = self.peek() {
            self.bump();
            self.skip(SyntaxKind::WHITESPACE);
            // Negative space: the lexer lexes all text after a separator as
            // the value, so unexpected text follows only a bare key.
            assert_ne!(self.peek(), Some(SyntaxKind::LEX_ERROR));
        } else {
            // Negative space: the lexer starts a value only after a
            // separator, so a bare key's value node stays empty.
            assert_ne!(self.peek(), Some(SyntaxKind::VALUE_TEXT));
            // One diagnostic per position. A missing separator is reported
            // where any text after the key (`key junk`) starts, so that text
            // gets its own message only where bare keys are allowed.
            if !self.options.allow_no_value {
                self.error("expected '=' or ':'");
            } else if self.peek() == Some(SyntaxKind::LEX_ERROR) {
                self.error("unexpected text after key");
            }
            // Text after a key without a separator is lexed as one error
            // token. Keep it on the entry's line, before its empty value, so
            // assigning a value replaces it.
            self.skip(SyntaxKind::LEX_ERROR);
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
        // Only the last line of the input may end without a line break.
        if !self.at_end() {
            self.expect_token(SyntaxKind::NEWLINE);
        }
        self.builder.finish_node();
    }

    /// Parse a line that begins with an unexpected token as a line node.
    /// Records one error and keeps the whole physical line, so the tree still
    /// round-trips and editors can address it like any other line.
    fn parse_error_line(&mut self) {
        self.builder.start_node(SyntaxKind::ERROR_LINE.into());
        self.skip(SyntaxKind::WHITESPACE);
        // `line_kind` dispatches here only on a token that starts no other
        // kind of line, and the lexer lexes such a line as one `LEX_ERROR`.
        assert_eq!(self.peek(), Some(SyntaxKind::LEX_ERROR));
        // Any other first character starts a key, a comment or a header, so
        // the line starts with a separator that no key comes before.
        let text = self.tokens[self.cursor].text;
        let message = if text.starts_with('=') {
            "expected a key before '='"
        } else {
            assert!(text.starts_with(':'));
            "expected a key before ':'"
        };
        self.error(message);
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
        // Negative space: the lexer emits indentation as one token, and
        // `parse_root` consumed the BOM, so whitespace never follows it.
        assert_ne!(
            self.tokens.get(index).map(|token| token.kind),
            Some(SyntaxKind::WHITESPACE)
        );
        match self.tokens.get(index).map(|token| token.kind) {
            None | Some(SyntaxKind::NEWLINE) => LineKind::Blank,
            Some(SyntaxKind::COMMENT) => LineKind::Comment,
            Some(SyntaxKind::L_BRACK) => LineKind::Section,
            Some(SyntaxKind::IDENT) => LineKind::Entry,
            Some(_) => LineKind::Error,
        }
    }

    /// Report `message` at the current offset. Messages are fixed text: none
    /// is formatted from internal values such as a token kind's name, and
    /// each allocates only its own length.
    fn error(&mut self, message: &'static str) {
        // Re-scanning the consumed prefix for each diagnostic makes a file
        // with one error per line quadratic in its input size.
        let error = ParseError {
            message: message.to_owned(),
            offset: self.offset,
        };
        // At most one diagnostic per position, in source order. The offset
        // only grows, and where two problems meet at one offset (a header
        // without its name and `]`, text after a bare key) the caller reports
        // them as one.
        assert!(
            self.errors
                .last()
                .is_none_or(|previous| previous.offset < error.offset)
        );
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
        // Paired with the lexer's check: every token holds at least one byte,
        // so every bump advances the offset. The lexer asserts it in every
        // build, and as `assert!` this copy costs 2-9% of parse time, so
        // debug builds only.
        debug_assert_ne!(token.text, "");
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
        // Fixed upper bound: at most every remaining token is on this line,
        // so a `bump` that stopped advancing ends the loop instead of hanging
        // it, and the caller's progress assertion reports it.
        for _ in self.cursor..self.tokens.len() {
            let kind = self.tokens[self.cursor].kind;
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
    use crate::editor::{EditOptions, Editor};

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

    /// Malformed lines, each with its node kind, its one diagnostic with
    /// `allow_no_value` off and then on, and the diagnostic's byte offset in
    /// the line. These lines used to report `unexpected token: LEX_ERROR`,
    /// after `expected '=' or ':'` at the same offset for text after a key.
    const MALFORMED_LINES: [(&str, SyntaxKind, &str, &str, usize); 6] = {
        const EQUALS: &str = "expected a key before '='";
        const COLON: &str = "expected a key before ':'";
        const SEPARATOR: &str = "expected '=' or ':'";
        const TEXT: &str = "unexpected text after key";
        [
            ("=bad", SyntaxKind::ERROR_LINE, EQUALS, EQUALS, 0),
            (":", SyntaxKind::ERROR_LINE, COLON, COLON, 0),
            ("key junk", SyntaxKind::ENTRY, SEPARATOR, TEXT, 4),
            ("key\t\tjunk", SyntaxKind::ENTRY, SEPARATOR, TEXT, 5),
            ("my key = value", SyntaxKind::ENTRY, SEPARATOR, TEXT, 3),
            ("!include /etc/λ", SyntaxKind::ENTRY, SEPARATOR, TEXT, 9),
        ]
    };

    #[test]
    fn malformed_lines_are_single_line_nodes_with_one_diagnostic() {
        for malformed_line in MALFORMED_LINES {
            for header in ["", "[s]\n", "[s]\r"] {
                for indent in ["", " \t"] {
                    for ending in ["\n", "\r\n", "\r", ""] {
                        check_malformed_line(malformed_line, header, indent, ending);
                    }
                }
            }
        }
    }

    /// Parse a line of [`MALFORMED_LINES`] after `header` and `indent`, ended
    /// by `ending` and, when that is a line break, followed by an entry, under
    /// every parser-option combination. Check that the line is one node of
    /// its kind, with its one diagnostic.
    fn check_malformed_line(
        (line, kind, strict, lenient, offset_in_line): (&str, SyntaxKind, &str, &str, usize),
        header: &str,
        indent: &str,
        ending: &str,
    ) {
        let next = if ending.is_empty() { "" } else { "next=1\n" };
        let source = format!("{header}{indent}{line}{ending}{next}");
        // A header is the first line of its section.
        let index = usize::from(!header.is_empty());
        let offset = header.len() + indent.len() + offset_in_line;
        for allow_no_value in [false, true] {
            for inline_comments in [false, true] {
                let options = ParseOptions {
                    allow_no_value,
                    inline_comments,
                };
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
                assert_eq!(lines.len(), index + 1 + usize::from(!next.is_empty()));
                assert_eq!(lines[index].kind(), kind, "{source:?}");
                assert_eq!(
                    lines[index].text().to_string(),
                    format!("{indent}{line}{ending}")
                );

                let message = if allow_no_value { lenient } else { strict };
                let errors: Vec<_> = parsed
                    .errors()
                    .iter()
                    .map(|error| (error.message.as_str(), error.offset))
                    .collect();
                assert_eq!(errors, [(message, offset)], "{source:?}, {options:?}");
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
            let mut editor =
                Editor::new(&source, &ParseOptions::default(), &EditOptions::default()).unwrap();
            editor.section("first").unwrap().remove();
            assert_eq!(editor.finish(), "\u{FEFF}[second]\nb=2\n");
        }
    }

    #[test]
    fn bom_is_not_removed_with_a_preamble_entry() {
        let root = parse("\u{FEFF}key=value\n[section]\n").syntax();
        let entry = File::cast(root.clone())
            .unwrap()
            .preamble_entries()
            .next()
            .unwrap();
        let removed = root.green().remove_child(entry.syntax().index());
        assert_eq!(removed.to_string(), "\u{FEFF}[section]\n");
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
        // Without inline comments, `; note` is text after a bare key, which
        // is reported once, as the missing separator.
        assert_eq!(
            diagnostics(&implicit),
            [("expected '=' or ':'".to_string(), 5)]
        );
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
            assert_eq!(
                error.display(source),
                "INI parse error at line 3, column 10\n\
                 \x20 |\n\
                 3 | [unclosed\n\
                 \x20 |          ^\n\
                 \x20 = expected ']'",
                "source: {source:?}"
            );
        }
    }

    #[test]
    fn a_bom_is_not_an_error_column() {
        // Each expected column is the number of characters before the error
        // plus one, and the caret's padding keeps the tabs among them. A BOM
        // is zero-width, so neither the reported column nor the caret may
        // count it.
        let lines = [
            ("=bad", 1, ""),
            (" \t=bad", 3, " \t"),
            ("[λ] junk", 5, "    "),
            ("\t[日本", 5, "\t   "),
        ];
        for bom in ["", "\u{FEFF}"] {
            for (line, column, padding) in lines {
                for newline in ["\n", "\r\n", "\r", ""] {
                    let caret = format!("\n  | {padding}^\n");

                    let source = format!("{bom}{line}{newline}");
                    let error = parse(&source).errors()[0].clone();
                    assert_eq!(error.line_col(&source), (1, column), "{source:?}");
                    let rendered = error.display(&source);
                    assert!(rendered.contains(&format!("\n1 | {line}\n")), "{rendered}");
                    assert!(rendered.contains(&caret), "{rendered}");

                    // The document marker never affected later lines.
                    let source = format!("{bom}[s]\r\n{line}{newline}");
                    let error = parse(&source).errors()[0].clone();
                    assert_eq!(error.line_col(&source), (2, column), "{source:?}");
                    let rendered = error.display(&source);
                    assert!(rendered.contains(&format!("\n2 | {line}\n")), "{rendered}");
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
            assert!(error.display(source).contains("\n1 | k\u{FEFF}\n  |   ^\n"));
        }
        let error = ParseError {
            message: "problem".to_string(),
            offset: 3,
        };
        assert_eq!(error.line_col("\u{FEFF}"), (1, 1));
        assert!(error.display("\u{FEFF}").contains("\n1 | \n  | ^\n"));
    }

    #[test]
    fn source_line_handles_final_line() {
        assert_eq!(source_line("first\nlast", 2), "last");
    }

    #[test]
    #[should_panic(expected = "assertion failed: newline_len > 0")]
    fn source_line_rejects_a_line_past_the_end() {
        // line_col never reports this line for this source.
        let _ = source_line("only line", 2);
    }

    #[test]
    #[should_panic(expected = "assertion failed: target_line >= 1")]
    fn source_line_rejects_line_zero() {
        // Without the assertion: the first line.
        let _ = source_line("only line", 0);
    }

    #[test]
    fn parse_error_records_offset_after_leading_whitespace() {
        let parsed = parse("  =bad\n");
        assert_eq!(parsed.errors()[0].offset, 2);
    }

    #[test]
    fn error_offsets_follow_consumed_bytes_including_unicode_and_continuations() {
        // Expected offsets come from the source, independently of the token
        // cursor. This also checks errors at EOF, and a header that lacks
        // both its name and its `]` at one offset.
        for newline in ["\n", "\r\n", "\r"] {
            let prefix = format!(
                "\u{FEFF}; café{newline}[valid]{newline}key = 日本 \\{newline}continued{newline}"
            );
            let source = format!("{prefix}  =bad{newline}[{newline}裸");
            let parsed = parse(&source);
            assert_eq!(parsed.syntax().text().to_string(), source);
            let offsets: Vec<_> = parsed.errors().iter().map(|error| error.offset).collect();
            let missing_name = source.rfind('[').unwrap() + 1;
            assert_eq!(offsets, [prefix.len() + 2, missing_name, source.len()]);
        }
    }

    #[test]
    fn a_header_reports_what_it_lacks_in_one_diagnostic() {
        // A header without a name has nothing between its `[` and the `]` or
        // the line break, so a missing name and a missing `]` are at one
        // offset. It used to report "expected section name" and "expected
        // ']'" there, two diagnostics for one position.
        let cases = [
            ("[s]", None),
            ("[ s \t]", None),
            ("[]", Some(("expected section name", 1))),
            ("[ \t]", Some(("expected section name", 3))),
            ("[s", Some(("expected ']'", 2))),
            ("[s \t", Some(("expected ']'", 4))),
            ("[", Some(("expected section name and ']'", 1))),
            ("[ \t", Some(("expected section name and ']'", 3))),
        ];
        for (header, expected) in cases {
            for indent in ["", " \t"] {
                for ending in ["\n", "\r\n", "\r", ""] {
                    let source = format!("{indent}{header}{ending}");
                    let parsed = parse(&source);
                    assert_eq!(parsed.syntax().text().to_string(), source);
                    let errors: Vec<_> = parsed
                        .errors()
                        .iter()
                        .map(|error| (error.message.as_str(), error.offset - indent.len()))
                        .collect();
                    assert_eq!(errors, expected.as_slice(), "{source:?}");
                }
            }
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
            assert_eq!(error.message, "expected a key before '='");
        }
    }

    #[test]
    fn error_display_handles_final_and_missing_source_lines() {
        let final_line = ParseError {
            message: "problem".to_string(),
            offset: 4,
        };
        let rendered = final_line.display("one\nlast");
        assert!(rendered.contains("\n2 | last\n  | ^\n"), "{rendered}");

        let past_end = ParseError {
            message: "problem".to_string(),
            offset: usize::MAX,
        };
        let rendered = past_end.display("one\n");
        assert!(rendered.contains("\n2 | \n  | ^\n"), "{rendered}");
    }

    #[test]
    fn error_display_format() {
        let source = "[s]\nk=v\n[unclosed\n";
        let parsed = parse(source);
        let error = &parsed.errors()[0];
        // The caret is under the column, just past "[unclosed", and every
        // gutter bar lines up. It used to be two columns to the left, under
        // the 'e', with the source row's bar two columns to the right.
        assert_eq!(
            error.display(source),
            "INI parse error at line 3, column 10\n\
             \x20 |\n\
             3 | [unclosed\n\
             \x20 |          ^\n\
             \x20 = expected ']'"
        );
    }

    /// Check the rendering of `\tkey junk` on `line`: the gutter is as wide
    /// as the line number, so the caret row starts its text where the source
    /// row does, and the caret is under column 6, the missing separator.
    fn assert_caret_under_the_column_on_line(line: usize) {
        let source = format!("{}\tkey junk\n", "\n".repeat(line - 1));
        let parsed = parse(&source);
        assert_eq!(parsed.errors().len(), 1);
        let gutter = " ".repeat(line.to_string().len());
        // The tab before the key stays a tab in the padding, so it expands
        // to the same tab stop in both rows.
        assert_eq!(
            parsed.errors()[0].display(&source),
            format!(
                "INI parse error at line {line}, column 6\n\
                 {gutter} |\n\
                 {line} | \tkey junk\n\
                 {gutter} | \t    ^\n\
                 {gutter} = expected '=' or ':'"
            )
        );
    }

    #[test]
    fn the_caret_is_under_the_column_for_line_numbers_of_one_to_four_digits() {
        // A fixed three-digit gutter put the caret two columns left of the
        // column below line 1000, and three from line 1000 to 9999.
        for line in [1, 9, 10, 99, 100, 999, 1000] {
            assert_caret_under_the_column_on_line(line);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "parses ten thousand lines")]
    fn the_caret_is_under_the_column_for_a_five_digit_line_number() {
        // A fixed three-digit gutter put the caret one column further left
        // for each digit after the third, so four columns left here.
        assert_caret_under_the_column_on_line(10_000);
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

    // The tests below parse token streams that the lexer never produces. The
    // parser panics instead of building a tree that breaks a guarantee of
    // the lexer.

    fn parse_tokens(tokens: &[(SyntaxKind, &str)]) {
        let options = ParseOptions::default();
        let mut parser = Parser {
            tokens: tokens
                .iter()
                .map(|&(kind, text)| Token { kind, text })
                .collect(),
            cursor: 0,
            offset: 0,
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
            options: &options,
        };
        parser.parse_root();
    }

    #[test]
    #[should_panic(expected = "left: Some(R_BRACK)\n right: Some(LEX_ERROR)")]
    fn an_error_line_starts_with_unexpected_text() {
        // Without the assertion: an error line for a token that the lexer
        // never starts one with, which the separator check below it rejects
        // without naming the token.
        parse_tokens(&[(SyntaxKind::R_BRACK, "]")]);
    }

    #[test]
    #[should_panic(expected = "assertion failed: text.starts_with(':')")]
    fn an_error_line_starts_with_a_separator() {
        // Without the assertion: "expected a key before ':'" on a line
        // without one.
        parse_tokens(&[(SyntaxKind::LEX_ERROR, "junk")]);
    }

    #[test]
    #[should_panic(expected = "left: Some(WHITESPACE)\n right: Some(WHITESPACE)")]
    fn indentation_is_one_token() {
        // Without the assertion: an error line that starts with whitespace,
        // which `parse_error_line` rejects without naming the cause.
        parse_tokens(&[
            (SyntaxKind::WHITESPACE, " "),
            (SyntaxKind::WHITESPACE, "\t"),
            (SyntaxKind::IDENT, "key"),
        ]);
    }

    #[test]
    #[should_panic(expected = "left: Some(VALUE_TEXT)\n right: Some(VALUE_TEXT)")]
    fn a_value_follows_a_separator() {
        // Without the assertion: a bare key whose value node holds text,
        // which `Entry::value` reports as `None`.
        parse_tokens(&[
            (SyntaxKind::IDENT, "key"),
            (SyntaxKind::VALUE_TEXT, "value"),
        ]);
    }

    #[test]
    #[should_panic(expected = "left: IDENT\n right: NEWLINE")]
    fn a_line_node_ends_at_its_line_break() {
        // Without the assertion: two entry nodes on one physical line, which
        // editor line indices cannot address.
        parse_tokens(&[(SyntaxKind::IDENT, "first"), (SyntaxKind::IDENT, "second")]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "left: \"\"\n right: \"\"")]
    fn every_token_holds_text() {
        // Without the assertion: an entry line node that holds no text.
        parse_tokens(&[(SyntaxKind::IDENT, "")]);
    }

    #[test]
    #[should_panic(expected = "left: Some(LEX_ERROR)\n right: Some(LEX_ERROR)")]
    fn no_unexpected_text_follows_a_separator() {
        // Without the assertion: text after a separator outside the value,
        // which the entry leaves to a second node on its line.
        parse_tokens(&[
            (SyntaxKind::IDENT, "key"),
            (SyntaxKind::EQ, "="),
            (SyntaxKind::LEX_ERROR, "junk"),
        ]);
    }

    #[test]
    #[should_panic(expected = "left: Some(LEX_ERROR)\n right: Some(NEWLINE)")]
    fn a_header_without_its_bracket_ends_at_its_name() {
        // Without the assertion: both "expected ']'" and "unexpected text
        // after section header" for the same position, which `error` rejects
        // without naming the cause.
        parse_tokens(&[
            (SyntaxKind::L_BRACK, "["),
            (SyntaxKind::IDENT, "name"),
            (SyntaxKind::LEX_ERROR, "junk"),
        ]);
    }

    /// Report a diagnostic at byte `offset` after one at byte 5, and return
    /// the offsets of the diagnostics.
    fn error_after_one_at_byte_5(offset: usize) -> Vec<usize> {
        let options = ParseOptions::default();
        let mut parser = Parser {
            tokens: Vec::new(),
            cursor: 0,
            offset,
            builder: GreenNodeBuilder::new(),
            errors: vec![ParseError {
                message: "first".to_string(),
                offset: 5,
            }],
            options: &options,
        };
        parser.error("second");
        parser.errors.iter().map(|error| error.offset).collect()
    }

    #[test]
    #[should_panic(expected = "assertion failed: self.errors")]
    fn error_rejects_a_diagnostic_before_the_previous_one() {
        // Without the assertion: diagnostics out of source order.
        error_after_one_at_byte_5(0);
    }

    #[test]
    #[should_panic(expected = "assertion failed: self.errors")]
    fn error_rejects_a_second_diagnostic_at_the_same_offset() {
        // Without the assertion: two diagnostics for one position, as `key
        // junk` and a lone `[` used to report.
        error_after_one_at_byte_5(5);
    }

    #[test]
    fn error_accepts_a_diagnostic_after_the_previous_one() {
        assert_eq!(error_after_one_at_byte_5(6), [5, 6]);
    }

    // The tests below check both sides of the input size limit. The first
    // three check, without allocating, the predicate that `parse_with`
    // asserts first. A 32-bit target has no length past the limit (there it
    // is `usize::MAX`), so only the third of them runs there.

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn an_input_of_max_input_len_bytes_fits() {
        assert!(fits_text_size(MAX_INPUT_LEN));
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn an_input_one_byte_past_max_input_len_does_not_fit() {
        // Without the check, parse_with lexed such an input in full before
        // rowan's u32 lengths overflowed: inside rowan in debug builds, and
        // into a tree whose length wrapped around in release builds.
        assert!(!fits_text_size(MAX_INPUT_LEN + 1));
    }

    #[test]
    fn the_input_size_limit_is_the_range_of_rowan_text_sizes() {
        // On 32-bit targets, the saturated length past the limit is
        // `usize::MAX`, which fits.
        for len in [
            0,
            1,
            MAX_INPUT_LEN - 1,
            MAX_INPUT_LEN,
            MAX_INPUT_LEN.saturating_add(1),
            usize::MAX,
        ] {
            let measurable = rowan::TextSize::try_from(len).is_ok();
            assert_eq!(fits_text_size(len), measurable, "{len}");
        }
    }

    // Linux backs a large zeroed allocation with the shared zero page until
    // it is written, so this 4 GiB input costs about a second of UTF-8
    // validation and a few MiB of memory. Miri would interpret every byte.
    #[test]
    #[cfg(all(target_os = "linux", target_pointer_width = "64", not(miri)))]
    #[should_panic(
        expected = "ini-edit parses at most MAX_INPUT_LEN (4294967295) bytes, got 4294967296"
    )]
    fn parse_panics_one_byte_past_max_input_len() {
        let input = String::from_utf8(vec![0; MAX_INPUT_LEN + 1]).unwrap();
        // With a predicate that accepted this length, parse would lex and
        // copy all 4 GiB. This fails the test at once instead: its message is
        // not the expected one.
        assert!(!fits_text_size(input.len()));
        // Without the assertion in parse_with: the 4 GiB input is lexed in
        // full, and rowan then panics inside text-size.
        drop(parse(&input));
    }
}
