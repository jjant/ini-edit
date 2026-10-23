//! Edge case tests derived from bugs reported against other INI libraries:
//! rust-ini, go-ini/ini, npm/ini, configparser-rs, `SimpleIni`, systemd,
//! PHP `parse_ini_file`, .NET ini-parser, tree-sitter-ini.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditError, EditOptions, Editor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse};

fn assert_round_trip(s: &str) {
    let p = parse(s);
    assert_eq!(p.syntax().text().to_string(), s);
}

// --- Comment chars in values (rust-ini #96, go-ini #318, npm/ini #81) ---

#[test]
fn semicolon_in_value_is_not_comment() {
    let src = "[s]\nurl = http://x.com/path;jsessionid=abc\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    let entry = file.sections().next().unwrap().entries().next().unwrap();
    assert_eq!(
        entry.value().as_deref(),
        Some("http://x.com/path;jsessionid=abc")
    );
}

#[test]
fn hash_in_value_is_not_comment() {
    let src = "[s]\ncolor = #ff0000\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    let entry = file.sections().next().unwrap().entries().next().unwrap();
    assert_eq!(entry.value().as_deref(), Some("#ff0000"));
}

#[test]
fn multiple_comment_chars_in_value() {
    assert_round_trip("[s]\nweird = a;b#c;d#e\n");
}

// --- Values containing = or : (rust-ini #133, SimpleIni) ---

#[test]
fn equals_in_value() {
    let src = "[s]\nconnstr = host=localhost;port=5432;db=mydb\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    let entry = file.sections().next().unwrap().entries().next().unwrap();
    assert_eq!(entry.key().as_deref(), Some("connstr"));
    assert_eq!(
        entry.value().as_deref(),
        Some("host=localhost;port=5432;db=mydb")
    );
}

#[test]
fn colon_in_value_with_equals_separator() {
    let src = "[s]\ntime = 12:30:00\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    let entry = file.sections().next().unwrap().entries().next().unwrap();
    assert_eq!(entry.value().as_deref(), Some("12:30:00"));
}

// --- Section name edge cases (rust-ini #84, .NET #255, #256) ---

#[test]
fn section_name_with_special_chars() {
    assert_round_trip("[section.with.dots]\nk = v\n");
    assert_round_trip("[section:with:colons]\nk = v\n");
    assert_round_trip("[section-with-dashes]\nk = v\n");
    assert_round_trip("[section_with_underscores]\nk = v\n");
}

#[test]
fn section_name_with_spaces() {
    let src = "[my section name]\nk = v\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    assert_eq!(
        file.sections().next().unwrap().name().as_deref(),
        Some("my section name")
    );
}

#[test]
fn section_name_unicode() {
    assert_round_trip("[日本語セクション]\nキー = 値\n");
    assert_round_trip("[Ñoño]\nclave = valor\n");
}

// --- No final newline (rust-ini #21, #34, #123, go-ini #362) ---

#[test]
fn no_final_newline_entry() {
    assert_round_trip("[s]\nk = v");
}

#[test]
fn no_final_newline_section_only() {
    assert_round_trip("[s]");
}

#[test]
fn no_final_newline_comment() {
    assert_round_trip("; just a comment");
}

#[test]
fn no_final_newline_multiple_entries() {
    assert_round_trip("[s]\na = 1\nb = 2");
}

// --- Empty values (rust-ini, PHP, tree-sitter-ini #9) ---

#[test]
fn empty_value_with_equals() {
    let src = "[s]\nkey =\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    let entry = file.sections().next().unwrap().entries().next().unwrap();
    assert_eq!(entry.key().as_deref(), Some("key"));
}

#[test]
fn empty_value_with_equals_no_space() {
    assert_round_trip("[s]\nkey=\n");
}

#[test]
fn empty_value_with_colon() {
    assert_round_trip("[s]\nkey :\n");
}

// --- Duplicate sections (go-ini, ini4j, .NET #215) ---

#[test]
fn duplicate_sections_round_trip() {
    let src = "[peer]\nkey = a\n[peer]\nkey = b\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].name().as_deref(), Some("peer"));
    assert_eq!(sections[1].name().as_deref(), Some("peer"));
}

#[test]
fn duplicate_sections_editor_targets_first() {
    let src = "[peer]\nkey = a\n[peer]\nkey = b\n";
    let mut editor = open(src);
    editor
        .section("peer")
        .unwrap()
        .set("key", "changed")
        .unwrap();
    // The first occurrence changes; the second stays.
    assert_eq!(
        meaning(&editor.finish()),
        [
            (None, vec![]),
            (Some("peer".into()), vec![entry("key", "changed")]),
            (Some("peer".into()), vec![entry("key", "b")]),
        ]
    );
}

// --- BOM handling (rust-ini #79, systemd) ---

#[test]
fn utf8_bom_round_trips() {
    let src = "\u{FEFF}[s]\nk = v\n";
    assert_round_trip(src);
}

#[test]
fn utf8_bom_with_crlf() {
    let src = "\u{FEFF}[s]\r\nk = v\r\n";
    assert_round_trip(src);
}

// --- Backslash handling (rust-ini #130, #123) ---

#[test]
fn backslash_in_value_preserved() {
    let src = "[paths]\ndir = C:\\Users\\admin\\Documents\n";
    assert_round_trip(src);
    let p = parse(src);
    let file = File::cast(p.syntax()).unwrap();
    let entry = file.sections().next().unwrap().entries().next().unwrap();
    assert_eq!(
        entry.value().as_deref(),
        Some("C:\\Users\\admin\\Documents")
    );
}

#[test]
fn backslash_at_end_of_file() {
    assert_round_trip("[s]\nk = v\\\n");
    assert_round_trip("[s]\nk = v\\");
}

// --- Whitespace edge cases (rust-ini #126, #140, configparser-rs) ---

#[test]
fn leading_whitespace_in_key() {
    // Some dialects (AWS CLI) use indented keys
    assert_round_trip("[s]\n  indented_key = value\n");
}

#[test]
fn trailing_whitespace_in_value() {
    assert_round_trip("[s]\nk = value   \n");
}

#[test]
fn whitespace_around_separator() {
    assert_round_trip("[s]\nk   =   v\n");
    assert_round_trip("[s]\nk\t=\tv\n");
}

#[test]
fn blank_lines_between_entries() {
    let src = "[s]\na = 1\n\n\nb = 2\n";
    assert_round_trip(src);
}

// --- Long lines (systemd #3302) ---

#[test]
fn very_long_value() {
    let long_val = "x".repeat(10_000);
    let src = format!("[s]\nk = {long_val}\n");
    assert_round_trip(&src);
}

#[test]
fn very_long_key() {
    let long_key = "k".repeat(1_000);
    let src = format!("[s]\n{long_key} = v\n");
    assert_round_trip(&src);
}

#[test]
fn very_long_section_name() {
    let long_name = "s".repeat(1_000);
    let src = format!("[{long_name}]\nk = v\n");
    assert_round_trip(&src);
}

// --- Malformed inputs that must still round-trip (tree-sitter-ini, systemd) ---

#[test]
fn unclosed_section_bracket() {
    assert_round_trip("[unclosed\nk = v\n");
}

#[test]
fn empty_section_name() {
    assert_round_trip("[]\nk = v\n");
}

#[test]
fn garbage_line_between_sections() {
    assert_round_trip("[a]\nk = v\n!@#$%^&*()\n[b]\nx = y\n");
}

#[test]
fn line_with_only_separator() {
    assert_round_trip("[s]\n=\n");
    assert_round_trip("[s]\n=value\n");
}

// --- Editor operations on edge cases ---
//
// Each edit is checked by its meaning in a reparse of the saved text, not only
// by the text round-tripping, which holds for any input (NEG-13).

/// An editor with every option spelled out.
fn open(source: &str) -> Editor {
    let parse_options = ParseOptions {
        allow_no_value: false,
        inline_comments: false,
    };
    let edit_options = EditOptions {
        separator_spacing: SeparatorSpacing::Preserve,
    };
    Editor::new(source, &parse_options, &edit_options).unwrap()
}

type Entries = Vec<(String, Option<String>)>;

/// The sections and entries a reader finds in `text`, the preamble first.
fn meaning(text: &str) -> Vec<(Option<String>, Entries)> {
    let parse = parse(text);
    assert_eq!(parse.syntax().text().to_string(), text);
    let file = File::cast(parse.syntax()).unwrap();
    let entries = |entries: Vec<ini_edit::ast::Entry>| {
        entries
            .into_iter()
            .map(|entry| (entry.key().unwrap(), entry.value()))
            .collect()
    };
    let mut result = vec![(None, entries(file.preamble_entries().collect()))];
    result.extend(
        file.sections()
            .map(|section| (section.name(), entries(section.entries().collect()))),
    );
    result
}

fn entry(key: &str, value: &str) -> (String, Option<String>) {
    (key.to_owned(), Some(value.to_owned()))
}

#[test]
fn editor_on_empty_input() {
    let mut editor = open("");
    editor
        .section_or_insert("new")
        .unwrap()
        .set("k", "v")
        .unwrap();
    assert_eq!(
        meaning(&editor.finish()),
        [(None, vec![]), (Some("new".into()), vec![entry("k", "v")])]
    );
}

#[test]
fn editor_on_comment_only_input() {
    let mut editor = open("; just comments\n# more\n");
    editor
        .section_or_insert("s")
        .unwrap()
        .set("k", "v")
        .unwrap();
    let out = editor.finish();
    assert!(out.starts_with("; just comments\n# more\n"));
    assert_eq!(
        meaning(&out),
        [(None, vec![]), (Some("s".into()), vec![entry("k", "v")])]
    );
}

#[test]
fn editor_set_value_with_equals() {
    let mut editor = open("[s]\nk = old\n");
    editor.section("s").unwrap().set("k", "a=b=c").unwrap();
    assert_eq!(
        meaning(&editor.finish()),
        [
            (None, vec![]),
            (Some("s".into()), vec![entry("k", "a=b=c")])
        ]
    );
}

#[test]
fn editor_set_value_with_comment_chars() {
    let mut editor = open("[s]\nk = old\n");
    editor
        .section("s")
        .unwrap()
        .set("k", "val;ue#here")
        .unwrap();
    assert_eq!(
        meaning(&editor.finish()),
        [
            (None, vec![]),
            (Some("s".into()), vec![entry("k", "val;ue#here")])
        ]
    );
}

#[test]
fn editor_remove_only_entry_in_section() {
    let mut editor = open("[s]\nk = v\n[other]\nx = y\n");
    assert_eq!(editor.section("s").unwrap().remove_entry("k"), Ok(true));
    assert_eq!(
        meaning(&editor.finish()),
        [
            (None, vec![]),
            (Some("s".into()), vec![]),
            (Some("other".into()), vec![entry("x", "y")]),
        ]
    );
}

#[test]
fn editor_rename_to_key_with_special_chars() {
    let mut editor = open("[s]\nold = v\n");
    let renamed = editor
        .section("s")
        .unwrap()
        .rename_key("old", "new.key-name");
    assert_eq!(renamed, Ok(true));
    assert_eq!(
        meaning(&editor.finish()),
        [
            (None, vec![]),
            (Some("s".into()), vec![entry("new.key-name", "v")])
        ]
    );
}

#[test]
fn editor_refuses_keys_and_values_that_would_not_read_back() {
    let mut editor = open("[s]\nk = v\n");
    let mut section = editor.section("s").unwrap();
    let refused = section.rename_key("k", "a=b");
    assert_eq!(refused, Err(EditError::InvalidKey { key: "a=b".into() }));
    let refused = section.set("k", "v\n[injected]");
    assert!(matches!(refused, Err(EditError::InvalidValue { .. })));
    assert_eq!(editor.finish(), "[s]\nk = v\n");
}

#[test]
fn editor_multiple_operations_same_section() {
    let mut editor = open("[s]\na = 1\nb = 2\nc = 3\n");
    let mut section = editor.section("s").unwrap();
    section.set("a", "10").unwrap();
    assert_eq!(section.remove_entry("b"), Ok(true));
    section.append_entry("d", "4").unwrap();
    assert_eq!(section.rename_key("c", "cc"), Ok(true));
    assert_eq!(
        meaning(&editor.finish()),
        [
            (None, vec![]),
            (
                Some("s".into()),
                vec![entry("a", "10"), entry("cc", "3"), entry("d", "4")]
            ),
        ]
    );
}

#[test]
fn editor_on_crlf_file() {
    let mut editor = open("[s]\r\nk = old\r\n");
    editor.section("s").unwrap().set("k", "new").unwrap();
    editor.section("s").unwrap().append_entry("n", "1").unwrap();
    assert_eq!(editor.finish(), "[s]\r\nk = new\r\nn = 1\r\n");
}

#[test]
fn editor_on_file_with_bom() {
    let mut editor = open("\u{FEFF}[s]\nk = old\n");
    editor.section("s").unwrap().set("k", "new").unwrap();
    let out = editor.finish();
    assert!(out.starts_with('\u{FEFF}'));
    assert_eq!(
        meaning(&out),
        [(None, vec![]), (Some("s".into()), vec![entry("k", "new")])]
    );
}

// --- Regression: fuzzer crash from out-of-bounds insert_raw_lines_at ---

#[test]
fn fuzzed_insert_raw_lines_oob_index() {
    // 0.4 clamped the position; it is now an error that changes nothing.
    let mut editor = open("");
    let mut section = editor.section_or_insert("s").unwrap();
    let result = section.insert_body_lines(84, &[""]);
    let expected = EditError::PositionOutOfRange {
        at: 84,
        body_line_count: 0,
    };
    assert_eq!(result, Err(expected));
    assert_eq!(editor.finish(), "[s]\n");
    let result = editor.preamble().insert_body_lines(84, &[""]);
    assert!(matches!(result, Err(EditError::PositionOutOfRange { .. })));
    assert_eq!(editor.finish(), "[s]\n");
}
