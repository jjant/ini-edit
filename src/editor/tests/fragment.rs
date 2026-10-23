//! The editor's reading of text in the middle of a document, paired with the lexer and parser.

use super::support::{all_options, options};
use crate::ast::{AstNode, Entry};
use crate::editor::fragment::{
    self, final_line, fragment_lines, joins_line_breaks, line_break_at, line_continues,
    parse_fragment,
};
use crate::lexer::{Token, lex, lex_with, line_break_len, line_len};
use crate::syntax_kind::{SyntaxKind, SyntaxNode};
use crate::{ParseOptions, parse_with};

/// Every string of up to `count` pieces.
fn strings(pieces: &[&str], count: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..count {
        layer = layer
            .iter()
            .flat_map(|prefix| pieces.iter().map(move |piece| format!("{prefix}{piece}")))
            .collect();
        all.extend(layer.iter().cloned());
    }
    all.sort();
    all.dedup();
    all
}

/// The green nodes of `nodes`.
fn greens(nodes: impl Iterator<Item = SyntaxNode>) -> Vec<rowan::GreenNode> {
    nodes.map(|node| node.green().to_owned()).collect()
}

#[test]
fn fragments_parse_like_the_same_lines_inside_a_document() {
    // The lexer's alphabet: delimiters, line breaks, whitespace, a continuation and U+FEFF.
    let pieces = [
        "", " ", "\t", "\n", "\r", "=", ":", "[", "]", ";", "\\", "\u{FEFF}", "a", "λ",
    ];
    let count = if cfg!(miri) { 1 } else { 3 };
    for options in all_options() {
        for text in strings(&pieces, count) {
            // The same lines after another line of a document.
            let document = parse_with(&format!("k=v\n{text}"), &options).syntax();
            let mut lines = document.children();
            assert_eq!(lines.next().unwrap().text(), "k=v\n");
            let fragment = parse_fragment(&text, &options);
            assert_eq!(greens(fragment.into_iter()), greens(lines), "{text:?}");
        }
    }
}

#[test]
fn a_fragment_starting_with_u_feff_is_text_not_a_byte_order_mark() {
    // A bare key U+FEFF is exactly the token a document-start parse takes as a byte order mark.
    let fragment = parse_fragment("\u{FEFF}\n", &options(true, false));
    let kinds: Vec<_> = fragment.iter().map(SyntaxNode::kind).collect();
    assert_eq!(kinds, [SyntaxKind::ENTRY]);
    let fragment = parse_fragment("\u{FEFF}k=1\n", &options(false, false));
    let entry = Entry::cast(fragment[0].clone()).unwrap();
    assert_eq!(entry.key().as_deref(), Some("\u{FEFF}k"));
}

#[test]
fn fragments_of_every_line_kind_are_the_document_s_lines() {
    let lines = [
        "[s]\n",
        " [s] ; c\n",
        "[]\n",
        "[x",
        "k=v\n",
        "k = v ; c\n",
        "flag\n",
        "flag ; c\n",
        "key junk\n",
        "=bad\n",
        "k=a \\\n  b\n",
        "k=c \\\r\n",
        "; c\n",
        "# c\r\n",
        "\n",
        "  \r",
        "\u{FEFF}k=1\n",
        "path=C:\\d\\",
    ];
    for options in all_options() {
        for line in lines {
            let document = parse_with(&format!("[z]\n{line}"), &options).syntax();
            let section = document.first_child().unwrap();
            let expected = greens(section.children().skip(1));
            let flattened = greens(fragment_lines(line, &options).into_iter());
            if line.trim_start_matches([' ', '\t']).starts_with('[') {
                // A header starts a section of its own inside the document.
                let expected = greens(
                    document
                        .children()
                        .skip(1)
                        .flat_map(|section| section.children()),
                );
                assert_eq!(flattened, expected, "{line:?}");
            } else {
                assert_eq!(flattened, expected, "{line:?}");
            }
        }
    }
}

#[test]
fn fragment_lines_are_the_parser_s_lines_of_the_fragment() {
    let lines = fragment_lines("[s]\nk=v\n", &ParseOptions::default());
    assert_eq!(lines.len(), 2);
    // The header and the entry of the fragment's section, read in the middle of a document.
    for line in &lines {
        let section = line.parent().unwrap();
        assert_eq!(section.kind(), SyntaxKind::SECTION);
        assert_eq!(section.parent().unwrap().kind(), SyntaxKind::ROOT);
    }
    assert_eq!(lines[0].kind(), SyntaxKind::SECTION_HEADER);
    assert_eq!(lines[1].kind(), SyntaxKind::ENTRY);
}

/// How the lexer reads `before` followed by `after`: whether one of its tokens holds both the
/// last byte of `before` and the first byte of `after` as one CRLF line break.
fn lexer_joins(before: &str, after: &str) -> bool {
    let text = format!("{before}{after}");
    let boundary = before.len();
    let mut start = 0;
    for token in lex(&text) {
        let end = start + token.text.len();
        let spans_boundary = start < boundary && boundary < end;
        if spans_boundary && text[boundary - 1..].starts_with("\r\n") {
            return true;
        }
        start = end;
    }
    false
}

#[test]
fn joining_line_breaks_pairs_with_the_lexer_in_every_context() {
    // Line endings in every lexing context: values, continuations, comments, headers, keys.
    let befores = [
        "",
        "x",
        "k=v\r",
        "k=v",
        "k=a \\\r",
        "; c\r",
        "[s]\r",
        "[s",
        "flag\r",
        "=bad\r",
        "\r",
        "\n",
        "\r\n",
        "k=\u{FEFF}\r",
    ];
    let afters = ["", "\n", "\nk=v", "\r", "\r\n", "x", "\n\n", "[s]"];
    for before in befores {
        for after in afters {
            let joins = joins_line_breaks(before.chars().next_back(), after.chars().next());
            assert_eq!(joins, lexer_joins(before, after), "{before:?} {after:?}");
        }
    }
    // The decision table: (CR, LF) joins; (other, LF), (CR, other) and the edges do not.
    assert!(joins_line_breaks(Some('\r'), Some('\n')));
    assert!(!joins_line_breaks(Some('x'), Some('\n')));
    assert!(!joins_line_breaks(Some('\r'), Some('x')));
    assert!(!joins_line_breaks(None, Some('\n')));
    assert!(!joins_line_breaks(Some('\r'), None));
}

#[test]
fn the_line_break_at_the_start_of_a_text_is_the_lexer_s() {
    for (text, expected) in [
        ("\r\n", "\r\n"),
        ("\r\nx", "\r\n"),
        ("\r", "\r"),
        ("\rx", "\r"),
        ("\r\r\n", "\r"),
        ("\r\u{FEFF}", "\r"),
        ("\n", "\n"),
        ("\n\r", "\n"),
        ("\nλ", "\n"),
    ] {
        assert_eq!(line_break_at(text), expected, "{text:?}");
        assert_eq!(lex(text)[0].text, expected, "{text:?}");
    }
}

#[test]
#[should_panic(expected = "assertion failed")]
fn a_line_break_is_read_only_where_one_starts() {
    let _ = line_break_at("x\n");
}

#[test]
fn the_final_line_is_the_text_after_the_last_line_break() {
    for (text, expected) in [
        ("", ""),
        ("abc", "abc"),
        ("a \\\nb", "b"),
        ("a \\\r\nb", "b"),
        ("a \\\rb c", "b c"),
        ("head \\\n", ""),
    ] {
        assert_eq!(final_line(text), expected, "{text:?}");
    }
}

/// The text of the first inline comment among `tokens`, if any.
fn comment_text<'a>(tokens: &[Token<'a>]) -> Option<&'a str> {
    tokens
        .iter()
        .find(|token| token.kind == SyntaxKind::COMMENT)
        .map(|token| token.text)
}

#[test]
fn the_final_line_is_the_line_the_lexer_searches_for_an_inline_comment() {
    // The characters that decide where a value's lines, and its inline comment, start and end.
    let pieces = [" ", "\t", "\r", "\n", "\\", ";", "#", "a"];
    let count = if cfg!(miri) { 2 } else { 5 };
    for tail in strings(&pieces, count) {
        // The first line continues, so the value always has a line before its final one.
        let text = format!("k=a \\\n{tail}");
        let tokens = lex_with(&text, true);
        let entry_len = tokens
            .iter()
            .take_while(|token| token.kind != SyntaxKind::NEWLINE)
            .count();
        let (entry, after_entry) = tokens.split_at(entry_len);
        assert_eq!(entry[0].kind, SyntaxKind::IDENT);
        assert_eq!(entry[1].kind, SyntaxKind::EQ);
        // The value span: the value, then its trailing spacing or its inline comment.
        let span: String = entry[2..].iter().map(|token| token.text).collect();
        let last = final_line(&span);
        let earlier = &span[..span.len() - last.len()];
        // The final line is the span's longest suffix without a line break.
        assert!(!last.contains(['\r', '\n']), "{text:?}");
        assert!(earlier.ends_with(['\r', '\n']), "{text:?}");
        // The lexer finds an inline comment only in that line, the one it finds there alone.
        let comment = comment_text(entry);
        if let Some(comment) = comment {
            assert!(last.ends_with(comment), "{text:?}");
        }
        let alone = format!("k={last}");
        assert_eq!(comment, comment_text(&lex_with(&alone, true)), "{text:?}");
        // The value continued over every earlier line, and the lexer ended it at the final line,
        // which therefore does not continue, unless the text ended there.
        let mut rest = earlier;
        while !rest.is_empty() {
            let line = &rest[..line_len(rest)];
            assert!(line_continues(line), "{text:?}");
            rest = &rest[line.len() + line_break_len(&rest[line.len()..])..];
        }
        if !after_entry.is_empty() {
            assert!(!line_continues(last), "{text:?}");
        }
    }
}

#[test]
fn line_continuation_pairs_with_the_lexer() {
    let pieces = ["", " ", "\t", "\\", "a", ";", "\u{FEFF}"];
    for line in strings(&pieces, 4) {
        // The lexer continues the value onto the next line iff the line continues.
        let text = format!("k={line}\nnext");
        let tokens = lex(&text);
        let continued = !tokens.iter().any(|token| token.kind == SyntaxKind::NEWLINE);
        assert_eq!(line_continues(&line), continued, "{line:?}");
    }
    assert!(line_continues("C:\\dir\\"));
    assert!(line_continues("a \\ \t"));
    assert!(!line_continues("a\\b"));
    assert!(!line_continues(""));
}

#[test]
#[should_panic(expected = "assertion failed")]
fn line_continuation_is_decided_for_one_line() {
    let _ = line_continues("a \\\nb");
}

#[test]
fn the_crlf_constant_is_one_line_break() {
    assert_eq!(lex(fragment::CRLF).len(), 1);
    assert_eq!(lex(fragment::CRLF)[0].kind, SyntaxKind::NEWLINE);
}
