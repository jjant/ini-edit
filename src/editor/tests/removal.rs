//! Removal: ranges, rule D, rule B, and removing whole sections or the preamble.

use super::support::{atomic, body_line_count, in_preamble, in_section, open};
use crate::editor::{EditError, Editor};

/// One removal on an editor, for tables of removals.
type Removal = fn(&mut Editor) -> Result<(), EditError>;

/// Remove body lines `range` of section `s` and return the text.
fn removed(source: &str, range: std::ops::Range<usize>) -> String {
    let mut editor = open(source);
    in_section(&mut editor, "s", |section| section.remove_body_lines(range)).unwrap();
    editor.finish()
}

#[test]
fn ranges_must_lie_within_the_body() {
    let mut editor = open("[s]\na=1\nb=2\n");
    let out_of_range = |start, end| {
        Err(EditError::RangeOutOfRange {
            start,
            end,
            body_line_count: 2,
        })
    };
    let reversed = std::ops::Range { start: 3, end: 1 };
    let result = in_section(&mut editor, "s", |section| {
        section.remove_body_lines(reversed)
    });
    assert_eq!(result, out_of_range(3, 1));
    let result = in_section(&mut editor, "s", |section| section.remove_body_lines(2..3));
    assert_eq!(result, out_of_range(2, 3));
    let result = in_section(&mut editor, "s", |section| {
        section.remove_body_lines(0..usize::MAX)
    });
    assert_eq!(result, out_of_range(0, usize::MAX));
    // An empty range at the end, or anywhere, removes nothing.
    in_section(&mut editor, "s", |section| section.remove_body_lines(2..2)).unwrap();
    in_section(&mut editor, "s", |section| section.remove_body_lines(0..0)).unwrap();
    assert_eq!(editor.finish(), "[s]\na=1\nb=2\n");
    in_section(&mut editor, "s", |section| section.remove_body_lines(0..2)).unwrap();
    assert_eq!(editor.finish(), "[s]\n");
}

#[test]
fn removals_delete_exactly_their_lines() {
    let source = "[s]\na=1\n; c\n\nb=2\n=bad\n[t]\n";
    assert_eq!(removed(source, 1..3), "[s]\na=1\nb=2\n=bad\n[t]\n");
    assert_eq!(removed(source, 4..5), "[s]\na=1\n; c\n\nb=2\n[t]\n");
    assert_eq!(removed(source, 0..5), "[s]\n[t]\n");
    // A continued entry is one line.
    assert_eq!(removed("[s]\nk=a \\\n b\nn=1\n", 0..1), "[s]\nn=1\n");
}

#[test]
fn rule_d_merges_a_cr_with_a_following_lf_blank_line() {
    // Into an entry, into a header, and into a comment line: bytes unchanged, one line fewer.
    for (source, range, expected, lines) in [
        ("[s]\na=1\rb=2\n\nc=3\n", 1..2, "[s]\na=1\r\nc=3\n", 2),
        ("[s]\rx=1\n\nk=2\n", 0..1, "[s]\r\nk=2\n", 1),
        ("[s]\n; c\rx=1\n\n", 1..2, "[s]\n; c\r\n", 1),
    ] {
        let mut editor = open(source);
        in_section(&mut editor, "s", |section| section.remove_body_lines(range)).unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
        assert_eq!(body_line_count(&mut editor, "s"), lines, "{source:?}");
    }
    // A CR before anything but an LF blank line, or an LF before an LF blank: no merge.
    for (source, range, expected, lines) in [
        ("[s]\na=1\rb=2\nc=3\n", 1..2, "[s]\na=1\rc=3\n", 2),
        ("[s]\na=1\rb=2\n \n", 1..2, "[s]\na=1\r \n", 2),
        ("[s]\na=1\nb=2\n\n", 1..2, "[s]\na=1\n\n", 2),
        ("[s]\na=1\rb=2\n\r\n", 1..2, "[s]\na=1\r\r\n", 2),
    ] {
        let mut editor = open(source);
        in_section(&mut editor, "s", |section| section.remove_body_lines(range)).unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
        assert_eq!(body_line_count(&mut editor, "s"), lines, "{source:?}");
    }
}

#[test]
fn rule_d_needs_a_line_on_both_sides() {
    // Nothing after: the range ends the document.
    let mut editor = open("[s]\ra=1\nb=2\n");
    in_section(&mut editor, "s", |section| section.remove_body_lines(1..2)).unwrap();
    assert_eq!(editor.finish(), "[s]\ra=1\n");
    // Nothing before: the range starts the preamble.
    let mut editor = open("a=1\n\nb=2\n");
    in_preamble(&mut editor, |preamble| preamble.remove_body_lines(0..1)).unwrap();
    assert_eq!(editor.finish(), "\nb=2\n");
    // The next section's header never starts with LF.
    let mut editor = open("[a]\rx=1\n[b]\n");
    in_section(&mut editor, "a", |section| section.remove_body_lines(0..1)).unwrap();
    assert_eq!(editor.finish(), "[a]\r[b]\n");
}

#[test]
fn rule_d_keeps_reverse_iteration_positions() {
    // Removing in reverse: a merge only drops the blank line after the removed range.
    let mut editor = open("[s]\ra=1\rb=2\n\nc=3\n");
    for index in (0..2).rev() {
        atomic(&mut editor, |editor| {
            let mut section = editor.section("s").unwrap();
            section.entry_at(index).unwrap().remove()
        })
        .unwrap();
    }
    // Removing `b` merged `a`'s CR with the blank line; removing `a` then took that CRLF.
    assert_eq!(editor.finish(), "[s]\rc=3\n");
    assert_eq!(editor.section("s").unwrap().entry_count(), 1);
}

#[test]
fn rule_b_guards_removals_at_the_document_start() {
    let refused = Err(EditError::ByteOrderMarkAtStart);
    let removals: [Removal; 3] = [
        |editor| editor.preamble().remove_entry("a").map(drop),
        |editor| editor.preamble().remove_body_lines(0..1),
        |editor| editor.preamble().entry_at(0).unwrap().remove(),
    ];
    for removal in removals {
        let mut editor = open("a=1\n\u{FEFF}b=2\n");
        assert_eq!(atomic(&mut editor, removal), refused);
        // With a byte order mark, or with any line left before it, the line is text.
        let mut editor = open("\u{FEFF}a=1\n\u{FEFF}b=2\n");
        atomic(&mut editor, removal).unwrap();
        assert_eq!(editor.finish(), "\u{FEFF}\u{FEFF}b=2\n");
    }
    let mut editor = open("g=1\na=1\n\u{FEFF}b=2\n");
    in_preamble(&mut editor, |preamble| preamble.remove_body_lines(1..2)).unwrap();
    assert_eq!(editor.finish(), "g=1\n\u{FEFF}b=2\n");
}

#[test]
fn remove_entry_reports_whether_there_was_one() {
    let mut editor = open("[s]\na=1\nb=2\n");
    assert_eq!(
        in_section(&mut editor, "s", |section| section.remove_entry("x")),
        Ok(false)
    );
    assert_eq!(
        in_section(&mut editor, "s", |section| section.remove_entry("a")),
        Ok(true)
    );
    assert_eq!(
        in_section(&mut editor, "s", |section| section.remove_entry("a")),
        Ok(false)
    );
    assert_eq!(editor.finish(), "[s]\nb=2\n");
}

#[test]
fn removing_a_key_followed_by_junk_removes_its_whole_line() {
    for line in ["key junk", "key\t\tjunk", "key = ok"] {
        for by_handle in [false, true] {
            let mut editor = open(&format!("[s]\n{line}\nnext=1\n"));
            in_section(&mut editor, "s", |section| {
                if by_handle {
                    section.entry_at(0).unwrap().remove()
                } else {
                    section.remove_entry("key").map(drop)
                }
            })
            .unwrap();
            assert_eq!(editor.finish(), "[s]\nnext=1\n");
        }
    }
}

#[test]
fn removing_a_section_removes_its_header_and_body() {
    let mut editor = open("; top\n[a]\nx=1\n\n[b]\ny=2\n[c]\n");
    atomic(&mut editor, |editor| {
        editor.section("b").unwrap().remove();
        Ok(())
    })
    .unwrap();
    assert_eq!(editor.finish(), "; top\n[a]\nx=1\n\n[c]\n");
    atomic(&mut editor, |editor| {
        editor.section_at(1).unwrap().remove();
        Ok(())
    })
    .unwrap();
    assert_eq!(editor.finish(), "; top\n[a]\nx=1\n\n");
    // The byte order mark belongs to the document, not to its first section.
    let mut editor = open("\u{FEFF} [first]\na=1\n[second]\nb=2\n");
    atomic(&mut editor, |editor| {
        editor.section("first").unwrap().remove();
        Ok(())
    })
    .unwrap();
    assert_eq!(editor.finish(), "\u{FEFF}[second]\nb=2\n");
}

#[test]
fn removing_the_preamble_keeps_the_byte_order_mark() {
    for (source, expected) in [
        ("\u{FEFF}g=1\n; c\n[s]\nk=v\n", "\u{FEFF}[s]\nk=v\n"),
        ("g=1\n\n[s]\n", "[s]\n"),
        ("g=1", ""),
        ("[s]\n", "[s]\n"),
        ("\u{FEFF}", "\u{FEFF}"),
    ] {
        let mut editor = open(source);
        atomic(&mut editor, |editor| {
            editor.preamble().remove();
            Ok(())
        })
        .unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
    }
}

#[test]
fn removals_match_reloaded_editing() {
    for source in ["[a]", "[a]\nk=1", "[a]\nk=", "[a]\nk=1\n", "[a]\n\n"] {
        let mut live = open(source);
        atomic(&mut live, |editor| {
            editor.section_or_insert("b")?.set("x", "2")
        })
        .unwrap();
        let mut reopened = open(&live.finish());
        for editor in [&mut live, &mut reopened] {
            atomic(editor, |editor| {
                editor.section("a").unwrap().remove();
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(live.finish(), reopened.finish(), "{source:?}");
        assert_eq!(live.finish(), "[b]\nx = 2\n");
    }
}

#[test]
fn removals_in_mixed_documents_change_no_byte() {
    for before in ["\r", "\n", "\r\n"] {
        for blank in ["\r", "\n", "\r\n"] {
            for indent in ["", "\t", " "] {
                let source = format!("[s]\na=old{before}x=remove\n{indent}{blank}c=keep\n");
                let mut editor = open(&source);
                assert_eq!(
                    in_section(&mut editor, "s", |section| section.remove_entry("x")),
                    Ok(true)
                );
                let expected = format!("[s]\na=old{before}{indent}{blank}c=keep\n");
                assert_eq!(editor.finish(), expected);
                // Rule D merges exactly the CR line with an unindented LF blank line.
                let merged = before == "\r" && indent.is_empty() && blank == "\n";
                let lines = if merged { 2 } else { 3 };
                assert_eq!(body_line_count(&mut editor, "s"), lines, "{source:?}");
            }
        }
    }
}
