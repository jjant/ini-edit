//! Rewrites of an entry's line: `set_value`, `set_key` and `rename_key`.

use super::support::{
    ENDINGS, all_spacings, atomic, in_preamble, in_section, open, open_with, options, spacing,
    under_miri,
};
use crate::editor::{EditError, Editor, SeparatorSpacing};

/// Set `key` in section `s` of `source` to `value`, with the given options, and return the text.
fn assigned(source: &str, inline: bool, policy: &SeparatorSpacing, value: &str) -> String {
    let mut editor = open_with(source, &options(true, inline), &spacing(policy.clone()));
    in_section(&mut editor, "s", |section| {
        section.entry_at(0).unwrap().set_value(value)
    })
    .unwrap();
    editor.finish()
}

#[test]
fn preserve_keeps_the_gap_and_the_separator() {
    let preserve = SeparatorSpacing::Preserve;
    for (source, expected) in [
        ("[s]\nk = old\n", "[s]\nk = new\n"),
        ("[s]\n  k\t:  old\n", "[s]\n  k\t:  new\n"),
        ("[s]\nk=old\n", "[s]\nk=new\n"),
        // A bare key and a `key junk` line get a new gap; the junk goes.
        ("[s]\nflag\n", "[s]\nflag = new\n"),
        ("[s]\nflag   \n", "[s]\nflag = new\n"),
        ("[s]\nmy key = value\n", "[s]\nmy = new\n"),
    ] {
        assert_eq!(
            assigned(source, false, &preserve, "new"),
            expected,
            "{source:?}"
        );
    }
}

#[test]
fn compact_and_exact_rewrite_the_gap_and_keep_the_separator_character() {
    let exact = SeparatorSpacing::exact("\t", "  ").unwrap();
    for (source, compact, exact_result) in [
        (
            "[s]\n  k   =   old\n",
            "[s]\n  k=new\n",
            "[s]\n  k\t=  new\n",
        ),
        ("[s]\nk : old\n", "[s]\nk:new\n", "[s]\nk\t:  new\n"),
        ("[s]\nflag junk\n", "[s]\nflag=new\n", "[s]\nflag\t=  new\n"),
    ] {
        let compact_policy = SeparatorSpacing::Compact;
        assert_eq!(assigned(source, false, &compact_policy, "new"), compact);
        assert_eq!(assigned(source, false, &exact, "new"), exact_result);
    }
}

#[test]
fn trailing_spacing_stays_except_after_an_empty_value_under_a_policy() {
    let source = "[s]\nk = old \t\n";
    // (policy, value, expected): `empty && !Preserve` drops the old trailing spacing.
    for (policy, value, expected) in [
        (SeparatorSpacing::Compact, "", "[s]\nk=\n"),
        (SeparatorSpacing::Preserve, "", "[s]\nk =  \t\n"),
        (SeparatorSpacing::Compact, "new", "[s]\nk=new \t\n"),
        (SeparatorSpacing::Preserve, "new", "[s]\nk = new \t\n"),
    ] {
        assert_eq!(
            assigned(source, false, &policy, value),
            expected,
            "{policy:?} {value:?}"
        );
    }
}

#[test]
fn an_inline_comment_stays_after_a_value_with_text_on_its_last_line() {
    for policy in all_spacings() {
        for (value, kept) in [
            ("new", true),
            ("a \\\n b", true),
            ("", false),
            ("head \\\n", false),
        ] {
            let output = assigned("[s]\nk = old ; note\nn=1\n", true, &policy, value);
            assert_eq!(output.matches("; note").count(), 1, "{output:?}");
            if kept {
                assert!(output.contains(&format!("{value} ; note")), "{output:?}");
            } else {
                assert!(output.starts_with("[s]\n ; note\nk"), "{output:?}");
            }
        }
    }
}

#[test]
fn a_moved_comment_takes_the_entry_s_terminator_or_the_line_ending() {
    let inline = options(false, true);
    let preserve = spacing(SeparatorSpacing::Preserve);
    for ending in ["\n", "\r\n", "\r"] {
        // An unterminated entry: the comment line gets the document's line ending.
        let mut editor = open_with(&format!("[s]{ending}k = v ; note"), &inline, &preserve);
        in_section(&mut editor, "s", |section| section.set("k", "")).unwrap();
        assert_eq!(editor.finish(), format!("[s]{ending} ; note{ending}k = "));
        // A terminated entry: the comment line gets the entry's terminator.
        let mut editor = open_with(&format!("[s]\nk = v ; note{ending}"), &inline, &preserve);
        in_section(&mut editor, "s", |section| section.set("k", "")).unwrap();
        assert_eq!(editor.finish(), format!("[s]\n ; note{ending}k = {ending}"));
    }
}

#[test]
fn a_bare_key_keeps_its_inline_comment() {
    for (policy, separator) in all_spacings().into_iter().zip([" = ", "=", "\t=  "]) {
        for comment in [" ; note", "\t#", " \t ; λ"] {
            for &ending in under_miri(&ENDINGS, 2) {
                let source = format!("[s]\nflag{comment}{ending}");
                let output = assigned(&source, true, &policy, "1");
                assert_eq!(output, format!("[s]\nflag{separator}1{comment}{ending}"));
                let output = assigned(&source, true, &policy, "");
                let terminator = if ending.is_empty() { "\n" } else { ending };
                let expected = format!("[s]\n{comment}{terminator}flag{separator}{ending}");
                assert_eq!(output, expected);
            }
        }
    }
}

#[test]
fn a_value_ending_in_cr_keeps_its_own_line_break() {
    let value = "one \\\r";
    // T1: an LF terminator after the CR becomes CRLF; other terminators stay.
    for (ending, expected) in [("\n", "\r\n"), ("\r\n", "\r\n"), ("\r", "\r"), ("", "")] {
        let output = assigned(
            &format!("[s]\nk=old{ending}"),
            false,
            &SeparatorSpacing::Preserve,
            value,
        );
        assert_eq!(output, format!("[s]\nk={value}{expected}"), "{ending:?}");
    }
    let output = assigned(
        "[s]\nk=old\nnext=stay\n",
        false,
        &SeparatorSpacing::Preserve,
        value,
    );
    assert_eq!(output, format!("[s]\nk={value}\r\nnext=stay\n"));
}

#[test]
fn the_value_policy_refuses_a_final_backslash_everywhere() {
    let inline = options(false, true);
    let preserve = spacing(SeparatorSpacing::Preserve);
    for (source, parse) in [
        // As the unterminated last line, and before a kept comment, a reader would read it back.
        ("[s]\npath=old", options(false, false)),
        ("[s]\npath = old ; note\nnext=2\n", inline.clone()),
        ("[s]\npath=old\nnext=2\n", options(false, false)),
    ] {
        for value in ["C:\\dir\\", "a \\ \t", "x \\\ny\\"] {
            let mut editor = open_with(source, &parse, &preserve);
            let result = in_section(&mut editor, "s", |section| section.set("path", value));
            assert_eq!(
                result,
                Err(EditError::InvalidValue {
                    value: value.into()
                })
            );
        }
    }
    // The documented workaround: drop the trailing backslash.
    let mut editor = open("[s]\npath=old\nnext=2\n");
    in_section(&mut editor, "s", |section| section.set("path", "C:\\dir")).unwrap();
    assert_eq!(editor.finish(), "[s]\npath=C:\\dir\nnext=2\n");
}

#[test]
fn a_handle_keeps_its_entry_across_its_own_rewrites() {
    let mut editor = open_with(
        "[s]\nk = v ; note\nk = w\n",
        &options(false, true),
        &spacing(SeparatorSpacing::Preserve),
    );
    atomic(&mut editor, |editor| {
        let mut section = editor.section("s").unwrap();
        let mut entry = section.entry_at(1).unwrap();
        entry.set_value("x")?;
        entry.set_key("j")?;
        assert_eq!(
            (entry.key(), entry.value().as_deref()),
            ("j".to_owned(), Some("x"))
        );
        Ok(())
    })
    .unwrap();
    atomic(&mut editor, |editor| {
        let mut section = editor.section("s").unwrap();
        let mut entry = section.entry_at(0).unwrap();
        // The moved comment shifts the body line, not the entry's index.
        entry.set_value("")?;
        assert_eq!((entry.index(), entry.body_line()), (0, 1));
        entry.set_value("again")
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\n ; note\nk = again\nj = x\n");
}

#[test]
fn set_key_keeps_the_rest_of_the_line() {
    let parse = options(true, true);
    let preserve = spacing(SeparatorSpacing::Preserve);
    for (source, expected) in [
        ("[s]\n  old  =  v ; c\n", "[s]\n  new  =  v ; c\n"),
        ("[s]\nold\n", "[s]\nnew\n"),
        ("[s]\nold ; c", "[s]\nnew ; c"),
        ("[s]\nold junk\r\n", "[s]\nnew junk\r\n"),
    ] {
        let mut editor = open_with(source, &parse, &preserve);
        in_section(&mut editor, "s", |section| {
            section.entry_at(0).unwrap().set_key("new")
        })
        .unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
    }
}

#[test]
fn rename_key_refuses_duplicates_but_not_itself() {
    let mut editor = open("[s]\na=1\nb=2\n[t]\nc=3\n");
    assert_eq!(
        in_section(&mut editor, "s", |section| section.rename_key("x", "y")),
        Ok(false)
    );
    // An absent key is not renamed, so the new key is not checked.
    assert_eq!(
        in_section(&mut editor, "s", |section| section.rename_key("x", "a=b")),
        Ok(false)
    );
    let duplicate = Err(EditError::DuplicateKey { key: "b".into() });
    assert_eq!(
        in_section(&mut editor, "s", |section| section.rename_key("a", "b")),
        duplicate
    );
    // Renaming a key to itself succeeds without changing a byte.
    assert_eq!(
        in_section(&mut editor, "s", |section| section.rename_key("a", "a")),
        Ok(true)
    );
    // A key in another section is no duplicate.
    assert_eq!(
        in_section(&mut editor, "s", |section| section.rename_key("a", "c")),
        Ok(true)
    );
    let refused = Err(EditError::InvalidKey { key: "c d".into() });
    assert_eq!(
        in_section(&mut editor, "s", |section| section.rename_key("c", "c d")),
        refused
    );
    assert_eq!(editor.finish(), "[s]\nc=1\nb=2\n[t]\nc=3\n");
}

#[test]
fn rename_key_in_the_preamble_follows_rule_b() {
    let mut editor = open("k=1\n[s]\n");
    let result = in_preamble(&mut editor, |preamble| {
        preamble.rename_key("k", "\u{FEFF}k")
    });
    assert_eq!(result, Err(EditError::ByteOrderMarkAtStart));
    let mut editor = open("\u{FEFF}k=1\n");
    let result = in_preamble(&mut editor, |preamble| {
        preamble.rename_key("k", "\u{FEFF}k")
    });
    assert_eq!(result, Ok(true));
    assert_eq!(editor.finish(), "\u{FEFF}\u{FEFF}k=1\n");
}

#[test]
fn positional_renames_can_pass_through_duplicates() {
    let mut editor = open("[paths]\na = 1\nb = 2\nc = 3\n");
    atomic(&mut editor, |editor: &mut Editor| {
        let mut section = editor.section("paths").unwrap();
        for index in 0..section.entry_count() {
            let mut entry = section.entry_at(index).unwrap();
            let key = format!("k{}", entry.index());
            entry.set_key(&key)?;
        }
        // Through a transient duplicate: rename the first to the second's key and back.
        section.entry_at(0).unwrap().set_key("k1")?;
        section.entry_at(1).unwrap().set_key("k0")
    })
    .unwrap();
    assert_eq!(editor.finish(), "[paths]\nk1 = 1\nk0 = 2\nk2 = 3\n");
}

#[test]
fn values_can_be_continued_empty_or_unicode() {
    for value in [
        "",
        "λ",
        "#fff",
        "a;b#c",
        "=[x]:y",
        "\u{a0}",
        "\u{FEFF}",
        "a \\\n  b",
        "a\\\r\nb",
    ] {
        let mut editor = open("[s]\nk=old\nn=1\n");
        in_section(&mut editor, "s", |section| section.set("k", value)).unwrap();
        let mut section = editor.section("s").unwrap();
        assert_eq!(section.entry("k").unwrap().value().as_deref(), Some(value));
        assert_eq!(section.entry("n").unwrap().value().as_deref(), Some("1"));
    }
}
