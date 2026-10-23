//! Insertion: slots, rule T's terminators, rule C, rule B and the checks of new lines.

use super::support::{
    all_options, all_spacings, atomic, body_line_count, in_preamble, in_section, open, open_with,
    options, spacing,
};
use crate::editor::plan::{Address, terminators};
use crate::editor::{EditError, Editor, SeparatorSpacing};

/// One call on an editor, for tables of operations.
type Operation = fn(&mut Editor) -> Result<(), EditError>;

/// A row of rule T: content ends, the text before and after, the line ending, the terminators.
type TerminatorRow<'a> = (
    &'a [Option<char>],
    Option<char>,
    Option<char>,
    &'a str,
    &'a [&'a str],
);

#[test]
fn terminators_follow_rule_t() {
    let lf = "\n";
    // (content ends, before, after, ending, expected)
    let rows: [TerminatorRow<'_>; 9] = [
        // An empty line after a CR in an LF document: T1 widens its break.
        (&[None], Some('\r'), None, lf, &["\r\n"]),
        // The next empty line follows that CRLF, so it keeps LF.
        (&[None, None], Some('\r'), None, lf, &["\r\n", "\n"]),
        // A line with content ends with its own text, not the CR before it.
        (&[Some('x')], Some('\r'), None, lf, &["\n"]),
        // A content ending in CR (a CR continuation) gets CRLF.
        (&[Some('\r')], Some('x'), None, lf, &["\r\n"]),
        // In a CR document, a break before an LF line is widened (T2), only the last one.
        (
            &[Some('x'), Some('y')],
            None,
            Some('\n'),
            "\r",
            &["\r", "\r\n"],
        ),
        // Before another kind of line, or at the end, nothing changes.
        (&[Some('x')], None, Some('k'), "\r", &["\r"]),
        (&[Some('x')], None, None, "\r", &["\r"]),
        // CRLF documents: no break of theirs starts with LF or ends with CR.
        (
            &[None, Some('\r')],
            Some('\r'),
            Some('\n'),
            "\r\n",
            &["\r\n", "\r\n"],
        ),
        // T1 then T2 on one line: CRLF either way.
        (&[None], Some('\r'), Some('\n'), lf, &["\r\n"]),
    ];
    for (content_ends, before, after, ending, expected) in rows {
        let actual = terminators(content_ends, before, after, ending);
        assert_eq!(
            actual, expected,
            "{content_ends:?} {before:?} {after:?} {ending:?}"
        );
    }
}

#[test]
#[should_panic(expected = "an insertion writes at least one line")]
fn terminators_need_a_line() {
    let _ = terminators(&[], None, None, "\n");
}

#[test]
fn rule_c_terminates_the_unterminated_last_line() {
    for (source, expected) in [
        ("[s]\nk=v", "[s]\nk=v\nn = 1\n"),
        ("[s]", "[s]\nn = 1\n"),
        ("[s]\n \t", "[s]\n \t\nn = 1\n"),
        ("[s]\n; note", "[s]\n; note\nn = 1\n"),
        ("[s]\n=bad", "[s]\n=bad\nn = 1\n"),
        ("[s]\nk=", "[s]\nk=\nn = 1\n"),
        // A CR continuation reads its CR as the value's line break, so it gets CRLF.
        ("[s]\nk=a \\\r", "[s]\nk=a \\\r\r\nn = 1\n"),
    ] {
        // At the end position, after the last line whatever its kind.
        let mut editor = open(source);
        let end = body_line_count(&mut editor, "s");
        in_section(&mut editor, "s", |section| {
            section.insert_entry(end, "n", "1")
        })
        .unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
    }
}

#[test]
fn rule_c_refuses_to_continue_the_last_line() {
    let inline = options(false, true);
    let preserve = spacing(SeparatorSpacing::Preserve);
    for (source, parse) in [
        ("[s]\nk=C:\\d\\", options(false, false)),
        ("[s]\nk=a \\ \t", options(false, false)),
        // The continuation is decided before the comment split.
        ("[s]\nk = v ; c\\", inline),
    ] {
        let operations: [Operation; 5] = [
            |editor| editor.section("s").unwrap().append_entry("n", "1"),
            |editor| editor.section("s").unwrap().insert_entry(1, "n", "1"),
            |editor| editor.section("s").unwrap().set("n", "1"),
            |editor| editor.section("s").unwrap().append_body_lines(&["; c"]),
            |editor| editor.section_or_insert("t").map(drop),
        ];
        for operation in operations {
            let mut editor = open_with(source, &parse, &preserve);
            let result = atomic(&mut editor, operation);
            assert_eq!(result, Err(EditError::LastLineContinues), "{source:?}");
        }
        // Inserting before the last line needs no terminator, so it succeeds.
        let mut editor = open_with(source, &parse, &preserve);
        in_section(&mut editor, "s", |section| {
            section.insert_entry(0, "n", "1")
        })
        .unwrap();
    }
}

#[test]
fn rule_b_guards_only_the_document_start() {
    let operations: [Operation; 3] = [
        |editor| editor.preamble().insert_entry(0, "\u{FEFF}k", "1"),
        |editor| editor.preamble().insert_body_lines(0, &["\u{FEFF}x"]),
        |editor| editor.preamble().append_body_lines(&["\u{FEFF}x"]),
    ];
    for operation in operations {
        // No line before, no byte order mark, U+FEFF first: refused.
        for source in ["", "[s]\n"] {
            let mut editor = open(source);
            let result = atomic(&mut editor, operation);
            assert_eq!(result, Err(EditError::ByteOrderMarkAtStart), "{source:?}");
        }
        // After a byte order mark, the U+FEFF is text.
        let mut editor = open("\u{FEFF}[s]\n");
        atomic(&mut editor, operation).unwrap();
        assert!(editor.finish().starts_with("\u{FEFF}\u{FEFF}"));
    }
    // After another line, or without U+FEFF, nothing is refused.
    let mut editor = open("a=1\n");
    in_preamble(&mut editor, |preamble| {
        preamble.append_entry("\u{FEFF}k", "1")
    })
    .unwrap();
    assert_eq!(editor.finish(), "a=1\n\u{FEFF}k = 1\n");
    let mut editor = open("");
    in_preamble(&mut editor, |preamble| {
        preamble.insert_entry(0, "k", "\u{FEFF}")
    })
    .unwrap();
    assert_eq!(editor.finish(), "k = \u{FEFF}\n");
}

#[test]
fn positions_run_from_zero_to_the_body_line_count() {
    let mut editor = open("[s]\na=1\n\nb=2\n[t]\n");
    for (at, expected) in [
        (0, "[s]\nx\na=1\n\nb=2\n[t]\n"),
        (2, "[s]\na=1\n\nx\nb=2\n[t]\n"),
        (3, "[s]\na=1\n\nb=2\nx\n[t]\n"),
    ] {
        let mut copy = open(&editor.finish());
        in_section(&mut copy, "s", |section| {
            section.insert_body_lines(at, &["x"])
        })
        .unwrap();
        assert_eq!(copy.finish(), expected, "{at}");
    }
    for at in [4, 5, usize::MAX] {
        let result = in_section(&mut editor, "s", |section| {
            section.insert_body_lines(at, &["x"])
        });
        let expected = EditError::PositionOutOfRange {
            at,
            body_line_count: 3,
        };
        assert_eq!(result, Err(expected));
        let result = in_section(&mut editor, "s", |section| {
            section.insert_entry(at, "k", "v")
        });
        assert!(matches!(result, Err(EditError::PositionOutOfRange { .. })));
    }
    // An empty section has one position, after its header.
    let result = in_section(&mut editor, "t", |section| {
        section.insert_body_lines(1, &[])
    });
    let expected = EditError::PositionOutOfRange {
        at: 1,
        body_line_count: 0,
    };
    assert_eq!(result, Err(expected));
    in_section(&mut editor, "t", |section| {
        section.insert_body_lines(0, &["y=1"])
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\na=1\n\nb=2\n[t]\ny=1\n");
}

#[test]
fn preamble_positions_end_at_the_first_header() {
    let mut editor = open("\u{FEFF}g=1\n[s]\nk=v\n");
    let count = editor.preamble().body_line_count();
    assert_eq!(count, 1);
    in_preamble(&mut editor, |preamble| {
        preamble.insert_body_lines(1, &["; end"])
    })
    .unwrap();
    in_preamble(&mut editor, |preamble| {
        preamble.insert_body_lines(0, &["; start"])
    })
    .unwrap();
    assert_eq!(editor.finish(), "\u{FEFF}; start\ng=1\n; end\n[s]\nk=v\n");
    let result = in_preamble(&mut editor, |preamble| preamble.insert_entry(4, "k", "v"));
    let expected = EditError::PositionOutOfRange {
        at: 4,
        body_line_count: 3,
    };
    assert_eq!(result, Err(expected));
}

#[test]
fn the_content_end_is_before_trailing_blank_lines() {
    for (source, expected) in [
        // Trailing blanks stay below; malformed and comment lines are content.
        ("[s]\na=1\n\n \n[t]\n", "[s]\na=1\nn = 1\n\n \n[t]\n"),
        ("[s]\n=bad\n\n", "[s]\n=bad\nn = 1\n\n"),
        ("[s]\n; c\n", "[s]\n; c\nn = 1\n"),
        // Only blank lines: the entry goes right after the header.
        ("[s]\n\n\n", "[s]\nn = 1\n\n\n"),
        // A header only.
        ("[s]\n", "[s]\nn = 1\n"),
    ] {
        let mut editor = open(source);
        in_section(&mut editor, "s", |section| section.append_entry("n", "1")).unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
    }
    for (source, expected) in [
        // The preamble's content end, after a byte order mark and before the first header.
        ("\u{FEFF}\n[s]\n", "\u{FEFF}n = 1\n\n[s]\n"),
        ("\u{FEFF}g=1\n\n[s]\n", "\u{FEFF}g=1\nn = 1\n\n[s]\n"),
        ("", "n = 1\n"),
        ("\u{FEFF}", "\u{FEFF}n = 1\n"),
    ] {
        let mut editor = open(source);
        in_preamble(&mut editor, |preamble| preamble.append_entry("n", "1")).unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
    }
}

#[test]
fn every_insertion_reports_the_first_failing_check() {
    type Insertion = fn(&mut Editor, &str, &str) -> Result<(), EditError>;
    let insertions: [Insertion; 3] = [
        |editor, key, value| editor.section("s").unwrap().append_entry(key, value),
        |editor, key, value| editor.section("s").unwrap().insert_entry(1, key, value),
        |editor, key, value| editor.section("s").unwrap().set(key, value),
    ];
    for insertion in insertions {
        let mut editor = open("[s]\nk=v\n");
        let result = atomic(&mut editor, |editor| insertion(editor, "a=b", "v"));
        assert_eq!(result, Err(EditError::InvalidKey { key: "a=b".into() }));
        let result = atomic(&mut editor, |editor| insertion(editor, "a", " v"));
        assert_eq!(result, Err(EditError::InvalidValue { value: " v".into() }));
        atomic(&mut editor, |editor| insertion(editor, "a", "v")).unwrap();
        assert_eq!(editor.finish(), "[s]\nk=v\na = v\n");
    }
}

#[test]
fn raw_lines_must_each_be_one_body_line() {
    let invalid = |index: usize, line: &str| {
        Err(EditError::InvalidLine {
            index,
            line: line.into(),
        })
    };
    for (lines, index) in [
        (&["a\nb"][..], 0),
        (&["[x]"][..], 0),
        (&[" [x] ; c"][..], 0),
        (&["k=a \\"][..], 0),
        (&["ok", "x\r"][..], 1),
        (&["\n"][..], 0),
        (&["k = a \\", "b"][..], 0),
    ] {
        for at_end in [false, true] {
            let mut editor = open("[s]\na=1\n");
            let result = in_section(&mut editor, "s", |section| {
                if at_end {
                    section.append_body_lines(lines)
                } else {
                    section.insert_body_lines(0, lines)
                }
            });
            assert_eq!(result, invalid(index, lines[index]), "{lines:?}");
        }
    }
    // One line per element, a continued value included.
    let mut editor = open("[s]\na=1\n");
    let lines = ["k = a \\\n  b", "c=3", "", "  ", "; c", "=bad", "key junk"];
    in_section(&mut editor, "s", |section| {
        section.append_body_lines(&lines)
    })
    .unwrap();
    assert_eq!(body_line_count(&mut editor, "s"), 8);
    assert_eq!(editor.section("s").unwrap().entry_count(), 4);
}

#[test]
fn an_empty_slice_changes_nothing() {
    for source in ["[s]", "[s]\nk=v", "[s]\nk=C:\\d\\", "[s]\nk=v\n"] {
        let mut editor = open(source);
        in_section(&mut editor, "s", |section| section.append_body_lines(&[])).unwrap();
        in_section(&mut editor, "s", |section| {
            section.insert_body_lines(0, &[])
        })
        .unwrap();
        let end = body_line_count(&mut editor, "s");
        in_section(&mut editor, "s", |section| {
            section.insert_body_lines(end, &[])
        })
        .unwrap();
        assert_eq!(editor.finish(), source);
    }
}

#[test]
fn new_entries_use_the_spacing_policy() {
    for (spacing_policy, expected) in all_spacings().into_iter().zip([" = ", "=", "\t=  "]) {
        for parse in all_options() {
            let mut editor = open_with("[s]\n", &parse, &spacing(spacing_policy.clone()));
            in_section(&mut editor, "s", |section| section.append_entry("k", "v")).unwrap();
            assert_eq!(editor.finish(), format!("[s]\nk{expected}v\n"));
        }
    }
}

#[test]
fn new_lines_use_the_document_s_line_ending() {
    // (first line ending, later line endings, expected ending)
    for (first, rest, ending) in [
        ("\n", "\n", "\n"),
        ("\r\n", "\r\n", "\r\n"),
        ("\r", "\r", "\r"),
        ("\r\n", "\n", "\r\n"),
        ("\r", "\r\n", "\r"),
        ("\n", "\r\n", "\n"),
    ] {
        for terminated in [true, false] {
            let last = if terminated { rest } else { "" };
            let source = format!("[s]{first}a=1{rest}k=v{last}");
            let tail = format!(
                "[s]{first}a=1{rest}k=v{}",
                if terminated { rest } else { ending }
            );
            let mut editor = open(&source);
            in_section(&mut editor, "s", |section| section.append_entry("n", "1")).unwrap();
            assert_eq!(
                editor.finish(),
                format!("{tail}n = 1{ending}"),
                "{source:?}"
            );
            let mut editor = open(&source);
            in_section(&mut editor, "s", |section| {
                section.insert_entry(0, "n", "1")
            })
            .unwrap();
            assert_eq!(
                editor.finish(),
                format!("[s]{first}n = 1{ending}a=1{rest}k=v{last}")
            );
            let mut editor = open(&source);
            atomic(&mut editor, |editor| {
                editor.section_or_insert("t").map(drop)
            })
            .unwrap();
            assert_eq!(editor.finish(), format!("{tail}{ending}[t]{ending}"));
        }
    }
}

#[test]
fn the_line_ending_follows_the_current_first_line() {
    let mut editor = open("[a]\r\nx=1\r\n[b]\nk=v\n");
    atomic(&mut editor, |editor| {
        editor.section("a").unwrap().remove();
        Ok(())
    })
    .unwrap();
    in_section(&mut editor, "b", |section| section.append_entry("n", "1")).unwrap();
    assert_eq!(editor.finish(), "[b]\nk=v\nn = 1\n");
}

#[test]
fn an_address_where_no_section_starts_is_a_programmer_error() {
    let mut editor = open("k=v\n[s]\n");
    // A preamble line, the middle of the section's header, and the end of the document.
    for start in [0, 5, 8] {
        let address = Address::Section(rowan::TextSize::from(start));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = crate::editor::SectionEditor::new(&mut editor, address);
        }));
        assert!(result.is_err());
    }
}
