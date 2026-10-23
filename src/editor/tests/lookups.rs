//! Lookups, reads, counts and the size limit.

use super::support::{atomic, in_section, open, open_with, open_with_len_max, options, spacing};
use crate::MAX_INPUT_LEN;
use crate::ast::AstNode;
use crate::editor::{EditError, EditOptions, Editor, LEN_MAX, SeparatorSpacing};
use crate::syntax_kind::SyntaxKind;

#[test]
fn sections_are_found_by_exact_name_and_by_position() {
    let mut editor = open("g=1\n[a]\nx=1\n[b]\n[a]\ny=2\n[]\n[ c ]\n");
    assert_eq!(editor.section_count(), 5);
    let first = editor.section("a").unwrap();
    assert_eq!(first.entry_count(), 1);
    assert_eq!(first.body_line_count(), 1);
    assert_eq!(editor.section("c").unwrap().name().as_deref(), Some("c"));
    for missing in ["x", "", " a", "a ", "A", " c "] {
        assert!(editor.section(missing).is_none(), "{missing:?}");
    }
    // Positions reach duplicates and nameless sections.
    let names: Vec<_> = (0..editor.section_count())
        .map(|index| editor.section_at(index).unwrap().name())
        .collect();
    let expected = [Some("a"), Some("b"), Some("a"), None, Some("c")];
    assert_eq!(names, expected.map(|name| name.map(str::to_owned)));
    let mut second = editor.section_at(2).unwrap();
    assert_eq!(second.entry("y").unwrap().value().as_deref(), Some("2"));
    assert!(editor.section_at(5).is_none());
    assert!(editor.section_at(usize::MAX).is_none());
    assert_eq!(editor.finish(), "g=1\n[a]\nx=1\n[b]\n[a]\ny=2\n[]\n[ c ]\n");
}

#[test]
fn the_preamble_is_a_container_without_a_name() {
    let mut editor = open("\u{FEFF}g=1\n; c\nh=2\n[s]\nk=v\n");
    let preamble = editor.preamble();
    assert!(preamble.is_preamble());
    assert_eq!(preamble.name(), None);
    assert_eq!(preamble.body_line_count(), 3);
    assert_eq!(preamble.entry_count(), 2);
    let section = editor.section("s").unwrap();
    assert!(!section.is_preamble());
    assert_eq!(section.name().as_deref(), Some("s"));
    // An empty document has an empty preamble.
    let mut empty = open("");
    assert_eq!(empty.preamble().body_line_count(), 0);
    assert_eq!(empty.section_count(), 0);
}

#[test]
fn entries_are_found_by_exact_key_and_by_position() {
    let parse = options(true, true);
    let preserve = spacing(SeparatorSpacing::Preserve);
    let mut editor = open_with("[s]\nk=1 ; one\n\nflag\nk=2\n=bad\n", &parse, &preserve);
    let mut section = editor.section("s").unwrap();
    assert_eq!(section.body_line_count(), 5);
    assert_eq!(section.entry_count(), 3);
    let first = section.entry("k").unwrap();
    assert_eq!(first.key(), "k");
    assert_eq!(first.value().as_deref(), Some("1"));
    assert_eq!(first.inline_comment().as_deref(), Some("; one"));
    assert_eq!((first.index(), first.body_line()), (0, 0));
    let bare = section.entry("flag").unwrap();
    assert_eq!(bare.value(), None);
    assert_eq!(bare.inline_comment(), None);
    assert_eq!((bare.index(), bare.body_line()), (1, 2));
    let second = section.entry_at(2).unwrap();
    assert_eq!(second.value().as_deref(), Some("2"));
    assert_eq!((second.index(), second.body_line()), (2, 3));
    assert!(section.entry_at(3).is_none());
    for missing in ["K", "k ", "", "=bad", "x"] {
        assert!(section.entry(missing).is_none(), "{missing:?}");
    }
}

#[test]
fn positions_are_live_after_other_edits() {
    let mut editor = open("[s]\na=1\n; c\nb=2\nc=3\n");
    in_section(&mut editor, "s", |section| section.remove_entry("a")).unwrap();
    let mut section = editor.section("s").unwrap();
    let entry = section.entry("c").unwrap();
    assert_eq!((entry.index(), entry.body_line()), (1, 2));
}

#[test]
fn handles_read_the_whole_editor() {
    let mut editor = open("[s]\na=1\n");
    let mut section = editor.section("s").unwrap();
    section.set("a", "2").unwrap();
    assert_eq!(section.editor().finish(), "[s]\na=2\n");
    let entry = section.entry("a").unwrap();
    assert_eq!(entry.editor().finish(), "[s]\na=2\n");
    assert_eq!(entry.editor().section_count(), 1);
}

#[test]
fn the_options_are_kept_as_given() {
    let parse = options(true, true);
    let exact = SeparatorSpacing::exact(" ", "\t").unwrap();
    let editor = open_with("", &parse, &spacing(exact.clone()));
    assert!(editor.parse_options().allow_no_value);
    assert!(editor.parse_options().inline_comments);
    assert_eq!(editor.edit_options().separator_spacing, exact);
    let editor = open_with(
        "",
        &options(false, false),
        &spacing(SeparatorSpacing::Compact),
    );
    assert!(!editor.parse_options().allow_no_value);
    assert!(!editor.parse_options().inline_comments);
    assert_eq!(editor.edit_options(), &spacing(SeparatorSpacing::Compact));
}

#[test]
fn file_snapshots_are_immutable() {
    let source = "[s]\nk=old\nk=duplicate\n[t]\nx=1\n";
    let mut editor = open(source);
    let snapshot = editor.file();
    assert!(!snapshot.syntax().is_mutable());
    in_section(&mut editor, "s", |section| section.set("k", "new")).unwrap();
    atomic(&mut editor, |editor| {
        editor.section("t").unwrap().remove();
        Ok(())
    })
    .unwrap();
    assert_eq!(snapshot.syntax().text().to_string(), source);
    assert_eq!(
        editor.file().syntax().text().to_string(),
        "[s]\nk=new\nk=duplicate\n"
    );
    assert_eq!(editor.file().syntax().kind(), SyntaxKind::ROOT);
}

#[test]
fn the_constructor_refuses_documents_past_the_limit() {
    let parse = options(false, false);
    let edit = EditOptions::default();
    assert!(Editor::with_len_max("[s]\n", &parse, &edit, 4).is_ok());
    let refused = Editor::with_len_max("[s]\n", &parse, &edit, 3).map(drop);
    assert_eq!(refused, Err(EditError::DocumentTooLarge { len: 4, max: 3 }));
    // The public constructor uses the editor's limit.
    let editor = Editor::new("", &parse, &edit).unwrap();
    assert_eq!(editor.len_max, LEN_MAX);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn the_limit_is_one_byte_below_the_parser_s_on_64_bit_targets() {
    // One byte stays free for the line break in front of every fragment the editor parses.
    assert_eq!(LEN_MAX, MAX_INPUT_LEN - 1);
}

#[test]
#[should_panic(expected = "assertion failed: len_max <= LEN_MAX")]
fn a_limit_above_the_editor_s_is_a_programmer_error() {
    let parse = options(false, false);
    let _ = Editor::with_len_max("", &parse, &EditOptions::default(), LEN_MAX + 1);
}

#[test]
fn sizes_are_checked_exactly_at_the_limit() {
    let editor = open_with_len_max("[s]\nk=old\n", 12);
    assert_eq!(editor.check_len(0, 2), Ok(12));
    assert_eq!(
        editor.check_len(0, 3),
        Err(EditError::DocumentTooLarge { len: 13, max: 12 })
    );
    // A rewrite that removes as much as it adds fits; removed bytes are subtracted.
    assert_eq!(editor.check_len(5, 7), Ok(12));
    assert_eq!(editor.check_len(10, 0), Ok(0));
    // The sum saturates instead of wrapping.
    assert_eq!(
        editor.check_len(0, usize::MAX),
        Err(EditError::DocumentTooLarge {
            len: usize::MAX,
            max: 12
        })
    );
}

#[test]
#[should_panic(expected = "assertion failed: removed <= len")]
fn removing_more_than_the_document_is_a_programmer_error() {
    let _ = open("k=v").check_len(4, 0);
}

#[test]
fn a_corrupted_tree_disagrees_with_its_reparse() {
    let editor = open("[s]\nk=v\n");
    assert!(editor.agrees_with_reparse());
    // Move the entry out of its section to the end of the root: the text stays the same, but a
    // reader puts that line in the section.
    let entry = editor.root.first_child().unwrap().last_child().unwrap();
    entry.detach();
    let end = editor.root.children_with_tokens().count();
    editor.root.splice_children(end..end, vec![entry.into()]);
    assert_eq!(editor.finish(), "[s]\nk=v\n");
    assert!(!editor.agrees_with_reparse());
}
