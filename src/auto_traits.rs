//! The auto traits that public types lack, each pinned by a `compile_fail` doctest.
//!
//! Which of `Send`, `Sync`, `UnwindSafe` and `RefUnwindSafe` a type implements is part of the API
//! ([Threads](crate#threads)): removing one breaks callers, and adding one is a promise to keep.
//! `tests/threads.rs` asserts at compile time the traits that each type has; each doctest here
//! fails once a type gains a trait it lacks. So every change to the list updates a test.
//!
//! The syntax tree's cursors lack all four: rowan's `SyntaxNode` and `SyntaxToken` count their
//! references in a `Cell`, without atomics, and every [`ast`](crate::ast) type wraps a
//! `SyntaxNode`. The editor's handles lack only `UnwindSafe`, as every `&mut` borrow does.
//!
//! Only `cargo test --doc` compiles this module, as it does the README's examples.

/// Two doctests in an item named `$item`. The first fails with E0277 because `$type` does not
/// implement `$bound`. Only nightly rustdoc checks that code: stable passes a `compile_fail`
/// doctest that fails for any reason, a misspelled path included. So the second one names the same
/// type and trait, and compiles: a wrong path fails it on every toolchain.
macro_rules! lacks {
    ($item:ident, $type:literal, $bound:literal) => {
        #[doc = "```compile_fail,E0277"]
        #[doc = concat!("fn implements<T: ", $bound, ">() {}")]
        #[doc = concat!("implements::<", $type, ">();")]
        #[doc = "```"]
        #[doc = ""]
        #[doc = "```"]
        #[doc = concat!("fn implements<T: ", $bound, ">() {}")]
        #[doc = "fn names<T>() {}"]
        #[doc = "implements::<()>();"]
        #[doc = concat!("names::<", $type, ">();")]
        #[doc = "```"]
        pub struct $item;
    };
}

/// A module named `$module` with a doctest for each trait that the cursor type `$type` lacks.
macro_rules! cursor {
    ($module:ident, $type:literal) => {
        #[doc = concat!("`", $type, "` is not `Send`, `Sync`, `UnwindSafe` or `RefUnwindSafe`.")]
        pub mod $module {
            lacks!(NotSend, $type, "Send");
            lacks!(NotSync, $type, "Sync");
            lacks!(NotUnwindSafe, $type, "std::panic::UnwindSafe");
            lacks!(NotRefUnwindSafe, $type, "std::panic::RefUnwindSafe");
        }
    };
}

cursor!(syntax_node, "ini_edit::SyntaxNode");
cursor!(syntax_token, "ini_edit::SyntaxToken");
cursor!(syntax_element, "ini_edit::SyntaxElement");
cursor!(file, "ini_edit::ast::File");
cursor!(section, "ini_edit::ast::Section");
cursor!(section_header, "ini_edit::ast::SectionHeader");
cursor!(entry, "ini_edit::ast::Entry");
cursor!(key, "ini_edit::ast::Key");
cursor!(value, "ini_edit::ast::Value");
cursor!(comment_line, "ini_edit::ast::CommentLine");
cursor!(blank_line, "ini_edit::ast::BlankLine");

/// `SectionEditor` holds a `&mut Editor`, so it is not `UnwindSafe`.
pub mod section_editor {
    lacks!(
        NotUnwindSafe,
        "ini_edit::editor::SectionEditor<'static>",
        "std::panic::UnwindSafe"
    );
}

/// `EntryEditor` holds a `&mut Editor`, so it is not `UnwindSafe`.
pub mod entry_editor {
    lacks!(
        NotUnwindSafe,
        "ini_edit::editor::EntryEditor<'static>",
        "std::panic::UnwindSafe"
    );
}
