//! Section creation by `section_or_insert`.

use super::support::{atomic, in_section, open};
use crate::editor::{EditError, Editor};

/// Create section `name` in `source`, or find it, and return the text.
fn with_section(source: &str, name: &str) -> String {
    let mut editor = open(source);
    atomic(&mut editor, |editor| {
        editor.section_or_insert(name).map(drop)
    })
    .unwrap();
    editor.finish()
}

#[test]
fn an_existing_section_is_found_not_created() {
    let source = "[a]\nx=1\n[b]\n[a]\n";
    let mut editor = open(source);
    let found = atomic(&mut editor, |editor| {
        let section = editor.section_or_insert("a")?;
        Ok(section.body_line_count())
    });
    assert_eq!(found, Ok(1));
    assert_eq!(editor.finish(), source);
}

#[test]
fn a_new_section_is_separated_by_one_blank_line() {
    for (source, expected) in [
        // An empty or byte-order-mark-only document needs no separator.
        ("", "[t]\n"),
        ("\u{FEFF}", "\u{FEFF}[t]\n"),
        // A blank last line is the separator.
        ("[s]\nk=v\n\n", "[s]\nk=v\n\n[t]\n"),
        ("g = 1\n\n", "g = 1\n\n[t]\n"),
        // Otherwise one is added, after terminating the last line.
        ("[s]\nk=v\n", "[s]\nk=v\n\n[t]\n"),
        ("[s]\nk=v", "[s]\nk=v\n\n[t]\n"),
        ("g = 1\n", "g = 1\n\n[t]\n"),
        ("; c", "; c\n\n[t]\n"),
        // A last section with only its header.
        ("[s]", "[s]\n\n[t]\n"),
        ("[s]\n", "[s]\n\n[t]\n"),
        // An unterminated blank line becomes the separator once terminated.
        (" \t", " \t\n[t]\n"),
        ("[s]\nk=1\n \t", "[s]\nk=1\n \t\n[t]\n"),
        // A value that is U+FEFF is text, not a byte order mark.
        ("k=\u{FEFF}", "k=\u{FEFF}\n\n[t]\n"),
    ] {
        assert_eq!(with_section(source, "t"), expected, "{source:?}");
    }
}

#[test]
fn a_new_section_uses_the_document_s_line_ending() {
    for (source, expected) in [
        ("[s]\rk=v\r", "[s]\rk=v\r\r[next]\r"),
        ("[s]\r\nk=v\r\n", "[s]\r\nk=v\r\n\r\n[next]\r\n"),
        // An LF blank line after a final CR line would read as one CRLF with it.
        ("[s]\nk=v\r", "[s]\nk=v\r\r\n[next]\n"),
        // The first line break may be inside a continued value.
        ("k=a \\\rb\nz=1\n", "k=a \\\rb\nz=1\n\r[next]\r"),
    ] {
        assert_eq!(with_section(source, "next"), expected, "{source:?}");
    }
}

#[test]
fn the_separator_belongs_to_the_previous_container() {
    let mut editor = open("[s]\nk=v\n");
    atomic(&mut editor, |editor| {
        editor.section_or_insert("t").map(drop)
    })
    .unwrap();
    assert_eq!(editor.section("s").unwrap().body_line_count(), 2);
    assert_eq!(editor.section("t").unwrap().body_line_count(), 0);
    let mut editor = open("g=1\n");
    atomic(&mut editor, |editor| {
        editor.section_or_insert("t").map(drop)
    })
    .unwrap();
    assert_eq!(editor.preamble().body_line_count(), 2);
}

#[test]
fn the_new_section_is_returned() {
    let mut editor = open("[s]\n");
    atomic(&mut editor, |editor: &mut Editor| {
        let mut section = editor.section_or_insert("t")?;
        assert_eq!(section.name().as_deref(), Some("t"));
        section.set("k", "v")?;
        section.append_body_lines(&["; c"])
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\n\n[t]\nk = v\n; c\n");
}

#[test]
fn creation_reports_the_first_failing_check() {
    let mut editor = open("[s]\nk=C:\\d\\");
    let result = atomic(&mut editor, |editor| {
        editor.section_or_insert("a]b").map(drop)
    });
    assert_eq!(
        result,
        Err(EditError::InvalidSectionName { name: "a]b".into() })
    );
    let result = atomic(&mut editor, |editor| {
        editor.section_or_insert("t").map(drop)
    });
    assert_eq!(result, Err(EditError::LastLineContinues));
    // An existing section is found even after a continuing last line.
    let found = atomic(&mut editor, |editor| {
        editor.section_or_insert("s").map(drop)
    });
    assert_eq!(found, Ok(()));
    in_section(&mut editor, "s", |section| section.set("k", "C:\\d")).unwrap();
    let created = atomic(&mut editor, |editor| {
        editor.section_or_insert("t").map(drop)
    });
    assert_eq!(created, Ok(()));
    assert_eq!(editor.finish(), "[s]\nk=C:\\d\n\n[t]\n");
}

#[test]
fn creating_a_section_after_clearing_the_previous_one() {
    let mut editor = open("[a]\nx=1\n[b]\ny=2\n");
    in_section(&mut editor, "b", |section| section.remove_body_lines(0..1)).unwrap();
    atomic(&mut editor, |editor| {
        editor.section_or_insert("c")?.set("z", "3")
    })
    .unwrap();
    assert_eq!(editor.finish(), "[a]\nx=1\n[b]\n\n[c]\nz = 3\n");
}

#[test]
fn a_new_section_is_separated_from_a_final_malformed_line() {
    for line in ["=bad", "  =bad", "key junk"] {
        for ending in ["", "\n"] {
            for prefix in ["", "[s]\nk=v\n\n"] {
                let source = format!("{prefix}{line}{ending}");
                assert_eq!(
                    with_section(&source, "next"),
                    format!("{prefix}{line}\n\n[next]\n")
                );
            }
        }
    }
}
