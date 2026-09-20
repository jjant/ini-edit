//! Decision tables and boundary cases used by source, branch, and MC/DC coverage.

use ini_edit::ast::{
    AstNode, BlankLine, CommentLine, Entry, File, Key, Section, SectionHeader, Value,
};
use ini_edit::editor::Editor;
use ini_edit::lexer::{lex, lex_with};
use ini_edit::{IniLang, ParseError, ParseOptions, SyntaxKind, SyntaxNode, parse, parse_with};
use rowan::Language;

fn kinds(input: &str) -> Vec<SyntaxKind> {
    let tokens = lex(input);
    assert_eq!(
        tokens.iter().map(|token| token.text).collect::<String>(),
        input
    );
    tokens.iter().map(|token| token.kind).collect()
}

fn token_text(input: &str, kind: SyntaxKind) -> Option<&str> {
    lex_with(input, true)
        .into_iter()
        .find(|token| token.kind == kind)
        .map(|token| token.text)
}

fn empty_node(kind: SyntaxKind) -> SyntaxNode {
    let mut builder = rowan::GreenNodeBuilder::new();
    builder.start_node(kind.into());
    builder.finish_node();
    SyntaxNode::new_root(builder.finish())
}

#[test]
fn lexer_newline_and_delimiter_decision_table() {
    use SyntaxKind::{
        COMMENT, EQ, IDENT, L_BRACK, LEX_ERROR, NEWLINE, R_BRACK, VALUE_TEXT, WHITESPACE,
    };

    assert_eq!(kinds(" \t"), vec![WHITESPACE]);
    assert_eq!(kinds("; comment\r"), vec![COMMENT, NEWLINE]);
    assert_eq!(kinds("key\r"), vec![IDENT, NEWLINE]);
    assert_eq!(kinds("=bad\r"), vec![LEX_ERROR, NEWLINE]);

    assert_eq!(kinds("[]\n"), vec![L_BRACK, R_BRACK, NEWLINE]);
    assert_eq!(
        kinds("[ \t]\n"),
        vec![L_BRACK, WHITESPACE, R_BRACK, NEWLINE]
    );
    assert_eq!(
        kinds("[name \t]\n"),
        vec![L_BRACK, IDENT, WHITESPACE, R_BRACK, NEWLINE]
    );
    assert_eq!(
        kinds("[name] ; comment\r"),
        vec![L_BRACK, IDENT, R_BRACK, WHITESPACE, COMMENT, NEWLINE]
    );
    assert_eq!(
        kinds("[name] # comment\n"),
        vec![L_BRACK, IDENT, R_BRACK, WHITESPACE, COMMENT, NEWLINE]
    );
    assert_eq!(
        kinds("[name] junk\r"),
        vec![L_BRACK, IDENT, R_BRACK, WHITESPACE, LEX_ERROR, NEWLINE]
    );
    assert_eq!(kinds("[unterminated\r"), vec![L_BRACK, IDENT, NEWLINE]);

    let continued = "k=a \\\rb\r";
    assert_eq!(kinds(continued), vec![IDENT, EQ, VALUE_TEXT, NEWLINE]);
    assert_eq!(token_text(continued, VALUE_TEXT), Some("a \\\rb"));
}

#[test]
fn inline_comment_mcdc_decision_table() {
    use SyntaxKind::{COMMENT, VALUE_TEXT};

    for (input, value, comment) in [
        ("k=value ; note\n", Some("value"), Some("; note")),
        ("k=value\t# note\n", Some("value"), Some("# note")),
        ("k=value  # note\n", Some("value"), Some("# note")),
        ("k=value\n", Some("value"), None),
        ("k=value \n", Some("value"), None),
        ("k=value x\n", Some("value x"), None),
        ("k=value;not-comment\n", Some("value;not-comment"), None),
        ("k=;at-start\n", Some(";at-start"), None),
        (
            "k=x \\\n ;at-final-start\n",
            Some("x \\\n ;at-final-start"),
            None,
        ),
        (
            "k=earlier ; marker\\\nfinal # comment\n",
            Some("earlier ; marker\\\nfinal"),
            Some("# comment"),
        ),
        (
            "k=earlier # marker\\\rfinal ; comment\r",
            Some("earlier # marker\\\rfinal"),
            Some("; comment"),
        ),
    ] {
        let tokens = lex_with(input, true);
        assert_eq!(
            tokens.iter().map(|token| token.text).collect::<String>(),
            input
        );
        assert_eq!(
            tokens
                .iter()
                .find(|token| token.kind == VALUE_TEXT)
                .map(|token| token.text),
            value,
            "input: {input:?}"
        );
        assert_eq!(
            tokens
                .iter()
                .find(|token| token.kind == COMMENT)
                .map(|token| token.text),
            comment,
            "input: {input:?}"
        );
    }
}

#[test]
fn parser_round_trips_boundary_matrix() {
    for newline in ["\n", "\r\n", "\r"] {
        for separator in ["=", ":"] {
            for leading in ["", " ", "\t"] {
                for trailing in ["", " ", "\t"] {
                    let input = format!(
                        "{leading}[section]{trailing}{newline}{leading}key{separator}value{trailing}{newline}"
                    );
                    let parsed = parse(&input);
                    assert_eq!(parsed.syntax().text().to_string(), input);
                    assert!(parsed.errors().is_empty(), "input: {input:?}");
                }
            }
        }
    }

    let options = ParseOptions {
        allow_no_value: true,
        inline_comments: true,
    };
    let input = "[s]\rflag\rkey=value ; note\r";
    let parsed = parse_with(input, &options);
    assert_eq!(parsed.syntax().text().to_string(), input);
    assert!(parsed.errors().is_empty());
}

#[test]
fn diagnostics_cover_final_empty_and_missing_lines() {
    let final_line = ParseError {
        message: "problem".to_string(),
        offset: 4,
    };
    assert!(final_line.display("one\nlast").contains("  2 | last"));

    let final_empty_line = ParseError {
        message: "problem".to_string(),
        offset: usize::MAX,
    };
    assert!(final_empty_line.display("one\n").contains("  2 | "));

    let missing_line = ParseError {
        message: "problem".to_string(),
        offset: usize::MAX,
    };
    assert!(missing_line.display("one").contains("  1 | one"));
}

#[test]
fn malformed_ast_accessors_are_total() {
    let file = File::cast(empty_node(SyntaxKind::ROOT)).unwrap();
    assert!(file.preamble_entries().next().is_none());
    assert!(file.sections().next().is_none());
    assert_eq!(file.syntax().kind(), SyntaxKind::ROOT);

    let section = Section::cast(empty_node(SyntaxKind::SECTION)).unwrap();
    assert!(section.header().is_none());
    assert!(section.name().is_none());
    assert!(section.entries().next().is_none());
    assert!(section.comment_lines().next().is_none());

    let header = SectionHeader::cast(empty_node(SyntaxKind::SECTION_HEADER)).unwrap();
    assert!(header.name_token().is_none());
    assert!(header.name().is_none());
    assert_eq!(header.syntax().kind(), SyntaxKind::SECTION_HEADER);

    let entry = Entry::cast(empty_node(SyntaxKind::ENTRY)).unwrap();
    assert!(entry.key_node().is_none());
    assert!(entry.key().is_none());
    assert!(entry.value_node().is_none());
    assert!(entry.value().is_none());
    assert!(!entry.uses_colon());
    assert!(entry.inline_comment().is_none());

    let key = Key::cast(empty_node(SyntaxKind::KEY)).unwrap();
    assert!(key.token().is_none());
    assert!(key.text().is_none());
    assert_eq!(key.syntax().kind(), SyntaxKind::KEY);

    let value = Value::cast(empty_node(SyntaxKind::VALUE)).unwrap();
    assert!(value.token().is_none());
    assert!(value.text().is_none());
    assert_eq!(value.syntax().kind(), SyntaxKind::VALUE);

    let comment = CommentLine::cast(empty_node(SyntaxKind::COMMENT_LINE)).unwrap();
    assert!(comment.token().is_none());
    assert!(comment.text().is_none());
    assert_eq!(comment.syntax().kind(), SyntaxKind::COMMENT_LINE);

    let blank = BlankLine::cast(empty_node(SyntaxKind::BLANK_LINE)).unwrap();
    assert_eq!(blank.syntax().kind(), SyntaxKind::BLANK_LINE);

    let root = empty_node(SyntaxKind::ROOT);
    assert!(Section::cast(root.clone()).is_none());
    assert!(SectionHeader::cast(root.clone()).is_none());
    assert!(Entry::cast(root.clone()).is_none());
    assert!(Key::cast(root.clone()).is_none());
    assert!(Value::cast(root.clone()).is_none());
    assert!(CommentLine::cast(root.clone()).is_none());
    assert!(BlankLine::cast(root).is_none());
}

#[test]
fn ast_separator_decision_table() {
    let file = File::cast(parse("[s]\nequals = one\ncolon: two\nbare\n").syntax()).unwrap();
    let entries: Vec<_> = file.sections().next().unwrap().entries().collect();

    assert_eq!(entries[0].value().as_deref(), Some("one"));
    assert!(!entries[0].uses_colon());
    assert_eq!(entries[1].value().as_deref(), Some("two"));
    assert!(entries[1].uses_colon());
    assert_eq!(entries[2].value(), None);
    assert!(!entries[2].uses_colon());
}

#[test]
fn editor_boundary_decision_table() {
    let editor = Editor::new("global = true\n\n");
    editor.section("new").append_entry("key", "value");
    assert_eq!(editor.finish(), "global = true\n\n[new]\nkey = value\n");

    let editor = Editor::new("[s]\r\n");
    editor
        .section("s")
        .append_raw_lines(&["lf\n", "crlf\r\n", "cr\r", "unterminated"]);
    assert_eq!(editor.finish(), "[s]\r\nlf\ncrlf\r\ncr\runterminated\n");

    let editor = Editor::new("[s]\na: 1\n");
    editor.section("s").set("a", "2");
    assert_eq!(editor.finish(), "[s]\na: 2\n");

    let editor = Editor::with_parse_options(
        "[s]\nflag\n",
        &ParseOptions {
            allow_no_value: true,
            ..Default::default()
        },
    );
    editor.section("s").set("flag", "on");
    assert_eq!(editor.finish(), "[s]\nflag = on\n");

    let editor = Editor::new("[s]\na = 1\nb = 2\n");
    let reversed_start = 3;
    let reversed_end = 1;
    editor
        .section("s")
        .remove_lines(reversed_start..reversed_end);
    editor.section("s").remove_lines(2..2);
    assert_eq!(editor.finish(), "[s]\na = 1\nb = 2\n");
}

#[test]
fn every_syntax_kind_round_trips_through_rowan() {
    for kind in [
        SyntaxKind::WHITESPACE,
        SyntaxKind::NEWLINE,
        SyntaxKind::COMMENT,
        SyntaxKind::L_BRACK,
        SyntaxKind::R_BRACK,
        SyntaxKind::EQ,
        SyntaxKind::COLON,
        SyntaxKind::IDENT,
        SyntaxKind::VALUE_TEXT,
        SyntaxKind::LEX_ERROR,
        SyntaxKind::ROOT,
        SyntaxKind::SECTION,
        SyntaxKind::SECTION_HEADER,
        SyntaxKind::ENTRY,
        SyntaxKind::KEY,
        SyntaxKind::VALUE,
        SyntaxKind::COMMENT_LINE,
        SyntaxKind::BLANK_LINE,
    ] {
        let raw = IniLang::kind_to_raw(kind);
        assert_eq!(IniLang::kind_from_raw(raw), kind);
    }

    assert!(SyntaxKind::WHITESPACE.is_trivia());
    assert!(SyntaxKind::NEWLINE.is_trivia());
    assert!(SyntaxKind::COMMENT.is_trivia());
    assert!(!SyntaxKind::IDENT.is_trivia());
}

#[test]
#[should_panic(expected = "kind out of range")]
fn invalid_syntax_kind_panics() {
    let _ = IniLang::kind_from_raw(rowan::SyntaxKind(u16::MAX));
}
