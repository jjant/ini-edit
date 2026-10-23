//! One regression test per audit reproducer. Each one failed on the 0.4 editor: the edit either
//! corrupted the document (the live tree and a parse of the saved text disagreed) or did something
//! other than its documentation said.

use super::support::{
    atomic, body_line_count, in_preamble, in_section, open, open_with, open_with_len_max, options,
    spacing,
};
use crate::ast::{AstNode, Entry};
use crate::editor::{EditError, Editor, EntryEditor, SectionEditor, SeparatorSpacing};
use crate::{ParseOptions, parse_with};

/// The keys of 7.2 that a reader would not read back.
const REFUSED_KEYS: [&str; 13] = [
    "",
    " ",
    " k",
    "k ",
    "a=b",
    "a:b",
    "a b",
    ";k",
    "#k",
    "[k]",
    "[k",
    "x\n[y]\nz",
    "k\r",
];

/// The keys of 7.2 that read back, U+FEFF ones included in the middle of a document.
const ACCEPTED_KEYS: [&str; 10] = [
    "k",
    "a.b-c",
    "λ",
    "k]",
    "k;c",
    "k#c",
    "k\\",
    "\\",
    "k\u{FEFF}",
    "\u{FEFF}k",
];

/// One call on an editor, for tables of edits.
type Edit = fn(&mut Editor) -> Result<(), EditError>;

/// A reopened editor with the same options.
fn reopen(editor: &Editor) -> Editor {
    Editor::new(
        &editor.finish(),
        editor.parse_options(),
        editor.edit_options(),
    )
    .unwrap()
}

/// The `(key, value)` pairs of section `name`, read from a parse of the saved text.
fn saved_entries(editor: &Editor, name: &str) -> Vec<(String, Option<String>)> {
    let parse = parse_with(&editor.finish(), editor.parse_options());
    let file = crate::ast::File::cast(parse.syntax()).unwrap();
    let section = file
        .sections()
        .find(|section| section.name().as_deref() == Some(name))
        .unwrap();
    section
        .entries()
        .map(|entry: Entry| (entry.key().unwrap(), entry.value()))
        .collect()
}

#[test]
fn neg_1_values_with_line_breaks_are_refused() {
    let values = [
        "a\n[evil]\nx=1",
        "a\nb",
        "a\rb",
        "a\r\nb",
        "a\n",
        "v\r",
        "\nx",
    ];
    for value in values {
        let refused = Err(EditError::InvalidValue {
            value: value.into(),
        });
        let mut editor = open("[s]\na=1\nb=2\n");
        assert_eq!(
            in_section(&mut editor, "s", |section| section.set("a", value)),
            refused
        );
        let result = in_section(&mut editor, "s", |section| section.append_entry("n", value));
        assert_eq!(result, refused);
        let result = in_section(&mut editor, "s", |section| {
            section.insert_entry(0, "n", value)
        });
        assert_eq!(result, refused);
        assert_eq!(editor.finish(), "[s]\na=1\nb=2\n");
    }
    // A continuation keeps its line break inside the value.
    let mut editor = open("[s]\na=1\nb=2\n");
    in_section(&mut editor, "s", |section| section.set("a", "a \\\n  b")).unwrap();
    assert_eq!(editor.finish(), "[s]\na=a \\\n  b\nb=2\n");
    assert_eq!(
        saved_entries(&editor, "s")[1],
        ("b".into(), Some("2".into()))
    );
}

#[test]
fn neg_2_keys_that_do_not_read_back_are_refused() {
    for key in REFUSED_KEYS {
        let refused = Err(EditError::InvalidKey { key: key.into() });
        let mut editor = open("[s]\na=1\nb=2\n");
        assert_eq!(
            in_section(&mut editor, "s", |section| section.set(key, "v")),
            refused
        );
        let result = in_section(&mut editor, "s", |section| {
            section.entry_at(0).unwrap().set_key(key)
        });
        assert_eq!(result, refused);
        let result = in_section(&mut editor, "s", |section| section.rename_key("a", key));
        assert_eq!(result.map(drop), refused);
        assert_eq!(editor.finish(), "[s]\na=1\nb=2\n");
    }
    for key in ACCEPTED_KEYS {
        let mut editor = open("[s]\na=1\n");
        in_section(&mut editor, "s", |section| section.set(key, "v")).unwrap();
        let value = editor.section("s").unwrap().entry(key).unwrap().value();
        assert_eq!(value.as_deref(), Some("v"), "{key:?}");
        assert_eq!(saved_entries(&editor, "s")[1].0, key);
        in_section(&mut editor, "s", |section| {
            section.entry_at(0).unwrap().set_key(key)
        })
        .unwrap();
        assert_eq!(saved_entries(&editor, "s")[0].0, key);
    }
}

#[test]
fn neg_3_section_names_that_do_not_read_back_are_refused() {
    for name in ["", " a", "a ", "\ta", "a]b", "a]", "a\nb", "a\r"] {
        let mut editor = open("[s]\nk=1\n");
        let result = atomic(&mut editor, |editor| {
            editor.section_or_insert(name).map(drop)
        });
        assert_eq!(
            result,
            Err(EditError::InvalidSectionName { name: name.into() })
        );
        assert!(editor.section(name).is_none());
        assert_eq!(editor.finish(), "[s]\nk=1\n");
    }
    for name in ["a", "my section", "[a", "a ; c", "a=b", "\u{FEFF}"] {
        let mut editor = open("[s]\nk=1\n");
        atomic(&mut editor, |editor| {
            editor.section_or_insert(name).map(drop)
        })
        .unwrap();
        assert_eq!(editor.section(name).unwrap().name().as_deref(), Some(name));
        // Looking it up again finds it instead of adding a second one.
        let mut reopened = reopen(&editor);
        atomic(&mut reopened, |editor| {
            editor.section_or_insert(name).map(drop)
        })
        .unwrap();
        assert_eq!(reopened.section_count(), 2);
    }
}

#[test]
fn neg_4_exact_spacing_must_be_spaces_and_tabs() {
    for text in ["x", "=", ":", "\u{a0}", "\n", " ;", "\\\n"] {
        let refused = Err(EditError::InvalidSpacing { text: text.into() });
        assert_eq!(SeparatorSpacing::exact(text, ""), refused);
        assert_eq!(SeparatorSpacing::exact("", text), refused);
    }
    let exact = SeparatorSpacing::exact("\t", "  ").unwrap();
    let mut editor = open_with("[s]\na=1\n", &options(false, false), &spacing(exact));
    in_section(&mut editor, "s", |section| section.set("a", "2")).unwrap();
    in_section(&mut editor, "s", |section| section.append_entry("k", "v")).unwrap();
    assert_eq!(editor.finish(), "[s]\na\t=  2\nk\t=  v\n");
}

#[test]
fn neg_5_lossy_whitespace_and_inline_markers_are_refused() {
    for value in [" v", "v ", "\tv", "v\t", " ", "\t"] {
        let mut editor = open("[s]\na=1\n");
        let result = in_section(&mut editor, "s", |section| section.set("a", value));
        assert_eq!(
            result,
            Err(EditError::InvalidValue {
                value: value.into()
            })
        );
    }
    let inline = options(false, true);
    let preserve = spacing(SeparatorSpacing::Preserve);
    let mut editor = open_with("[s]\na=old ; note\n", &inline, &preserve);
    for value in ["a ; c", "x ; y", "v #x", "a ;"] {
        let refused = Err(EditError::InvalidValue {
            value: value.into(),
        });
        assert_eq!(
            in_section(&mut editor, "s", |section| section.set("a", value)),
            refused
        );
        let result = in_section(&mut editor, "s", |section| section.append_entry("n", value));
        assert_eq!(result, refused);
    }
    for value in ["a;c", "#fff"] {
        in_section(&mut editor, "s", |section| section.set("a", value)).unwrap();
        let entry = &saved_entries(&editor, "s")[0];
        assert_eq!(entry.1.as_deref(), Some(value));
    }
    // Without inline comments the same text is an ordinary value.
    let mut editor = open("[s]\na=old\n");
    in_section(&mut editor, "s", |section| section.set("a", "x ; y")).unwrap();
    assert_eq!(editor.finish(), "[s]\na=x ; y\n");
}

#[test]
fn neg_6_the_rename_guard_cannot_be_bypassed() {
    for key in [" b", "b ", "b\t", "b=x", "b:x", "b junk"] {
        let mut editor = open("[s]\na=1\nb=2\n");
        let result = in_section(&mut editor, "s", |section| section.rename_key("a", key));
        assert_eq!(result, Err(EditError::InvalidKey { key: key.into() }));
    }
    // A raw line is a real entry, so the guard sees it, live and after reopening alike.
    let mut editor = open("[s]\na=1\n");
    in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["b=raw"])
    })
    .unwrap();
    let duplicate = Err(EditError::DuplicateKey { key: "b".into() });
    assert_eq!(
        in_section(&mut editor, "s", |section| section.rename_key("a", "b")),
        duplicate
    );
    let mut reopened = reopen(&editor);
    assert_eq!(
        in_section(&mut reopened, "s", |section| section.rename_key("a", "b")),
        duplicate
    );
    assert_eq!(editor.finish(), "[s]\na=1\nb=raw\n");
}

#[test]
fn neg_7_raw_lines_are_real_lines() {
    for (lines, index) in [
        (&["; one\n; two"][..], 0),
        (&["k=2\n[b]", "y=3"][..], 0),
        (&["k=v\\"][..], 0),
    ] {
        let mut editor = open("[s]\na=1\n");
        let result = in_section(&mut editor, "s", |section| section.append_body_lines(lines));
        let line = lines[index].to_owned();
        assert_eq!(result, Err(EditError::InvalidLine { index, line }));
    }
    // A raw entry is an entry: `set` assigns it instead of adding a duplicate.
    let mut editor = open("[s]\na=1\n");
    in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["k=old", "; note"])
    })
    .unwrap();
    in_section(&mut editor, "s", |section| section.set("k", "new")).unwrap();
    assert_eq!(editor.finish(), "[s]\na=1\nk=new\n; note\n");
    let file = editor.file();
    let section = file.sections().next().unwrap();
    let comments: Vec<_> = section
        .comment_lines()
        .filter_map(|line| line.text())
        .collect();
    assert_eq!(comments, ["; note"]);
}

#[test]
fn neg_8_lines_above_a_header_belong_to_the_previous_container() {
    let mut live = open("[a]\nx=1\n[b]\ny=2\n");
    in_section(&mut live, "a", |section| {
        section.insert_body_lines(1, &["; c"])
    })
    .unwrap();
    let mut reopened = reopen(&live);
    for editor in [&mut live, &mut reopened] {
        in_section(editor, "a", |section| section.remove_body_lines(1..2)).unwrap();
    }
    assert_eq!(live.finish(), reopened.finish());
    assert_eq!(live.finish(), "[a]\nx=1\n[b]\ny=2\n");

    let mut live = open("[s]\na=1\n");
    in_preamble(&mut live, |preamble| {
        preamble.insert_body_lines(0, &["; top"])
    })
    .unwrap();
    let mut reopened = reopen(&live);
    for editor in [&mut live, &mut reopened] {
        in_preamble(editor, |preamble| preamble.remove_body_lines(0..1)).unwrap();
    }
    assert_eq!(live.finish(), reopened.finish());
    assert_eq!(live.finish(), "[s]\na=1\n");
}

#[test]
fn neg_9_a_continued_entry_is_one_line() {
    let mut editor = open("[s]\nk = a \\\n  b\nnext=1\n");
    assert_eq!(body_line_count(&mut editor, "s"), 2);
    in_section(&mut editor, "s", |section| section.remove_body_lines(1..2)).unwrap();
    assert_eq!(editor.finish(), "[s]\nk = a \\\n  b\n");
}

#[test]
fn neg_10_a_header_is_not_a_body_line() {
    let mut editor = open("[a]\nx=1\n[b]\ny=2\n");
    in_section(&mut editor, "b", |section| section.remove_body_lines(0..1)).unwrap();
    assert_eq!(editor.finish(), "[a]\nx=1\n[b]\n");
    let result = in_section(&mut editor, "b", |section| section.remove_body_lines(0..1));
    let expected = EditError::RangeOutOfRange {
        start: 0,
        end: 1,
        body_line_count: 0,
    };
    assert_eq!(result, Err(expected));
}

#[test]
fn neg_11_and_dx_7_positions_are_computed_from_the_tree() {
    let mut editor = open("[s]\na=1\nb=2\nc=3\n");
    assert_eq!(
        in_section(&mut editor, "s", |section| section.remove_entry("a")),
        Ok(true)
    );
    let mut section = editor.section("s").unwrap();
    let entry = section.entry("c").unwrap();
    assert_eq!((entry.index(), entry.body_line()), (1, 1));
}

#[test]
fn neg_12_duplicates_use_the_first_match() {
    let mut editor = open("[a]\nx=1\n[a]\ny=2\n[s]\nk=1\nk=2\n");
    in_section(&mut editor, "a", |section| section.set("y", "3")).unwrap();
    atomic(&mut editor, |editor| {
        editor.section_at(1).unwrap().set("y", "4")
    })
    .unwrap();
    assert_eq!(
        in_section(&mut editor, "s", |section| section.remove_entry("k")),
        Ok(true)
    );
    let mut section = editor.section("s").unwrap();
    assert_eq!(section.entry("k").unwrap().value().as_deref(), Some("2"));
    assert_eq!(editor.finish(), "[a]\nx=1\ny = 3\n[a]\ny=4\n[s]\nk=2\n");
}

#[test]
fn neg_m1_the_preamble_is_editable() {
    let mut editor = open("root = true\n[*]\nx=1\n");
    in_preamble(&mut editor, |preamble| preamble.set("root", "false")).unwrap();
    assert_eq!(editor.finish(), "root = false\n[*]\nx=1\n");
    // Preamble lines go after a byte order mark.
    let mut editor = super::support::open("\u{FEFF}[s]\n");
    in_preamble(&mut editor, |preamble| preamble.append_entry("g", "1")).unwrap();
    assert_eq!(editor.finish(), "\u{FEFF}g = 1\n[s]\n");
    // `""` names no section, so it creates none.
    assert!(editor.section("").is_none());
    assert_eq!(editor.section_count(), 1);
}

#[test]
fn safe_5_a_key_starting_with_u_feff_is_text_in_the_middle_of_a_document() {
    let mut editor = open("[s]\nk=1\n");
    in_section(&mut editor, "s", |section| section.set("\u{FEFF}k", "2")).unwrap();
    assert_eq!(editor.finish(), "[s]\nk=1\n\u{FEFF}k = 2\n");
    assert_eq!(saved_entries(&editor, "s")[1].0, "\u{FEFF}k");
}

/// Insertions that grow a document: a document, its limit, an edit that reaches the limit
/// exactly, and one that goes one byte past it.
fn growing_insertions() -> [(&'static str, usize, Edit, Edit); 3] {
    [
        (
            "[s]\n",
            10,
            |editor| editor.section("s").unwrap().append_entry("k", "1"),
            |editor| editor.section("s").unwrap().append_entry("k", "12"),
        ),
        // The terminator added to the unterminated last line counts.
        (
            "[s]\nk=v",
            9,
            |editor| editor.section("s").unwrap().append_body_lines(&[""]),
            |editor| editor.section("s").unwrap().append_body_lines(&["x"]),
        ),
        (
            "[s]\nk=v\n",
            13,
            |editor| editor.section_or_insert("t").map(drop),
            |editor| editor.section_or_insert("tt").map(drop),
        ),
    ]
}

/// Section `s`, which every growing edit's document has.
fn section_s(editor: &mut Editor) -> SectionEditor<'_> {
    editor.section("s").unwrap()
}

/// Run `edit` on the first entry of section `s`.
fn on_first_entry(
    editor: &mut Editor,
    edit: impl FnOnce(&mut EntryEditor<'_>) -> Result<(), EditError>,
) -> Result<(), EditError> {
    edit(&mut section_s(editor).entry_at(0).unwrap())
}

/// Rewrites that grow a document, as [`growing_insertions`].
fn growing_rewrites() -> [(&'static str, usize, Edit, Edit); 4] {
    [
        (
            "[s]\nk=v\n",
            9,
            |editor| section_s(editor).set("k", "vv"),
            |editor| section_s(editor).set("k", "vvv"),
        ),
        (
            "[s]\nk=v\n",
            9,
            |editor| on_first_entry(editor, |entry| entry.set_key("kk")),
            |editor| on_first_entry(editor, |entry| entry.set_key("kkk")),
        ),
        (
            "[s]\nk=v\n",
            12,
            |editor| section_s(editor).rename_key("k", "kkkkk").map(drop),
            |editor| section_s(editor).rename_key("k", "kkkkkk").map(drop),
        ),
        // A comment moved above its entry counts too.
        (
            "[s]\nk=v ;c\n",
            15,
            |editor| section_s(editor).set("k", "a \\\n"),
            |editor| section_s(editor).set("k", "ab \\\n"),
        ),
    ]
}

#[test]
fn sql_1_every_growth_is_bounded_by_the_limit() {
    let parse = ParseOptions::default();
    let edit = spacing(SeparatorSpacing::Preserve);
    assert!(Editor::with_len_max("[s]\n", &parse, &edit, 4).is_ok());
    let too_large = Editor::with_len_max("[s]\n", &parse, &edit, 3).map(drop);
    assert_eq!(
        too_large,
        Err(EditError::DocumentTooLarge { len: 4, max: 3 })
    );
    let growing = growing_insertions().into_iter().chain(growing_rewrites());
    for (source, len_max, at_limit, past_limit) in growing {
        let parse = options(false, true);
        let mut editor = Editor::with_len_max(source, &parse, &edit, len_max).unwrap();
        let result = atomic(&mut editor, past_limit);
        assert!(
            matches!(result, Err(EditError::DocumentTooLarge { .. })),
            "{source:?}"
        );
        let mut editor = Editor::with_len_max(source, &parse, &edit, len_max).unwrap();
        atomic(&mut editor, at_limit).unwrap();
        assert_eq!(
            editor.finish().len(),
            len_max,
            "{source:?}: {:?}",
            editor.finish()
        );
    }
    // A rewrite that shrinks the document succeeds at the limit.
    let mut editor = open_with_len_max("[s]\nk=old\n", 10);
    in_section(&mut editor, "s", |section| section.set("k", "")).unwrap();
    assert_eq!(editor.finish(), "[s]\nk=\n");
}

#[test]
fn dx_11_lookups_have_no_side_effects() {
    let mut editor = open("[s]\nk=1\n");
    assert!(editor.section("typo").is_none());
    assert!(editor.section("s").unwrap().entry("typo").is_none());
    assert_eq!(editor.finish(), "[s]\nk=1\n");
}

#[test]
fn judges_raw_elements_cannot_join_across_their_boundary() {
    let mut editor = open("[s]\nk=v\n");
    let result = in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["; raw\r", ""])
    });
    let line = "; raw\r".to_owned();
    assert_eq!(result, Err(EditError::InvalidLine { index: 0, line }));
}

#[test]
fn judges_an_empty_key_on_an_unterminated_bare_key_is_an_error() {
    let parse = options(true, false);
    let preserve = spacing(SeparatorSpacing::Preserve);
    for source in ["flag", "[s]\nflag"] {
        let refused = Err(EditError::InvalidKey { key: String::new() });
        let mut editor = open_with(source, &parse, &preserve);
        let in_container = |editor: &mut Editor| {
            if source.starts_with('[') {
                editor.section("s").unwrap().rename_key("flag", "")
            } else {
                editor.preamble().rename_key("flag", "")
            }
        };
        assert_eq!(atomic(&mut editor, in_container).map(drop), refused);
        let set_key = |editor: &mut Editor| {
            let mut container = if source.starts_with('[') {
                editor.section("s").unwrap()
            } else {
                editor.preamble()
            };
            container.entry_at(0).unwrap().set_key("")
        };
        assert_eq!(atomic(&mut editor, set_key), refused);
    }
}

#[test]
fn judges_rewriting_an_entry_keeps_the_next_line() {
    let mut editor = open("[s]\nkey = old\nnext=1\n");
    in_section(&mut editor, "s", |section| section.set("key", "new")).unwrap();
    assert_eq!(editor.finish(), "[s]\nkey = new\nnext=1\n");
}

#[test]
fn judges_the_line_ending_is_read_as_the_lexer_reads_it() {
    // (document, line ending): the first line break may be inside a continued value.
    for (source, ending) in [
        ("[s]\r\nk=v\n", "\r\n"),
        ("[s]\rk=v\n", "\r"),
        ("[s]\nk=v\r\n", "\n"),
        ("k=a \\\r\nb\nz=1\n", "\r\n"),
        ("k=a \\\rb\nz=1\n", "\r"),
        ("[s]", "\n"),
    ] {
        let editor = open(source);
        assert_eq!(editor.line_ending(), ending, "{source:?}");
    }
}
