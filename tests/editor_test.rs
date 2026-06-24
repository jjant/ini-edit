//! Integration tests for the Editor API.

use ini_edit::editor::Editor;
use ini_edit::parse;

#[test]
fn append_raw_lines_and_remove_lines() {
    let src = "[config]\nkey = value\n";
    let ed = Editor::new(src);

    // Insert a managed block.
    ed.section("config").append_raw_lines(&[
        "; --- BEGIN MANAGED ---",
        "managed_key = managed_val",
        "; --- END MANAGED ---",
    ]);

    let after_insert = ed.finish();
    assert!(after_insert.contains("; --- BEGIN MANAGED ---"));
    assert!(after_insert.contains("managed_key = managed_val"));
    assert!(after_insert.contains("; --- END MANAGED ---"));
    assert!(after_insert.contains("key = value")); // original preserved

    // Remove the inserted block using remove_lines.
    // Re-parsed line-node layout (child index == line index):
    //   0 SECTION_HEADER, 1 ENTRY(key), 2 COMMENT_LINE(BEGIN),
    //   3 ENTRY(managed_key), 4 COMMENT_LINE(END)
    let ed2 = Editor::new(&after_insert);
    ed2.section("config").remove_lines(2..5);

    let after_remove = ed2.finish();
    assert!(!after_remove.contains("MANAGED"), "got: {after_remove}");
    assert!(!after_remove.contains("managed_key"), "got: {after_remove}");
    assert!(after_remove.contains("key = value"));
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

    let ed = Editor::new(src);

    // Modify existing value.
    ed.section("server").set("port", "9090");

    // Add new entry.
    ed.section("server").append_entry("timeout", "30");

    // Rename a key.
    assert!(
        ed.section("database")
            .rename_key("url", "connection_string")
    );

    // Create a new section.
    ed.section("cache").set("backend", "redis");

    let output = ed.finish();

    // Verify edits.
    assert!(output.contains("port = 9090"), "got: {output}");
    assert!(output.contains("timeout = 30"), "got: {output}");
    assert!(
        output.contains("connection_string = postgres://localhost/db"),
        "got: {output}"
    );
    assert!(output.contains("[cache]"), "got: {output}");
    assert!(output.contains("backend = redis"), "got: {output}");

    // Verify preservation.
    assert!(output.contains("; App config"), "got: {output}");
    assert!(output.contains("host = 0.0.0.0"), "got: {output}");

    // Verify the result still parses cleanly.
    let p = parse(&output);
    assert!(p.errors().is_empty());
    assert_eq!(p.syntax().text().to_string(), output);
}

#[test]
fn remove_section_preserves_rest() {
    let src = "[a]\nx = 1\n[b]\ny = 2\n[c]\nz = 3\n";
    let ed = Editor::new(src);
    ed.section("b").remove();
    let output = ed.finish();

    assert!(output.contains("[a]\nx = 1"));
    assert!(output.contains("[c]\nz = 3"));
    assert!(!output.contains("[b]"));

    let p = parse(&output);
    assert_eq!(p.syntax().text().to_string(), output);
}
