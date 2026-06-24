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
pub fn lex_with(input: &str, inline_comments: bool) -> Vec<Token<'_>> {
    let mut lexer = Lexer {
        rest: input,
        tokens: Vec::new(),
        inline_comments,
    };
    // Skip UTF-8 BOM if present, emitting it as whitespace (trivia).
    if lexer.rest.starts_with('\u{FEFF}') {
        lexer.bump(SyntaxKind::WHITESPACE, 3);
    }
    while !lexer.rest.is_empty() {
        lexer.lex_line();
    }
    lexer.tokens
}

struct Lexer<'a> {
    rest: &'a str,
    tokens: Vec<Token<'a>>,
    inline_comments: bool,
}

impl Lexer<'_> {
    fn bump(&mut self, kind: SyntaxKind, len: usize) {
        let (text, rest) = self.rest.split_at(len);
        self.tokens.push(Token { kind, text });
        self.rest = rest;
    }

    fn eat_horizontal_ws(&mut self) {
        let len = self
            .rest
            .bytes()
            .take_while(|&b| b == b' ' || b == b'\t')
            .count();
        if len > 0 {
            self.bump(SyntaxKind::WHITESPACE, len);
        }
    }

    fn eat_newline(&mut self) -> bool {
        if self.rest.starts_with("\r\n") {
            self.bump(SyntaxKind::NEWLINE, 2);
            true
        } else if self.rest.starts_with('\n') || self.rest.starts_with('\r') {
            self.bump(SyntaxKind::NEWLINE, 1);
            true
        } else {
            false
        }
    }

    fn lex_line(&mut self) {
        self.eat_horizontal_ws();
        let Some(first) = self.rest.chars().next() else {
            return;
        };
        match first {
            '\r' | '\n' => {
                self.eat_newline();
            }
            ';' | '#' => {
                self.lex_comment_to_eol();
                self.eat_newline();
            }
            '[' => self.lex_section_header_line(),
            _ => self.lex_entry_line(),
        }
    }

    fn lex_comment_to_eol(&mut self) {
        let len = self
            .rest
            .bytes()
            .take_while(|&b| b != b'\n' && b != b'\r')
            .count();
        self.bump(SyntaxKind::COMMENT, len);
    }

    fn lex_section_header_line(&mut self) {
        self.bump(SyntaxKind::L_BRACK, 1);
        self.eat_horizontal_ws();

        // Section name: everything up to `]` or EOL, trailing ws separated.
        let raw_len = self
            .rest
            .bytes()
            .take_while(|&b| b != b']' && b != b'\n' && b != b'\r')
            .count();
        if raw_len > 0 {
            let raw = &self.rest[..raw_len];
            let trimmed = raw.trim_end_matches([' ', '\t']);
            let name_len = trimmed.len();
            let trail_ws = raw_len - name_len;
            if name_len > 0 {
                self.bump(SyntaxKind::IDENT, name_len);
            }
            if trail_ws > 0 {
                self.bump(SyntaxKind::WHITESPACE, trail_ws);
            }
        }

        if self.rest.starts_with(']') {
            self.bump(SyntaxKind::R_BRACK, 1);
        }

        // Trailing content on header line.
        self.eat_horizontal_ws();
        if self.rest.starts_with(';') || self.rest.starts_with('#') {
            self.lex_comment_to_eol();
        } else {
            let junk_len = self
                .rest
                .bytes()
                .take_while(|&b| b != b'\n' && b != b'\r')
                .count();
            if junk_len > 0 {
                self.bump(SyntaxKind::LEX_ERROR, junk_len);
            }
        }
        self.eat_newline();
    }

    fn lex_entry_line(&mut self) {
        let key_len = self
            .rest
            .bytes()
            .take_while(|&b| {
                b != b'=' && b != b':' && b != b' ' && b != b'\t' && b != b'\n' && b != b'\r'
            })
            .count();
        if key_len == 0 {
            self.lex_rest_of_line_as_error();
            self.eat_newline();
            return;
        }
        self.bump(SyntaxKind::IDENT, key_len);
        self.eat_horizontal_ws();

        match self.rest.chars().next() {
            Some('=') => self.bump(SyntaxKind::EQ, 1),
            Some(':') => self.bump(SyntaxKind::COLON, 1),
            _ => {
                self.lex_rest_of_line_as_error();
                self.eat_newline();
                return;
            }
        }
        self.eat_horizontal_ws();

        // Value: everything to EOL. If the logical value ends with `\`
        // (backslash continuation), consume the newline and next line too,
        // repeating until no trailing backslash. The entire multi-line span
        // becomes one VALUE_TEXT token (lossless).
        let value_start = self.rest;
        let mut total_len: usize = 0;
        loop {
            let line_len = self.rest[total_len..]
                .bytes()
                .take_while(|&b| b != b'\n' && b != b'\r')
                .count();
            let line_end = total_len + line_len;
            // Check if line ends with backslash (ignoring trailing whitespace).
            let line_content = &self.rest[total_len..line_end];
            let trimmed = line_content.trim_end_matches([' ', '\t']);
            let has_continuation = trimmed.ends_with('\\');

            if has_continuation {
                // Include the newline in the value span.
                let nl_len = if self.rest[line_end..].starts_with("\r\n") {
                    2
                } else {
                    usize::from(
                        self.rest[line_end..].starts_with('\n')
                            || self.rest[line_end..].starts_with('\r'),
                    )
                };
                total_len = line_end + nl_len;
                // Continue to next line.
            } else {
                total_len = line_end;
                break;
            }
        }

        if total_len > 0 {
            let raw = &value_start[..total_len];

            // When inline comments are enabled, a whitespace-preceded `;`/`#`
            // marker on the final physical line ends the value and starts a
            // trailing comment.
            let split = if self.inline_comments {
                find_inline_comment(raw)
            } else {
                None
            };

            if let Some((value_len, ws_len)) = split {
                if value_len > 0 {
                    self.bump(SyntaxKind::VALUE_TEXT, value_len);
                }
                if ws_len > 0 {
                    self.bump(SyntaxKind::WHITESPACE, ws_len);
                }
                let comment_len = total_len - value_len - ws_len;
                self.bump(SyntaxKind::COMMENT, comment_len);
            } else {
                // Separate trailing whitespace from the last line of the value.
                let trimmed = raw.trim_end_matches([' ', '\t']);
                let value_len = trimmed.len();
                let trail_ws_len = total_len - value_len;
                if value_len > 0 {
                    self.bump(SyntaxKind::VALUE_TEXT, value_len);
                }
                if trail_ws_len > 0 {
                    self.bump(SyntaxKind::WHITESPACE, trail_ws_len);
                }
            }
        }
        self.eat_newline();
    }

    fn lex_rest_of_line_as_error(&mut self) {
        let len = self
            .rest
            .bytes()
            .take_while(|&b| b != b'\n' && b != b'\r')
            .count();
        if len > 0 {
            self.bump(SyntaxKind::LEX_ERROR, len);
        }
    }
}

/// Locate a trailing inline comment within a value span.
///
/// Scans the final physical line of `raw` for the first run of spaces/tabs that
/// is immediately followed by a `;` or `#` marker and is not at the start of
/// that line. Returns `(value_len, ws_len)`: the byte length of the value
/// preceding the whitespace run, and the length of the whitespace run itself.
/// The comment spans from `value_len + ws_len` to the end of `raw`.
///
/// Returns `None` when there is no such marker, which leaves the whole span as
/// the value. This protects markers that are not whitespace-adjacent
/// (`a=1;b=2`), markers at the value start (`#fff`), and `;`/`#` on earlier
/// continued lines (only the final physical line is inspected).
fn find_inline_comment(raw: &str) -> Option<(usize, usize)> {
    let bytes = raw.as_bytes();
    // Only the final physical line of a (possibly continued) value is eligible.
    let final_line_start = raw.rfind(['\n', '\r']).map_or(0, |i| i + 1);

    let mut i = final_line_start;
    while i < bytes.len() {
        if bytes[i] == b' ' || bytes[i] == b'\t' {
            let ws_start = i;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < bytes.len()
                && (bytes[i] == b';' || bytes[i] == b'#')
                && ws_start > final_line_start
            {
                return Some((ws_start, i - ws_start));
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use SyntaxKind::*;

    fn lex_kinds(input: &str) -> Vec<SyntaxKind> {
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input, "lexer must be lossless");
        toks.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn empty() {
        assert!(lex("").is_empty());
    }

    #[test]
    fn blank_line() {
        assert_eq!(lex_kinds("\n"), vec![NEWLINE]);
    }

    #[test]
    fn comments() {
        assert_eq!(lex_kinds("; hi\n"), vec![COMMENT, NEWLINE]);
        assert_eq!(lex_kinds("# hi"), vec![COMMENT]);
    }

    #[test]
    fn section_header() {
        assert_eq!(lex_kinds("[foo]\n"), vec![L_BRACK, IDENT, R_BRACK, NEWLINE]);
    }

    #[test]
    fn section_with_spaces_in_name() {
        assert_eq!(
            lex_kinds("[my section]\n"),
            vec![L_BRACK, IDENT, R_BRACK, NEWLINE]
        );
        let toks = lex("[my section]\n");
        assert_eq!(toks[1].text, "my section");
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
        let toks = lex("k=v\r\n");
        assert_eq!(toks.last().unwrap().text, "\r\n");
    }

    #[test]
    fn utf8_bom() {
        let input = "\u{FEFF}[author]\nE-MAIL = u@gogs.io\n";
        let toks = lex(input);
        // Round-trip
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        // BOM is emitted as whitespace (trivia), not an error
        assert_eq!(toks[0].kind, WHITESPACE);
        assert_eq!(toks[0].text, "\u{FEFF}");
        // Parsing continues normally after BOM
        assert_eq!(toks[1].kind, L_BRACK);
    }

    #[test]
    fn backslash_continuation() {
        let input = "k=hello \\\nworld\n";
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        let value_tok = toks.iter().find(|t| t.kind == VALUE_TEXT).unwrap();
        assert_eq!(value_tok.text, "hello \\\nworld");
    }

    #[test]
    fn backslash_continuation_multiple_lines() {
        let input = "k=a \\\nb \\\nc\n";
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        let value_tok = toks.iter().find(|t| t.kind == VALUE_TEXT).unwrap();
        assert_eq!(value_tok.text, "a \\\nb \\\nc");
    }

    #[test]
    fn backslash_not_at_end_is_literal() {
        let input = "k=path\\to\\file\n";
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        let value_tok = toks.iter().find(|t| t.kind == VALUE_TEXT).unwrap();
        assert_eq!(value_tok.text, "path\\to\\file");
    }

    #[test]
    fn backslash_continuation_with_crlf() {
        let input = "k=a \\\r\nb\r\n";
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        let value_tok = toks.iter().find(|t| t.kind == VALUE_TEXT).unwrap();
        assert_eq!(value_tok.text, "a \\\r\nb");
    }

    #[test]
    fn backslash_at_eof_is_literal() {
        // No newline after backslash — it's just a literal backslash.
        let input = "k=val\\";
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        let value_tok = toks.iter().find(|t| t.kind == VALUE_TEXT).unwrap();
        assert_eq!(value_tok.text, "val\\");
    }

    #[test]
    fn bare_cr_line_ending() {
        let input = "k=v\r[s]\rx=y\r";
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
    }

    #[test]
    fn line_without_separator() {
        // A bare identifier followed by more text but no = or : triggers LEX_ERROR
        // for the trailing content.
        let input = "=value_no_key\n";
        let toks = lex(input);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        // The `=` isn't a valid start for a key, so the whole line becomes error.
        assert!(toks.iter().any(|t| t.kind == LEX_ERROR));
    }

    // --- inline comments (opt-in) ---

    fn val<'a>(toks: &'a [Token<'a>]) -> Option<&'a str> {
        toks.iter().find(|t| t.kind == VALUE_TEXT).map(|t| t.text)
    }
    fn com<'a>(toks: &'a [Token<'a>]) -> Option<&'a str> {
        toks.iter().find(|t| t.kind == COMMENT).map(|t| t.text)
    }

    #[test]
    fn inline_comment_basic() {
        let toks = lex_with("k = 1   ; note\n", true);
        assert_eq!(
            toks.iter().map(|t| t.kind).collect::<Vec<_>>(),
            vec![
                IDENT, WHITESPACE, EQ, WHITESPACE, VALUE_TEXT, WHITESPACE, COMMENT, NEWLINE
            ]
        );
        assert_eq!(val(&toks), Some("1"));
        assert_eq!(com(&toks), Some("; note"));
    }

    #[test]
    fn inline_comment_disabled_by_default() {
        let toks = lex("k = 1   ; note\n");
        assert_eq!(val(&toks), Some("1   ; note"));
        assert!(com(&toks).is_none());
    }

    #[test]
    fn inline_comment_requires_whitespace_before_marker() {
        // Connection-string style — no space before `;` keeps it in the value.
        let toks = lex_with("conn = a=1;b=2;c=3\n", true);
        assert_eq!(val(&toks), Some("a=1;b=2;c=3"));
        assert!(com(&toks).is_none());
    }

    #[test]
    fn inline_comment_marker_at_value_start_is_value() {
        let toks = lex_with("color = #fff\n", true);
        assert_eq!(val(&toks), Some("#fff"));
        assert!(com(&toks).is_none());
    }

    #[test]
    fn inline_comment_hash_after_value() {
        let toks = lex_with("color = #fff ; my color\n", true);
        assert_eq!(val(&toks), Some("#fff"));
        assert_eq!(com(&toks), Some("; my color"));
    }

    #[test]
    fn inline_comment_url_with_fragment_preserved() {
        let toks = lex_with("url = http://example.com/#section\n", true);
        assert_eq!(val(&toks), Some("http://example.com/#section"));
        assert!(com(&toks).is_none());
    }

    #[test]
    fn inline_comment_value_with_internal_spaces() {
        let toks = lex_with("name = John Smith ; note\n", true);
        assert_eq!(val(&toks), Some("John Smith"));
        assert_eq!(com(&toks), Some("; note"));
    }

    #[test]
    fn inline_comment_no_value_before_marker_stays_value() {
        // After the separator, the post-`=` whitespace is consumed, so a marker
        // at the very start of the value region is treated as value.
        let toks = lex_with("k =    ; not a comment\n", true);
        assert_eq!(val(&toks), Some("; not a comment"));
        assert!(com(&toks).is_none());
    }

    #[test]
    fn inline_comment_real_world_boot_project() {
        let input =
            "Bootproject.RetainMismatch.Init=1           ; HANDLES RETAIN VARIABLE MISMATCHES\n";
        let toks = lex_with(input, true);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        assert_eq!(val(&toks), Some("1"));
        assert_eq!(com(&toks), Some("; HANDLES RETAIN VARIABLE MISMATCHES"));
    }

    #[test]
    fn inline_comment_lossless_mixed() {
        let input = "k = 1   ; note\ncolor=#fff ; c\nconn = a;b\nurl = http://x/#f\n";
        let toks = lex_with(input, true);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
    }

    #[test]
    fn inline_comment_on_final_continuation_line() {
        let input = "k = a \\\nb ; note\n";
        let toks = lex_with(input, true);
        let reconstructed: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(reconstructed, input);
        assert_eq!(val(&toks), Some("a \\\nb"));
        assert_eq!(com(&toks), Some("; note"));
    }

    #[test]
    fn inline_comment_tab_before_marker() {
        let toks = lex_with("k = v\t; note\n", true);
        assert_eq!(val(&toks), Some("v"));
        assert_eq!(com(&toks), Some("; note"));
    }
}
