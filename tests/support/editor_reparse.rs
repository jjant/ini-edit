use ini_edit::ast::AstNode;
use ini_edit::editor::Editor;

pub const SOURCES: [&str; 6] = [
    "",
    "[a]",
    "[a]\nx=0",
    "[a]\nx=0\n",
    "[a]\n; tail",
    "\u{FEFF}[a]\r\nx=0\r\n",
];

fn apply(editor: &Editor, operation: u8) {
    match operation % 8 {
        0 => editor.section("a").append_entry("x", "1"),
        1 => editor.section("a").insert_entry_at_line(0, "y", "2"),
        2 => editor.section("a").remove_lines(1..2),
        3 => editor.section("b").set("z", "3"),
        4 => editor.section("a").remove(),
        5 => editor.section("b").remove_lines(1..usize::MAX),
        6 => editor.section("a").set("x", ""),
        _ => editor.section("a").append_raw_lines(&["; tail"]),
    }
}

// Saving and reopening valid input should not affect the next edit. Comparing
// complete syntax trees also checks line ownership, which determines the
// meaning of public child-index operations such as remove_lines.
pub fn check(source: &str, operations: &[u8]) {
    let live = Editor::new(source);
    let mut saved = source.to_owned();
    for (step, &operation) in operations.iter().enumerate() {
        let reopened = Editor::new(&saved);
        apply(&live, operation);
        apply(&reopened, operation);
        saved = live.finish();
        assert_eq!(
            saved,
            reopened.finish(),
            "source={source:?} operations={operations:?} step={step}"
        );
        let parsed = ini_edit::parse(&saved);
        assert!(parsed.errors().is_empty(), "{saved:?}");
        assert_eq!(
            live.file().syntax().green().into_owned(),
            *parsed.green(),
            "source={source:?} operations={operations:?} step={step} output={saved:?}"
        );
    }
}
