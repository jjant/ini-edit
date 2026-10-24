//! The editor's tree: identity of its nodes, sharing between the trees before and after an edit,
//! and moving or sharing an editor between threads. `tests/threads.rs` pins the auto traits that
//! each public type has, and `src/auto_traits.rs` the ones it lacks.

use super::support::{atomic, in_preamble, in_section, open};
use crate::editor::{EditError, Editor, plan};
use crate::syntax_kind::SyntaxNode;

/// For each child node of `new`, whether it is a child node of `old` itself, not an equal copy.
fn kept(old: &SyntaxNode, new: &SyntaxNode) -> Vec<bool> {
    new.children()
        .map(|child| {
            old.children()
                .any(|old| plan::is_node(&child, &old.green().to_owned()))
        })
        .collect()
}

#[test]
fn a_node_is_identified_by_its_own_green_node_not_by_an_equal_one() {
    let editor = open("[s]\nk=v\n");
    let section = editor.sections().next().unwrap();
    assert!(plan::is_node(&section, &section.green().to_owned()));
    // The same section, parsed again: equal, but another node.
    let reparsed = crate::parse("[s]\nk=v\n").syntax().first_child().unwrap();
    assert_eq!(reparsed.green(), section.green());
    assert!(!plan::is_node(&section, &reparsed.green().to_owned()));
}

/// An edit of the document in [`an_edit_shares_every_node_it_does_not_touch`], and what it keeps:
/// for each root child after the edit, whether it is itself a root child from before the edit,
/// and the same for the lines of section `b`, the third root child.
type SharingCase = (
    fn(&mut Editor) -> Result<(), EditError>,
    &'static [bool],
    &'static [bool],
);

#[test]
fn an_edit_shares_every_node_it_does_not_touch() {
    let source = "p=1\n[a]\nx=1\n[b]\ny=1\nz=1\n[c]\nw=1\n";
    let cases: [SharingCase; 6] = [
        (
            |editor| in_section(editor, "b", |section| section.set("y", "2")),
            &[true, true, false, true],
            &[true, false, true],
        ),
        (
            |editor| in_section(editor, "b", |section| section.append_body_lines(&["; c"])),
            &[true, true, false, true],
            &[true, true, true, false],
        ),
        (
            |editor| in_section(editor, "b", |section| section.remove_body_lines(0..1)),
            &[true, true, false, true],
            &[true, true],
        ),
        (
            |editor| {
                in_section(editor, "b", |section| {
                    section.rename_key("z", "q").map(drop)
                })
            },
            &[true, true, false, true],
            &[true, true, false],
        ),
        (
            |editor| in_preamble(editor, |preamble| preamble.set("p", "2")),
            &[false, true, true, true],
            &[true, true, true],
        ),
        (
            |editor| in_section(editor, "a", |section| section.insert_body_lines(0, &[""])),
            &[true, false, true, true],
            &[true, true, true],
        ),
    ];
    for (index, (edit, root, lines)) in cases.into_iter().enumerate() {
        let mut editor = open(source);
        let before = editor.root();
        edit(&mut editor).unwrap();
        let after = editor.root();
        assert_eq!(kept(&before, &after), root, "case {index}");
        let old = before.children().nth(2).unwrap();
        let new = after.children().nth(2).unwrap();
        assert_eq!(kept(&old, &new), lines, "case {index}");
    }
}

#[test]
fn a_removal_that_merges_line_breaks_rebuilds_only_the_line_before() {
    // Rule D: the CR that ends `k=1` and the LF blank line after `x=1` read as one CRLF.
    let mut editor = open("[s]\nk=1\rx=1\n\nz=1\n");
    let before = editor.root();
    in_section(&mut editor, "s", |section| section.remove_body_lines(1..2)).unwrap();
    assert_eq!(editor.finish(), "[s]\nk=1\r\nz=1\n");
    let old = before.first_child().unwrap();
    let new = editor.root().first_child().unwrap();
    assert_eq!(kept(&old, &new), [true, false, true]);
}

#[test]
fn removing_a_section_shares_every_other_one() {
    let mut editor = open("p=1\n[a]\nx=1\n[b]\ny=1\n[c]\nw=1\n");
    let before = editor.root();
    atomic(&mut editor, |editor| {
        editor.section("b").unwrap().remove();
        Ok(())
    })
    .unwrap();
    assert_eq!(editor.finish(), "p=1\n[a]\nx=1\n[c]\nw=1\n");
    assert_eq!(kept(&before, &editor.root()), [true, true, true]);
}

#[test]
fn a_new_section_shares_the_last_section_unless_that_section_gets_a_new_last_line() {
    // After a terminated blank line, the new section needs no separator, and the last section
    // stays as it was. Otherwise the last section gets a new last line, the blank separator or its
    // unterminated blank line terminated (rule C), and is rebuilt around its other lines.
    let cases = [
        (
            "[a]\nx=1\n\n",
            "[a]\nx=1\n\n[b]\n",
            [true, false],
            [true; 3],
        ),
        (
            "[a]\nx=1\n",
            "[a]\nx=1\n\n[b]\n",
            [false; 2],
            [true, true, false],
        ),
        (
            "[a]\nx=1\n  ",
            "[a]\nx=1\n  \n[b]\n",
            [false; 2],
            [true, true, false],
        ),
    ];
    for (source, output, root, lines) in cases {
        let mut editor = open(source);
        let before = editor.root();
        let result = atomic(&mut editor, |editor| {
            editor.section_or_insert("b").map(drop)
        });
        assert_eq!(result, Ok(()));
        assert_eq!(editor.finish(), output);
        let after = editor.root();
        assert_eq!(kept(&before, &after), root, "{source:?}");
        let old = before.first_child().unwrap();
        let new = after.first_child().unwrap();
        assert_eq!(kept(&old, &new), lines, "{source:?}");
    }
}

#[test]
fn an_editor_is_read_by_several_threads_and_edited_in_another() {
    let source = "[s]\nk=1\n[t]\nx=1\n";
    let mut editor = open(source);
    // Readers share the editor: each one walks its own cursors over the one green tree.
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..2)
            .map(|_| scope.spawn(|| (editor.section_count(), editor.finish())))
            .collect();
        for reader in readers {
            assert_eq!(reader.join().unwrap(), (2, source.to_owned()));
        }
    });
    // The editor moves to another thread and edits there, while this thread walks the old tree,
    // which shares section `t` with the new one.
    let old = editor.green.clone();
    let writer = std::thread::spawn(move || {
        in_section(&mut editor, "s", |section| section.set("k", "2")).unwrap();
        editor
    });
    for _ in 0..4 {
        let root = SyntaxNode::new_root(old.clone());
        assert_eq!(root.descendants().count(), 11);
    }
    let editor = writer.join().unwrap();
    assert_eq!(editor.finish(), "[s]\nk=2\n[t]\nx=1\n");
    assert_eq!(old.to_string(), source);
}
