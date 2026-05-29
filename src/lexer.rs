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
#[must_use]
pub fn lex(input: &str) -> Vec<Token<'_>> {
    let mut lexer = Lexer {
        rest: input,
        tokens: Vec::new(),
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
            // Separate trailing whitespace from the last line of the value.
            let raw = &value_start[..total_len];
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
}
