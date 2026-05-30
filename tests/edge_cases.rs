//! Edge case tests derived from bugs reported against other INI libraries:
//! rust-ini, go-ini/ini, npm/ini, configparser-rs, `SimpleIni`, systemd,
//! PHP `parse_ini_file`, .NET ini-parser, tree-sitter-ini.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::Editor;
use ini_edit::parse;

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
    let ed = Editor::new(src);
    ed.section("peer").set("key", "changed");
    let out = ed.finish();
    // First occurrence should be modified
    assert!(out.contains("key = changed"));
    // Second should remain
    assert!(out.contains("key = b"));
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

#[test]
fn editor_on_empty_input() {
    let ed = Editor::new("");
    ed.section("new").set("k", "v");
    let out = ed.finish();
    assert!(out.contains("[new]"));
    assert!(out.contains("k = v"));
    // Must re-parse cleanly
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_on_comment_only_input() {
    let ed = Editor::new("; just comments\n# more\n");
    ed.section("s").set("k", "v");
    let out = ed.finish();
    assert!(out.contains("; just comments"));
    assert!(out.contains("[s]"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_set_value_with_equals() {
    let ed = Editor::new("[s]\nk = old\n");
    ed.section("s").set("k", "a=b=c");
    let out = ed.finish();
    assert!(out.contains("k = a=b=c"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_set_value_with_comment_chars() {
    let ed = Editor::new("[s]\nk = old\n");
    ed.section("s").set("k", "val;ue#here");
    let out = ed.finish();
    assert!(out.contains("val;ue#here"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_remove_only_entry_in_section() {
    let ed = Editor::new("[s]\nk = v\n[other]\nx = y\n");
    let _ = ed.section("s").remove_entry("k");
    let out = ed.finish();
    assert!(out.contains("[s]"));
    assert!(!out.contains("k = v"));
    assert!(out.contains("[other]"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_rename_to_key_with_special_chars() {
    let ed = Editor::new("[s]\nold = v\n");
    let _ = ed.section("s").rename_key("old", "new.key-name");
    let out = ed.finish();
    assert!(out.contains("new.key-name = v"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_multiple_operations_same_section() {
    let ed = Editor::new("[s]\na = 1\nb = 2\nc = 3\n");
    ed.section("s").set("a", "10");
    let _ = ed.section("s").remove_entry("b");
    ed.section("s").append_entry("d", "4");
    let _ = ed.section("s").rename_key("c", "cc");
    let out = ed.finish();
    assert!(out.contains("a = 10"));
    assert!(!out.contains("b = 2"));
    assert!(out.contains("d = 4"));
    assert!(out.contains("cc = 3"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_on_crlf_file() {
    let ed = Editor::new("[s]\r\nk = old\r\n");
    ed.section("s").set("k", "new");
    let out = ed.finish();
    assert!(out.contains("k = new"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

#[test]
fn editor_on_file_with_bom() {
    let ed = Editor::new("\u{FEFF}[s]\nk = old\n");
    ed.section("s").set("k", "new");
    let out = ed.finish();
    assert!(out.starts_with('\u{FEFF}'));
    assert!(out.contains("k = new"));
    let re = parse(&out);
    assert_eq!(re.syntax().text().to_string(), out);
}

// --- Regression: fuzzer crash from out-of-bounds insert_raw_lines_at ---

#[test]
fn fuzzed_insert_raw_lines_oob_index() {
    let ed = Editor::new("");
    ed.section("").insert_raw_lines_at(84, &[""]);
    let output = ed.finish();
    let re = parse(&output);
    assert_eq!(re.syntax().text().to_string(), output);
}
