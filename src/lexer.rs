//! Line-aware lexer for INI files.
//!
//! INI is line-oriented and the RHS of an assignment can contain `=`, `[`,
//! `]`, etc. The lexer drives work per-line, dispatching on the first
//! non-whitespace character to decide whether it's a comment, section
//! header, or entry.
//!
//! The lexer is lossless: concatenating every `Token::text` reproduces the
//! original input exactly.

use crate::syntax_kind::SyntaxKind;

/// The UTF-8 byte order mark (U+FEFF). A document may start with one: it
/// identifies the encoding and is not part of the first line.
pub(crate) const BOM: &str = "\u{FEFF}";

// The lexer emits the BOM as one token of `BOM.len()` bytes: U+FEFF encoded
// as UTF-8.
const _: () = assert!(matches!(BOM.as_bytes(), [0xEF, 0xBB, 0xBF]));

/// A single lexed token — a zero-copy slice of the original input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    /// The syntactic category of this token.
    pub kind: SyntaxKind,
    /// The source text of this token.
    pub text: &'a str,
}

/// Lex an entire input string into a flat token stream.
///
/// Equivalent to [`lex_with`] with inline comments disabled.
#[must_use]
pub fn lex(input: &str) -> Vec<Token<'_>> {
    lex_with(input, false)
}

/// Lex an entire input string into a flat token stream, with options.
///
/// When `inline_comments` is `true`, a `;`/`#` marker on an entry line that is
/// preceded by whitespace (and is not at the start of the value) ends the value
/// and begins a trailing [`COMMENT`](SyntaxKind::COMMENT) token. Markers that
/// are not whitespace-adjacent (`a=1;b=2`) or that sit at the value start
/// (`color = #fff`) remain part of the value.
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "the assertions check lexer invariants, which hold for every input"
)]
pub fn lex_with(input: &str, inline_comments: bool) -> Vec<Token<'_>> {
    let mut lexer = Lexer {
        rest: input,
        tokens: Vec::new(),
        inline_comments,
    };
    // A leading BOM is trivia, so it becomes a whitespace token.
    if lexer.rest.starts_with(BOM) {
        lexer.bump(SyntaxKind::WHITESPACE, BOM.len());
    }
    while !lexer.rest.is_empty() {
        let rest_len = lexer.rest.len();
        lexer.lex_line();
        // Every line consumes input, so the loop runs at most `input.len()`
        // times. `eat_newline` checks that each line ends at its line break.
        assert!(lexer.rest.len() < rest_len);
    }
    // The tokens partition the input. Paired with the parser's offset check;
    // O(n), so debug builds only.
    #[cfg(all(debug_assertions, not(miri)))]
    assert_eq!(
        lexer
            .tokens
            .iter()
            .map(|token| token.text.len())
            .sum::<usize>(),
        input.len()
    );
    lexer.tokens
}

struct Lexer<'a> {
    /// The input that has not been lexed yet.
    rest: &'a str,
    tokens: Vec<Token<'a>>,
    inline_comments: bool,
}

impl Lexer<'_> {
    fn bump(&mut self, kind: SyntaxKind, len: usize) {
        // Every token holds at least one byte, which is what makes lexing
        // progress.
        assert!(len > 0);
        let (text, rest) = self.rest.split_at(len);
        // Negative space: only a line break or a continued value holds a line
        // break. O(token), so debug builds only.
        #[cfg(all(debug_assertions, not(miri)))]
        if !matches!(kind, SyntaxKind::NEWLINE | SyntaxKind::VALUE_TEXT) {
            assert_eq!(line_len(text), len);
        }
        self.tokens.push(Token { kind, text });
        self.rest = rest;
    }

    fn eat_horizontal_whitespace(&mut self) {
        let len = self
            .rest
            .bytes()
            .take_while(|&byte| byte == b' ' || byte == b'\t')
            .count();
        if len > 0 {
            self.bump(SyntaxKind::WHITESPACE, len);
        }
        // The whole run became one token. This restates the scan above, on
        // the hottest path, so debug builds only.
        debug_assert!(!self.rest.starts_with([' ', '\t']));
    }

    /// Consume the line break that ends a line. Every kind of line is lexed
    /// up to its line break or to the end of the input.
    fn eat_newline(&mut self) {
        let newline_len = line_break_len(self.rest);
        if newline_len > 0 {
            self.bump(SyntaxKind::NEWLINE, newline_len);
        } else {
            // A line without a line break is the last line.
            assert_eq!(self.rest, "");
        }
    }

    /// Lex one logical line: a physical line, or an entry line together with
    /// the lines its backslash-continued value runs onto. Each kind of line is
    /// lexed up to its last line break, which this function then consumes for
    /// all of them.
    fn lex_line(&mut self) {
        self.eat_horizontal_whitespace();
        let Some(first) = self.rest.chars().next() else {
            return;
        };
        match first {
            ';' | '#' => self.lex_comment(),
            '[' => self.lex_section_header_line(),
            _ => self.lex_entry_line(),
        }
        self.eat_newline();
    }

    /// Lex a `;` or `#` comment up to the line break.
    fn lex_comment(&mut self) {
        // Every caller dispatches on the marker. A restated dispatch, so
        // debug builds only.
        debug_assert!(self.rest.starts_with([';', '#']));
        self.bump(SyntaxKind::COMMENT, line_len(self.rest));
    }

    /// Lex a `[name]` header line up to its line break, including a trailing
    /// comment or unexpected text.
    fn lex_section_header_line(&mut self) {
        // `lex_line` dispatches headers on their `[`. A restated dispatch, so
        // debug builds only.
        debug_assert!(self.rest.starts_with('['));
        self.bump(SyntaxKind::L_BRACK, 1);
        self.eat_horizontal_whitespace();

        // The name runs up to `]` or the line break, and its trailing
        // whitespace is a separate token. One pass that stops at either is
        // faster than line_len followed by a search for `]`.
        let raw_name_len = self
            .rest
            .bytes()
            .take_while(|&byte| byte != b']' && !is_line_break(byte))
            .count();
        if raw_name_len > 0 {
            let name_len = self.rest[..raw_name_len]
                .trim_end_matches([' ', '\t'])
                .len();
            let trailing_whitespace_len = raw_name_len - name_len;
            // Leading horizontal whitespace was consumed above, so a non-empty
            // raw name always contains at least one non-whitespace byte.
            assert!(name_len > 0);
            self.bump(SyntaxKind::IDENT, name_len);
            if trailing_whitespace_len > 0 {
                self.bump(SyntaxKind::WHITESPACE, trailing_whitespace_len);
            }
        }

        if self.rest.starts_with(']') {
            self.bump(SyntaxKind::R_BRACK, 1);
        }

        // A comment may follow the header on its line. Any other text is an
        // error.
        self.eat_horizontal_whitespace();
        if self.rest.starts_with(';') || self.rest.starts_with('#') {
            self.lex_comment();
        } else {
            self.lex_rest_of_line_as_error();
        }
    }

    /// Lex an entry line, and the lines its value continues onto, up to the
    /// last line break: the key, then a separator and the value, a bare key's
    /// inline comment, or unexpected text. A line that starts with a separator
    /// has no key, so all of it is unexpected text. Blank lines come here too
    /// and produce no token before their line break.
    fn lex_entry_line(&mut self) {
        // `lex_line` dispatches every other line here, after its indentation.
        // A restated dispatch, so debug builds only.
        debug_assert!(!self.rest.starts_with([' ', '\t', ';', '#', '[']));
        let key_len = self
            .rest
            .bytes()
            .take_while(|&byte| {
                byte != b'='
                    && byte != b':'
                    && byte != b' '
                    && byte != b'\t'
                    && !is_line_break(byte)
            })
            .count();
        if key_len == 0 {
            self.lex_rest_of_line_as_error();
            return;
        }
        self.bump(SyntaxKind::IDENT, key_len);
        self.eat_horizontal_whitespace();

        match self.rest.chars().next() {
            Some('=') => self.bump(SyntaxKind::EQ, 1),
            Some(':') => self.bump(SyntaxKind::COLON, 1),
            // A key ends at whitespace, so a marker here is whitespace-preceded:
            // a bare key's inline comment (`flag ; note`).
            Some(';' | '#') if self.inline_comments => {
                self.lex_comment();
                return;
            }
            _ => {
                self.lex_rest_of_line_as_error();
                return;
            }
        }
        self.eat_horizontal_whitespace();

        let span_len = value_span_len(self.rest);
        if span_len > 0 {
            self.lex_value(span_len);
        }
    }

    /// Lex the `span_len` bytes of a value span (see [`value_span_len`]) as
    /// the value, then its trailing whitespace or inline comment.
    fn lex_value(&mut self, span_len: usize) {
        let span = &self.rest[..span_len];
        // Whitespace immediately after the separator was already consumed.
        // This restates the caller, so debug builds only.
        debug_assert!(!span.starts_with([' ', '\t']));

        // When inline comments are enabled, a whitespace-preceded `;`/`#`
        // marker on the final physical line ends the value and starts a
        // trailing comment.
        let inline_comment = if self.inline_comments {
            find_inline_comment(span)
        } else {
            None
        };

        if let Some((value_len, whitespace_len)) = inline_comment {
            // find_inline_comment only returns a marker after a non-empty
            // value and the whole run of horizontal whitespace before it, and
            // the comment starts with that marker.
            assert!(value_len > 0);
            assert!(whitespace_len > 0);
            assert!(!span[..value_len].ends_with([' ', '\t']));
            assert!(value_len + whitespace_len < span_len);
            let comment_len = span_len - value_len - whitespace_len;
            self.bump(SyntaxKind::VALUE_TEXT, value_len);
            self.bump(SyntaxKind::WHITESPACE, whitespace_len);
            self.bump(SyntaxKind::COMMENT, comment_len);
        } else {
            // Separate trailing whitespace from the last line of the value.
            let value_len = span.trim_end_matches([' ', '\t']).len();
            let trailing_whitespace_len = span_len - value_len;
            // The span starts with a non-whitespace byte, so it cannot trim to
            // empty.
            assert!(value_len > 0);
            self.bump(SyntaxKind::VALUE_TEXT, value_len);
            if trailing_whitespace_len > 0 {
                self.bump(SyntaxKind::WHITESPACE, trailing_whitespace_len);
            }
        }
    }

    fn lex_rest_of_line_as_error(&mut self) {
        let len = line_len(self.rest);
        if len > 0 {
            self.bump(SyntaxKind::LEX_ERROR, len);
        }
    }
}

/// Byte length of the value span at the start of `text`: the rest of its line
/// and, while a line ends with `\` (ignoring trailing spaces and tabs), the
/// line break after it and the next line. A continued value is therefore one
/// `VALUE_TEXT` token, line breaks included, which keeps it lossless.
fn value_span_len(text: &str) -> usize {
    let mut value_end = 0;
    loop {
        let line_break_start = value_end + line_len(&text[value_end..]);
        let line = &text[value_end..line_break_start];
        if line.trim_end_matches([' ', '\t']).ends_with('\\') {
            // The line break is part of the value, which continues on the
            // next line.
            let next_line_start = line_break_start + line_break_len(&text[line_break_start..]);
            // A continued line holds at least its backslash, so the loop runs
            // at most `text.len()` times.
            assert!(next_line_start > value_end);
            value_end = next_line_start;
        } else {
            return line_break_start;
        }
    }
}

/// Locate a trailing inline comment within a value span.
///
/// Scans the final physical line of `span` for the first run of spaces/tabs
/// that is immediately followed by a `;` or `#` marker and is not at the start
/// of that line. Returns `(value_len, whitespace_len)`: the byte length of the
/// value preceding the whitespace run, and the length of the whitespace run
/// itself. The comment spans from `value_len + whitespace_len` to the end of
/// `span`.
///
/// Returns `None` when there is no such marker, which leaves the whole span as
/// the value. This protects markers that are not whitespace-adjacent
/// (`a=1;b=2`), markers at the value start (`#fff`), and `;`/`#` on earlier
/// continued lines (only the final physical line is inspected).
fn find_inline_comment(span: &str) -> Option<(usize, usize)> {
    let bytes = span.as_bytes();
    // Only the final physical line of a (possibly continued) value is eligible.
    let final_line_start = span
        .rfind(['\n', '\r'])
        .map_or(0, |line_break| line_break + 1);

    let mut index = final_line_start;
    while index < bytes.len() {
        let step_start = index;
        if bytes[index] == b' ' || bytes[index] == b'\t' {
            let whitespace_start = index;
            index += bytes[index..]
                .iter()
                .take_while(|&&byte| byte == b' ' || byte == b'\t')
                .count();
            if index < bytes.len()
                && (bytes[index] == b';' || bytes[index] == b'#')
                && whitespace_start > final_line_start
            {
                return Some((whitespace_start, index - whitespace_start));
            }
        } else {
            index += 1;
        }
        // Every step advances, so the scan takes at most `bytes.len()` steps.
        assert!(index > step_start);
    }
    None
}

/// Byte length of the line at the start of `text`, up to but excluding its
/// line break (`\n`, `\r\n` or `\r`), or to the end of `text` if it has none.
pub(crate) fn line_len(text: &str) -> usize {
    text.bytes()
        .take_while(|&byte| !is_line_break(byte))
        .count()
}

/// Byte length of the line break at the start of `text`: 2 for `\r\n`, 1 for
/// `\n` or a bare `\r`, and 0 if `text` does not start with one.
pub(crate) fn line_break_len(text: &str) -> usize {
    if text.starts_with("\r\n") {
        2
    } else {
        usize::from(text.bytes().next().is_some_and(is_line_break))
    }
}

/// Whether `byte` is one of the two bytes that line breaks (`\n`, `\r\n` and
/// `\r`) are made of. Neither occurs inside a multi-byte UTF-8 character, so
/// scanning bytes finds every line break.
fn is_line_break(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use SyntaxKind::*;

    fn lex_kinds(input: &str) -> Vec<SyntaxKind> {
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input, "lexer must be lossless");
        tokens.iter().map(|token| token.kind).collect()
    }

    #[test]
    fn empty() {
        assert_eq!(lex(""), []);
    }

    #[test]
    fn blank_line() {
        assert_eq!(lex_kinds("\n"), vec![NEWLINE]);
    }

    #[test]
    fn whitespace_only_final_line() {
        let tokens = lex(" \t");
        assert_eq!(
            tokens,
            vec![Token {
                kind: WHITESPACE,
                text: " \t"
            }]
        );
    }

    #[test]
    fn comments() {
        assert_eq!(lex_kinds("; hi\n"), vec![COMMENT, NEWLINE]);
        assert_eq!(lex_kinds("# hi"), vec![COMMENT]);
        assert_eq!(lex_kinds("; hi\r"), vec![COMMENT, NEWLINE]);
    }

    #[test]
    fn section_header() {
        assert_eq!(lex_kinds("[foo]\n"), vec![L_BRACK, IDENT, R_BRACK, NEWLINE]);
    }

    #[test]
    fn section_header_with_trailing_comment() {
        assert_eq!(
            lex_kinds("[foo] ; note\n"),
            vec![L_BRACK, IDENT, R_BRACK, WHITESPACE, COMMENT, NEWLINE]
        );
    }

    #[test]
    fn section_with_spaces_in_name() {
        assert_eq!(
            lex_kinds("[my section]\n"),
            vec![L_BRACK, IDENT, R_BRACK, NEWLINE]
        );
        let tokens = lex("[my section]\n");
        assert_eq!(tokens[1].text, "my section");
    }

    #[test]
    fn section_header_boundaries() {
        assert_eq!(lex_kinds("[]\n"), vec![L_BRACK, R_BRACK, NEWLINE]);
        assert_eq!(
            lex_kinds("[ \t]\n"),
            vec![L_BRACK, WHITESPACE, R_BRACK, NEWLINE]
        );
        assert_eq!(
            lex_kinds("[name \t]\n"),
            vec![L_BRACK, IDENT, WHITESPACE, R_BRACK, NEWLINE]
        );
        assert_eq!(
            lex_kinds("[name] ; comment\r"),
            vec![L_BRACK, IDENT, R_BRACK, WHITESPACE, COMMENT, NEWLINE]
        );
        assert_eq!(
            lex_kinds("[name] # comment\n"),
            vec![L_BRACK, IDENT, R_BRACK, WHITESPACE, COMMENT, NEWLINE]
        );
        assert_eq!(
            lex_kinds("[name] junk\r"),
            vec![L_BRACK, IDENT, R_BRACK, WHITESPACE, LEX_ERROR, NEWLINE]
        );
        assert_eq!(lex_kinds("[unterminated\r"), vec![L_BRACK, IDENT, NEWLINE]);
    }

    #[test]
    fn entry_eq() {
        assert_eq!(lex_kinds("k=v\n"), vec![IDENT, EQ, VALUE_TEXT, NEWLINE]);
    }

    #[test]
    fn entry_colon() {
        assert_eq!(lex_kinds("k:v\n"), vec![IDENT, COLON, VALUE_TEXT, NEWLINE]);
    }

    #[test]
    fn entry_whitespace_preserved() {
        assert_eq!(
            lex_kinds("k = v \n"),
            vec![
                IDENT, WHITESPACE, EQ, WHITESPACE, VALUE_TEXT, WHITESPACE, NEWLINE
            ]
        );
        assert_eq!(
            lex_kinds("k\t=\tvalue\n"),
            vec![IDENT, WHITESPACE, EQ, WHITESPACE, VALUE_TEXT, NEWLINE]
        );
    }

    #[test]
    fn entry_lexer_consumes_its_trailing_whitespace_and_newline() {
        let mut lexer = Lexer {
            rest: "k=value  \nnext=x\n",
            tokens: Vec::new(),
            inline_comments: false,
        };
        lexer.lex_line();

        assert_eq!(lexer.rest, "next=x\n");
        assert_eq!(
            lexer.tokens,
            vec![
                Token {
                    kind: IDENT,
                    text: "k"
                },
                Token {
                    kind: EQ,
                    text: "="
                },
                Token {
                    kind: VALUE_TEXT,
                    text: "value"
                },
                Token {
                    kind: WHITESPACE,
                    text: "  "
                },
                Token {
                    kind: NEWLINE,
                    text: "\n"
                },
            ]
        );
    }

    #[test]
    fn empty_value() {
        assert_eq!(lex_kinds("k=\n"), vec![IDENT, EQ, NEWLINE]);
    }

    #[test]
    fn value_with_special_chars() {
        assert_eq!(lex_kinds("k=a=[b]\n"), vec![IDENT, EQ, VALUE_TEXT, NEWLINE]);
    }

    #[test]
    fn crlf() {
        let tokens = lex("k=v\r\n");
        assert_eq!(tokens.last().unwrap().text, "\r\n");
    }

    #[test]
    fn utf8_bom() {
        let input = "\u{FEFF}[author]\nE-MAIL = u@gogs.io\n";
        let tokens = lex(input);
        // The tokens reproduce the input.
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        // The BOM is emitted as whitespace (trivia), not as an error.
        assert_eq!(tokens[0].kind, WHITESPACE);
        assert_eq!(tokens[0].text, "\u{FEFF}");
        // Lexing continues normally after the BOM.
        assert_eq!(tokens[1].kind, L_BRACK);
    }

    #[test]
    fn backslash_continuation() {
        let input = "k=hello \\\nworld\n";
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        let value_token = tokens
            .iter()
            .find(|token| token.kind == VALUE_TEXT)
            .unwrap();
        assert_eq!(value_token.text, "hello \\\nworld");
    }

    #[test]
    fn backslash_continuation_multiple_lines() {
        let input = "k=a \\\nb \\\nc\n";
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        let value_token = tokens
            .iter()
            .find(|token| token.kind == VALUE_TEXT)
            .unwrap();
        assert_eq!(value_token.text, "a \\\nb \\\nc");
    }

    #[test]
    fn backslash_not_at_end_is_literal() {
        let input = "k=path\\to\\file\n";
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        let value_token = tokens
            .iter()
            .find(|token| token.kind == VALUE_TEXT)
            .unwrap();
        assert_eq!(value_token.text, "path\\to\\file");
    }

    #[test]
    fn backslash_continuation_with_crlf() {
        let input = "k=a \\\r\nb\r\n";
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        let value_token = tokens
            .iter()
            .find(|token| token.kind == VALUE_TEXT)
            .unwrap();
        assert_eq!(value_token.text, "a \\\r\nb");
    }

    #[test]
    fn backslash_continuation_with_bare_cr() {
        let input = "k=a \\\rb\r";
        let tokens = lex(input);
        assert_eq!(
            tokens.iter().map(|token| token.text).collect::<String>(),
            input
        );
        let value_token = tokens
            .iter()
            .find(|token| token.kind == VALUE_TEXT)
            .unwrap();
        assert_eq!(value_token.text, "a \\\rb");
    }

    #[test]
    fn backslash_at_eof_is_literal() {
        // No newline after backslash — it's just a literal backslash.
        let input = "k=val\\";
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        let value_token = tokens
            .iter()
            .find(|token| token.kind == VALUE_TEXT)
            .unwrap();
        assert_eq!(value_token.text, "val\\");
    }

    #[test]
    fn bare_cr_line_ending() {
        let input = "k=v\r[s]\rx=y\r";
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
    }

    #[test]
    fn line_without_separator() {
        // A bare identifier followed by more text but no = or : triggers LEX_ERROR
        // for the trailing content.
        let input = "=value_no_key\n";
        let tokens = lex(input);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        // The `=` isn't a valid start for a key, so the whole line becomes error.
        assert!(tokens.iter().any(|token| token.kind == LEX_ERROR));
        assert_eq!(
            lex_kinds("key\r"),
            vec![IDENT, NEWLINE],
            "a bare key followed by CR must terminate cleanly"
        );
        assert_eq!(
            lex_kinds("=bad\r"),
            vec![LEX_ERROR, NEWLINE],
            "error lines must stop at a bare CR"
        );
    }

    // The tests below cover inline comments, which are opt-in.

    fn value_text<'a>(tokens: &'a [Token<'a>]) -> Option<&'a str> {
        tokens
            .iter()
            .find(|token| token.kind == VALUE_TEXT)
            .map(|token| token.text)
    }
    fn comment_text<'a>(tokens: &'a [Token<'a>]) -> Option<&'a str> {
        tokens
            .iter()
            .find(|token| token.kind == COMMENT)
            .map(|token| token.text)
    }

    #[test]
    fn inline_comment_basic() {
        let tokens = lex_with("k = 1   ; note\n", true);
        assert_eq!(
            tokens.iter().map(|token| token.kind).collect::<Vec<_>>(),
            vec![
                IDENT, WHITESPACE, EQ, WHITESPACE, VALUE_TEXT, WHITESPACE, COMMENT, NEWLINE
            ]
        );
        assert_eq!(value_text(&tokens), Some("1"));
        assert_eq!(comment_text(&tokens), Some("; note"));
    }

    #[test]
    fn inline_comment_disabled_by_default() {
        let tokens = lex("k = 1   ; note\n");
        assert_eq!(value_text(&tokens), Some("1   ; note"));
        assert!(comment_text(&tokens).is_none());
    }

    #[test]
    fn inline_comment_requires_whitespace_before_marker() {
        // Connection-string style — no space before `;` keeps it in the value.
        let tokens = lex_with("conn = a=1;b=2;c=3\n", true);
        assert_eq!(value_text(&tokens), Some("a=1;b=2;c=3"));
        assert!(comment_text(&tokens).is_none());
    }

    #[test]
    fn inline_comment_marker_at_value_start_is_value() {
        let tokens = lex_with("color = #fff\n", true);
        assert_eq!(value_text(&tokens), Some("#fff"));
        assert!(comment_text(&tokens).is_none());
    }

    #[test]
    fn inline_comment_hash_after_value() {
        let tokens = lex_with("color = #fff ; my color\n", true);
        assert_eq!(value_text(&tokens), Some("#fff"));
        assert_eq!(comment_text(&tokens), Some("; my color"));
    }

    #[test]
    fn inline_comment_url_with_fragment_preserved() {
        let tokens = lex_with("url = http://example.com/#section\n", true);
        assert_eq!(value_text(&tokens), Some("http://example.com/#section"));
        assert!(comment_text(&tokens).is_none());
    }

    #[test]
    fn inline_comment_value_with_internal_spaces() {
        let tokens = lex_with("name = John Smith ; note\n", true);
        assert_eq!(value_text(&tokens), Some("John Smith"));
        assert_eq!(comment_text(&tokens), Some("; note"));
    }

    #[test]
    fn inline_comment_no_value_before_marker_stays_value() {
        // After the separator, the post-`=` whitespace is consumed, so a marker
        // at the very start of the value region is treated as value.
        let tokens = lex_with("k =    ; not a comment\n", true);
        assert_eq!(value_text(&tokens), Some("; not a comment"));
        assert!(comment_text(&tokens).is_none());
    }

    #[test]
    fn inline_comment_real_world_boot_project() {
        let input =
            "Bootproject.RetainMismatch.Init=1           ; HANDLES RETAIN VARIABLE MISMATCHES\n";
        let tokens = lex_with(input, true);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        assert_eq!(value_text(&tokens), Some("1"));
        assert_eq!(
            comment_text(&tokens),
            Some("; HANDLES RETAIN VARIABLE MISMATCHES")
        );
    }

    #[test]
    fn inline_comment_lossless_mixed() {
        let input = "k = 1   ; note\ncolor=#fff ; c\nconn = a;b\nurl = http://x/#f\n";
        let tokens = lex_with(input, true);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
    }

    #[test]
    fn inline_comment_on_final_continuation_line() {
        let input = "k = a \\\nb ; note\n";
        let tokens = lex_with(input, true);
        let reconstructed: String = tokens.iter().map(|token| token.text).collect();
        assert_eq!(reconstructed, input);
        assert_eq!(value_text(&tokens), Some("a \\\nb"));
        assert_eq!(comment_text(&tokens), Some("; note"));
    }

    #[test]
    fn inline_comment_tab_before_marker() {
        let tokens = lex_with("k = v\t; note\n", true);
        assert_eq!(value_text(&tokens), Some("v"));
        assert_eq!(comment_text(&tokens), Some("; note"));
    }

    #[test]
    fn inline_comment_after_bare_key() {
        for (input, comment) in [
            ("flag ; note\n", "; note"),
            ("flag\t#\r", "#"),
            ("flag  ; λ", "; λ"),
        ] {
            let tokens = lex_with(input, true);
            assert_eq!(
                tokens.iter().map(|token| token.text).collect::<String>(),
                input
            );
            let kinds: Vec<_> = tokens.iter().map(|token| token.kind).collect();
            assert_eq!(&kinds[..3], [IDENT, WHITESPACE, COMMENT], "{input:?}");
            assert_eq!(comment_text(&tokens), Some(comment));
            // Without the option, the marker is unexpected text after the key.
            let tokens = lex(input);
            assert_eq!(tokens[2].kind, LEX_ERROR, "{input:?}");
        }
        // Markers need whitespace before them, and text after a key is not a
        // comment just because it contains one.
        assert_eq!(lex_kinds("flag;note\n"), vec![IDENT, NEWLINE]);
        assert_eq!(
            lex_with("flag junk ; note", true)
                .iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![IDENT, WHITESPACE, LEX_ERROR]
        );
    }

    #[test]
    fn inline_comment_mcdc_truth_table() {
        assert_eq!(find_inline_comment("value ; note"), Some((5, 1)));
        assert_eq!(find_inline_comment("value\t# note"), Some((5, 1)));
        assert_eq!(find_inline_comment("value  # note"), Some((5, 2)));

        assert_eq!(find_inline_comment("value"), None);
        assert_eq!(find_inline_comment("value "), None);
        assert_eq!(find_inline_comment("value x"), None);
        assert_eq!(find_inline_comment("; at start"), None);
        assert_eq!(find_inline_comment(" # at start"), None);

        assert_eq!(
            find_inline_comment("earlier ; marker\\\nfinal # comment"),
            Some((23, 1))
        );
        assert_eq!(
            find_inline_comment("earlier # marker\\\rfinal ; comment"),
            Some((23, 1))
        );
    }

    #[test]
    fn is_line_break_accepts_only_cr_and_lf() {
        let line_breaks: Vec<u8> = (0..=u8::MAX).filter(|&byte| is_line_break(byte)).collect();
        assert_eq!(line_breaks, [b'\n', b'\r']);
    }

    #[test]
    fn line_len_stops_at_every_kind_of_line_break() {
        for (text, len) in [
            ("", 0),
            ("\n", 0),
            ("\r", 0),
            ("key", 3),
            ("key\nnext", 3),
            ("key\r\nnext", 3),
            ("key\rnext", 3),
            ("λ ;\n", 4),
        ] {
            assert_eq!(line_len(text), len, "{text:?}");
        }
    }

    #[test]
    fn line_break_len_measures_one_line_break() {
        for (text, len) in [
            ("\r\n", 2),
            ("\r\n\n", 2),
            ("\n\r", 1),
            ("\n", 1),
            ("\r", 1),
            ("\rnext", 1),
            ("", 0),
            ("next\n", 0),
            (" \n", 0),
        ] {
            assert_eq!(line_break_len(text), len, "{text:?}");
        }
    }

    #[test]
    fn value_span_len_follows_backslash_continuations() {
        for (text, len) in [
            ("", 0),
            ("value", 5),
            ("value \t\nnext", 7),
            ("a\\b\nnext", 3),
            ("a \\\nb\nnext", 5),
            ("a \\ \t\r\nb\r\nnext", 8),
            ("a\\\rb\\\nc\n", 7),
            ("\\\n\\\n\n", 4),
            // A continuation at the end of the input keeps its line break.
            ("a\\", 2),
            ("a\\\n", 3),
        ] {
            assert_eq!(value_span_len(text), len, "{text:?}");
        }
    }

    // The tests below call a lexing step in a state that its callers never
    // create. Each step panics instead of emitting a wrong token.

    fn lexer_at(rest: &str) -> Lexer<'_> {
        Lexer {
            rest,
            tokens: Vec::new(),
            inline_comments: false,
        }
    }

    #[test]
    #[should_panic(expected = "assertion failed: len > 0")]
    fn bump_rejects_an_empty_token() {
        lexer_at("key").bump(IDENT, 0);
    }

    #[test]
    #[cfg(all(debug_assertions, not(miri)))]
    #[should_panic(expected = "left: 3\n right: 9")]
    fn bump_rejects_a_line_break_inside_a_token() {
        lexer_at("key\nnext=").bump(IDENT, 9);
    }

    #[test]
    #[should_panic(expected = "left: \"junk\"\n right: \"\"")]
    fn eat_newline_rejects_a_line_lexed_short_of_its_line_break() {
        lexer_at("junk").eat_newline();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "assertion failed: self.rest.starts_with([';', '#'])")]
    fn lex_comment_requires_a_comment_marker() {
        lexer_at("key").lex_comment();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "assertion failed: self.rest.starts_with('[')")]
    fn lex_section_header_line_requires_a_bracket() {
        lexer_at("name]").lex_section_header_line();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "assertion failed: !self.rest.starts_with(")]
    fn lex_entry_line_requires_the_indentation_to_be_eaten() {
        lexer_at(" key=value").lex_entry_line();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "assertion failed: !span.starts_with(")]
    fn lex_value_requires_the_separator_whitespace_to_be_eaten() {
        lexer_at(" value").lex_value(" value".len());
    }
}
