//! Options, spacing validation and the error texts.

use crate::editor::{EditError, EditOptions, SeparatorSpacing, Whitespace};
use crate::lexer::lex;
use crate::syntax_kind::SyntaxKind;

#[test]
fn whitespace_is_a_run_of_spaces_and_tabs() {
    for text in ["", " ", "\t", " \t  "] {
        assert_eq!(Whitespace::new(text).unwrap().as_str(), text);
    }
    for text in ["x", " x", "\n", "\u{a0}", "\u{FEFF}", " ;", "="] {
        let refused = Err(EditError::InvalidSpacing { text: text.into() });
        assert_eq!(Whitespace::new(text), refused);
    }
}

#[test]
fn whitespace_pairs_with_the_lexer() {
    let pieces = ["", " ", "\t", "x", "=", ";", "\n", "\u{a0}"];
    for first in pieces {
        for second in pieces {
            let text = format!("{first}{second}");
            // Spacing reads back as spacing on both sides of a separator, as one token or none.
            let line = format!("k{text}={text}v");
            let tokens = lex(&line);
            let kinds: Vec<_> = tokens.iter().map(|token| token.kind).collect();
            let expected = if text.is_empty() {
                vec![SyntaxKind::IDENT, SyntaxKind::EQ, SyntaxKind::VALUE_TEXT]
            } else {
                use SyntaxKind::{EQ, IDENT, VALUE_TEXT, WHITESPACE};
                vec![IDENT, WHITESPACE, EQ, WHITESPACE, VALUE_TEXT]
            };
            let reads_back = kinds == expected
                && tokens
                    .iter()
                    .filter(|token| token.kind == SyntaxKind::WHITESPACE)
                    .all(|token| token.text == text);
            assert_eq!(Whitespace::new(&text).is_ok(), reads_back, "{text:?}");
        }
    }
}

#[test]
fn exact_spacing_checks_before_then_after() {
    let spacing = SeparatorSpacing::exact("\t", "  ").unwrap();
    let SeparatorSpacing::Exact { before, after } = &spacing else {
        panic!("{spacing:?}");
    };
    assert_eq!((before.as_str(), after.as_str()), ("\t", "  "));
    assert_eq!(spacing.new_gap(), ("\t", "  "));
    let refused = |text: &str| Err(EditError::InvalidSpacing { text: text.into() });
    assert_eq!(SeparatorSpacing::exact("x", "y"), refused("x"));
    assert_eq!(SeparatorSpacing::exact(" ", "y"), refused("y"));
}

#[test]
fn the_default_options_are_spelled_out() {
    // SAFE-10: internal code spells options out; this pins what the default is.
    let explicit = EditOptions {
        separator_spacing: SeparatorSpacing::Preserve,
    };
    assert_eq!(EditOptions::default(), explicit);
    assert_eq!(SeparatorSpacing::default(), SeparatorSpacing::Preserve);
    assert_eq!(Whitespace::default().as_str(), "");
    assert_eq!(SeparatorSpacing::Preserve.new_gap(), (" ", " "));
    assert_eq!(SeparatorSpacing::Compact.new_gap(), ("", ""));
}

/// Every error with the exact text it displays.
fn error_texts() -> [(EditError, &'static str); 11] {
    [
        (
            EditError::DocumentTooLarge { len: 15, max: 10 },
            "the document would be 15 bytes long, more than the 10 bytes the editor can hold",
        ),
        (
            EditError::InvalidSpacing { text: " ;".into() },
            "\" ;\" is not separator spacing: spacing may contain only spaces and tabs",
        ),
        (
            EditError::InvalidKey { key: "a=b".into() },
            "\"a=b\" cannot be written as a key: keys must not be empty, must not contain '=', \
             ':', a space, a tab or a line break, and must not start with ';', '#' or '['",
        ),
        (
            EditError::InvalidValue {
                value: "a\nb".into(),
            },
            "\"a\\nb\" cannot be written as a value: values must not start or end with a space \
             or tab, may break lines only right after a continuation backslash, must not end \
             their last line with a backslash, and, with inline comments enabled, must not \
             contain ';' or '#' after a space or tab that follows other text on their last line",
        ),
        (
            EditError::InvalidSectionName { name: "a]".into() },
            "\"a]\" cannot be written as a section name: names must not be empty, must not \
             contain ']' or a line break, and must not start or end with a space or tab",
        ),
        (
            EditError::InvalidLine {
                index: 1,
                line: "[x]".into(),
            },
            "line 1 (\"[x]\") cannot be inserted: each element must be exactly one line that is \
             not a section header and does not end with a continuation backslash; only an \
             entry's value may continue onto further lines",
        ),
        (
            EditError::DuplicateKey { key: "b".into() },
            "another entry in the same section or preamble already has the key \"b\"",
        ),
        (
            EditError::PositionOutOfRange {
                at: 3,
                body_line_count: 2,
            },
            "position 3 is past the end of a body with 2 lines",
        ),
        (
            EditError::RangeOutOfRange {
                start: 3,
                end: 1,
                body_line_count: 2,
            },
            "lines 3..1 are not a range within a body with 2 lines",
        ),
        (
            EditError::LastLineContinues,
            "the document's last line has no line break and ends with a continuation \
             backslash, so a line added after it would become part of its value",
        ),
        (
            EditError::ByteOrderMarkAtStart,
            "the edit would start a document that has no byte order mark with U+FEFF, which a \
             reader would take as a byte order mark",
        ),
    ]
}

#[test]
fn every_error_states_its_rule() {
    for (error, text) in error_texts() {
        assert_eq!(error.to_string(), text);
        let boxed: Box<dyn std::error::Error> = Box::new(error.clone());
        assert_eq!(boxed.to_string(), text);
    }
}
