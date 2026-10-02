//! Large inputs run separately from the fast suite and mutation testing.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::Editor;
use ini_edit::{ParseOptions, parse, parse_with};

#[test]
#[ignore = "large-input suite; run in the extended-tests CI job"]
fn multi_megabyte_values_and_large_unicode_identifiers() {
    let section = "界".repeat(32_768);
    let key = "κ".repeat(65_536);
    let value = "🙂".repeat(1_048_576);
    let source = format!("\u{FEFF}[{section}]\r\n{key} = {value}\r\n; keep");
    let parsed = parse(&source);
    assert!(parsed.errors().is_empty());
    assert_eq!(parsed.syntax().text().to_string(), source);
    let editor = Editor::new(&source);
    editor.section(&section).set(&key, "small");
    assert_eq!(
        editor.finish(),
        format!("\u{FEFF}[{section}]\r\n{key} = small\r\n; keep")
    );
}

#[test]
#[ignore = "large-input suite; run in the extended-tests CI job"]
fn many_continuations_are_one_value() {
    let mut value = String::new();
    for ending in ["\n", "\r\n", "\r"].into_iter().cycle().take(32_768) {
        value.push_str("λ \\");
        value.push_str(ending);
    }
    value.push_str("last");
    let source = format!("[s]\nk={value} ; retained\nnext=stay\n");
    let options = ParseOptions {
        inline_comments: true,
        ..Default::default()
    };
    let parsed = parse_with(&source, &options);
    assert!(parsed.errors().is_empty());
    let file = File::cast(parsed.syntax()).unwrap();
    let section = file.sections().next().unwrap();
    assert_eq!(section.entries().count(), 2);
    assert_eq!(
        section.entries().next().unwrap().value().as_deref(),
        Some(value.as_str())
    );
    let editor = Editor::with_parse_options(&source, &options);
    editor.section("s").set("k", "short");
    assert_eq!(editor.finish(), "[s]\nk=short ; retained\nnext=stay\n");
}

#[test]
#[ignore = "large-input suite; run in the extended-tests CI job"]
fn many_errors_have_correct_offsets_without_losing_text() {
    let source = "bad key\n".repeat(65_536);
    let parsed = parse(&source);
    assert_eq!(parsed.syntax().text().to_string(), source);
    // Each line is missing a separator and contains unexpected trailing text.
    assert_eq!(parsed.errors().len(), 2 * 65_536);
    for (line, errors) in parsed.errors().chunks_exact(2).enumerate() {
        for error in errors {
            assert_eq!(error.offset, line * "bad key\n".len() + "bad ".len());
        }
    }
}
