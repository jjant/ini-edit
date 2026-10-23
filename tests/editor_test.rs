//! Integration tests for the Editor API.

use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse};

/// Parse options with both options off.
fn plain() -> ParseOptions {
    ParseOptions {
        allow_no_value: false,
        inline_comments: false,
    }
}

/// Edit options with `separator_spacing`.
fn spacing(separator_spacing: SeparatorSpacing) -> EditOptions {
    EditOptions { separator_spacing }
}

#[test]
fn append_body_lines_and_remove_body_lines() {
    let src = "[config]\nkey = value\n";
    let preserve = spacing(SeparatorSpacing::Preserve);
    let mut editor = Editor::new(src, &plain(), &preserve).unwrap();

    // Insert a managed block.
    editor
        .section("config")
        .unwrap()
        .append_body_lines(&[
            "; --- BEGIN MANAGED ---",
            "managed_key = managed_val",
            "; --- END MANAGED ---",
        ])
        .unwrap();

    let after_insert = editor.finish();
    assert_eq!(
        after_insert,
        "[config]\nkey = value\n; --- BEGIN MANAGED ---\nmanaged_key = managed_val\n\
         ; --- END MANAGED ---\n"
    );
    // The raw entry is an entry right away.
    let mut config = editor.section("config").unwrap();
    let managed = config.entry("managed_key").unwrap().value();
    assert_eq!(managed.as_deref(), Some("managed_val"));

    // Remove the block by body lines, after reopening: 0 is `key = value`, 1..4 the block.
    let mut reopened = Editor::new(&after_insert, &plain(), &preserve).unwrap();
    let mut config = reopened.section("config").unwrap();
    assert_eq!(config.body_line_count(), 4);
    config.remove_body_lines(1..4).unwrap();
    assert_eq!(reopened.finish(), src);
}

#[test]
fn full_editing_workflow() {
    let src = "\
; App config
[server]
host = 0.0.0.0
port = 8080

[database]
url = postgres://localhost/db
";

    let mut editor = Editor::new(src, &plain(), &spacing(SeparatorSpacing::Preserve)).unwrap();

    // Modify an existing value and add a new entry.
    let mut server = editor.section("server").unwrap();
    server.set("port", "9090").unwrap();
    server.append_entry("timeout", "30").unwrap();

    // Rename a key.
    let mut database = editor.section("database").unwrap();
    assert_eq!(database.rename_key("url", "connection_string"), Ok(true));

    // Create a new section.
    let mut cache = editor.section_or_insert("cache").unwrap();
    cache.set("backend", "redis").unwrap();

    let output = editor.finish();
    assert_eq!(
        output,
        "; App config\n[server]\nhost = 0.0.0.0\nport = 9090\ntimeout = 30\n\n[database]\n\
         connection_string = postgres://localhost/db\n\n[cache]\nbackend = redis\n"
    );

    // The result still parses cleanly.
    let parsed = parse(&output);
    assert!(parsed.errors().is_empty());
    assert_eq!(parsed.syntax().text().to_string(), output);
}

#[test]
fn remove_section_preserves_rest() {
    let src = "[a]\nx = 1\n[b]\ny = 2\n[c]\nz = 3\n";
    let mut editor = Editor::new(src, &plain(), &spacing(SeparatorSpacing::Preserve)).unwrap();
    editor.section("b").unwrap().remove();
    assert_eq!(editor.finish(), "[a]\nx = 1\n[c]\nz = 3\n");
}

#[test]
fn configurable_separator_spacing_only_normalizes_touched_entries() {
    let parse_options = ParseOptions {
        allow_no_value: true,
        inline_comments: true,
    };
    let mut editor = Editor::new(
        "[service]\r\nendpoint = old   ; retained\r\nflag   \r\nuntouched : value\r\n",
        &parse_options,
        &spacing(SeparatorSpacing::Compact),
    )
    .unwrap();

    let mut service = editor.section("service").unwrap();
    service.set("endpoint", "unix:///tmp/service.sock").unwrap();
    service.set("flag", "enabled").unwrap();
    service.append_entry("timeout", "30").unwrap();

    assert_eq!(
        editor.finish(),
        "[service]\r\nendpoint=unix:///tmp/service.sock   ; retained\r\nflag=enabled\r\n\
         untouched : value\r\ntimeout=30\r\n"
    );
}

#[test]
fn exact_separator_spacing_applies_to_updates_and_insertions() {
    let exact = spacing(SeparatorSpacing::exact("\t", "  ").unwrap());
    let mut editor = Editor::new("[service]\nendpoint: old\n", &plain(), &exact).unwrap();

    let mut service = editor.section("service").unwrap();
    service.set("endpoint", "new").unwrap();
    service.insert_entry(1, "timeout", "30").unwrap();

    assert_eq!(
        editor.finish(),
        "[service]\nendpoint\t:  new\ntimeout\t=  30\n"
    );
}
