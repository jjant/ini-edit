//! Helpers shared by the editor's unit tests.

use crate::ParseOptions;
use crate::editor::{EditError, EditOptions, Editor, SeparatorSpacing};

/// Parse options spelled out, as every editor is built with explicit options.
pub(super) fn options(allow_no_value: bool, inline_comments: bool) -> ParseOptions {
    ParseOptions {
        allow_no_value,
        inline_comments,
    }
}

/// The four parse option sets.
pub(super) fn all_options() -> [ParseOptions; 4] {
    [
        options(false, false),
        options(true, false),
        options(false, true),
        options(true, true),
    ]
}

/// Edit options with `spacing`.
pub(super) fn spacing(separator_spacing: SeparatorSpacing) -> EditOptions {
    EditOptions { separator_spacing }
}

/// The three spacing policies, `Exact` with a tab before and two spaces after the separator.
pub(super) fn all_spacings() -> [SeparatorSpacing; 3] {
    [
        SeparatorSpacing::Preserve,
        SeparatorSpacing::Compact,
        SeparatorSpacing::exact("\t", "  ").unwrap(),
    ]
}

/// An editor with both parse options off and `Preserve` spacing.
pub(super) fn open(source: &str) -> Editor {
    open_with(
        source,
        &options(false, false),
        &spacing(SeparatorSpacing::Preserve),
    )
}

/// An editor with the given options.
pub(super) fn open_with(source: &str, parse: &ParseOptions, edit: &EditOptions) -> Editor {
    let editor = Editor::new(source, parse, edit).unwrap();
    assert!(editor.agrees_with_reparse(), "{source:?}");
    editor
}

/// An editor whose limit is `len_max` bytes, with default options.
pub(super) fn open_with_len_max(source: &str, len_max: usize) -> Editor {
    let edit = spacing(SeparatorSpacing::Preserve);
    Editor::with_len_max(source, &options(false, false), &edit, len_max).unwrap()
}

/// Run `operation`. On `Err`, the text and the tree must be unchanged (C2); either way, the tree
/// must equal a parse of the text (C1).
pub(super) fn atomic<T: std::fmt::Debug>(
    editor: &mut Editor,
    operation: impl FnOnce(&mut Editor) -> Result<T, EditError>,
) -> Result<T, EditError> {
    let text = editor.finish();
    let green = editor.green.clone();
    let result = operation(editor);
    if result.is_err() {
        assert_eq!(editor.finish(), text, "C2: the text changed: {result:?}");
        assert_eq!(editor.green, green, "C2: the tree changed");
        // Planning only reads the tree, so a refused edit leaves the very same tree.
        let same = std::ptr::from_ref::<rowan::GreenNodeData>(&editor.green)
            == std::ptr::from_ref::<rowan::GreenNodeData>(&green);
        assert!(same, "C2: the tree was rebuilt");
    }
    assert!(
        editor.agrees_with_reparse(),
        "C1 after {result:?}: {:?}",
        editor.finish()
    );
    result
}

/// [`atomic`] on the section named `name`, which must exist.
pub(super) fn in_section<T: std::fmt::Debug>(
    editor: &mut Editor,
    name: &str,
    operation: impl FnOnce(&mut crate::editor::SectionEditor<'_>) -> Result<T, EditError>,
) -> Result<T, EditError> {
    atomic(editor, |editor| {
        operation(&mut editor.section(name).unwrap())
    })
}

/// [`atomic`] on the preamble.
pub(super) fn in_preamble<T: std::fmt::Debug>(
    editor: &mut Editor,
    operation: impl FnOnce(&mut crate::editor::SectionEditor<'_>) -> Result<T, EditError>,
) -> Result<T, EditError> {
    atomic(editor, |editor| operation(&mut editor.preamble()))
}

/// The body line count of section `name`.
pub(super) fn body_line_count(editor: &mut Editor, name: &str) -> usize {
    editor.section(name).unwrap().body_line_count()
}

/// All four line endings a line can have: LF, CRLF, CR, and none at the end of the document.
pub(super) const ENDINGS: [&str; 4] = ["\n", "\r\n", "\r", ""];

/// `items`, or under Miri, which runs these tests thousands of times slower, its first `count`.
/// The full tables still run in every native build, coverage run and mutation run.
pub(super) fn under_miri<T>(items: &[T], count: usize) -> &[T] {
    if cfg!(miri) {
        &items[..count.min(items.len())]
    } else {
        items
    }
}
