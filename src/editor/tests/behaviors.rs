//! Behaviors pinned by the 0.4 editor's tests, ported to body-line positions. Where a rule of
//! 0.5 changes the expected bytes, the comment names it.

use super::support::{
    ENDINGS, all_options, all_spacings, atomic, body_line_count, in_section, open, open_with,
    options, spacing, under_miri,
};
use crate::editor::{EditError, Editor, SeparatorSpacing};

/// The value of the first entry keyed `key` in section `s`, read from a reopened editor.
fn reopened_value(editor: &Editor, key: &str) -> Option<String> {
    let mut reopened = Editor::new(
        &editor.finish(),
        editor.parse_options(),
        editor.edit_options(),
    )
    .unwrap();
    let mut section = reopened.section("s").unwrap();
    section.entry(key).unwrap().value()
}

#[test]
fn assigning_a_key_followed_by_junk_replaces_the_junk() {
    for (policy, separator) in all_spacings().into_iter().zip([" = ", "=", "\t=  "]) {
        for (key, junk) in [
            ("key", " junk"),
            ("key", "\t\tjunk"),
            ("my", " key = value"),
        ] {
            for &ending in under_miri(&ENDINGS, 2) {
                for (by_handle, value) in [(false, "v"), (true, "v"), (false, ""), (true, "")] {
                    let source = format!("[s]\n{key}{junk}{ending}");
                    let mut editor =
                        open_with(&source, &options(false, false), &spacing(policy.clone()));
                    in_section(&mut editor, "s", |section| {
                        if by_handle {
                            section.entry_at(0).unwrap().set_value(value)
                        } else {
                            section.set(key, value)
                        }
                    })
                    .unwrap();
                    assert_eq!(
                        editor.finish(),
                        format!("[s]\n{key}{separator}{value}{ending}")
                    );
                }
            }
        }
    }
}

#[test]
fn every_malformed_line_is_one_body_line() {
    for line in ["=bad", "  =bad", ":", "key junk", "my key = value"] {
        for &ending in under_miri(&ENDINGS[..3], 1) {
            let source = format!("[s]{ending}{line}{ending}b=2{ending}c=3{ending}");
            let mut editor = open(&source);
            assert_eq!(body_line_count(&mut editor, "s"), 3);
            in_section(&mut editor, "s", |section| section.remove_body_lines(1..2)).unwrap();
            assert_eq!(
                editor.finish(),
                format!("[s]{ending}{line}{ending}c=3{ending}")
            );
            let mut editor = open(&source);
            in_section(&mut editor, "s", |section| section.remove_body_lines(0..1)).unwrap();
            assert_eq!(
                editor.finish(),
                format!("[s]{ending}b=2{ending}c=3{ending}")
            );
            for (at, expected) in [
                (
                    1,
                    format!("[s]{ending}{line}{ending}; note{ending}b=2{ending}c=3{ending}"),
                ),
                (
                    2,
                    format!("[s]{ending}{line}{ending}b=2{ending}; note{ending}c=3{ending}"),
                ),
            ] {
                let mut editor = open(&source);
                in_section(&mut editor, "s", |section| {
                    section.insert_body_lines(at, &["; note"])
                })
                .unwrap();
                assert_eq!(editor.finish(), expected);
            }
            let mut editor = open(&format!("[s]{ending}{line}"));
            in_section(&mut editor, "s", |section| section.append_entry("n", "1")).unwrap();
            assert_eq!(
                editor.finish(),
                format!("[s]{ending}{line}{ending}n = 1{ending}")
            );
        }
    }
}

#[test]
fn appended_lines_go_after_malformed_lines_and_before_trailing_blanks() {
    for (source, expected) in [
        ("[s]\n=bad", "[s]\n=bad\nb = 2\n"),
        ("[s]\n=bad\n", "[s]\n=bad\nb = 2\n"),
        ("[s]\n=bad\n\n", "[s]\n=bad\nb = 2\n\n"),
        (
            "[s]\na = 1\n; trailing note\n\n[next]\n",
            "[s]\na = 1\n; trailing note\nb = 2\n\n[next]\n",
        ),
    ] {
        let mut editor = open(source);
        in_section(&mut editor, "s", |section| section.append_entry("b", "2")).unwrap();
        assert_eq!(editor.finish(), expected, "{source:?}");
    }
}

#[test]
fn repeated_insertion_reuses_an_added_terminator() {
    let mut editor = open("[s]");
    in_section(&mut editor, "s", |section| {
        section.insert_entry(0, "a", "1")
    })
    .unwrap();
    in_section(&mut editor, "s", |section| {
        section.insert_entry(0, "b", "2")
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\nb = 2\na = 1\n");
}

#[test]
fn a_line_above_a_header_after_a_cr_line_keeps_the_breaks_separate() {
    // 0.4 also inserted this line inside `[b]`; it is the end of `[a]` (D5), and T1 makes its
    // line break CRLF instead of rewriting a neighbor.
    let mut live = open("[a]\nk=v\r[b]\nx=1\n");
    let end = body_line_count(&mut live, "a");
    in_section(&mut live, "a", |section| {
        section.insert_body_lines(end, &[""])
    })
    .unwrap();
    assert_eq!(live.finish(), "[a]\nk=v\r\r\n[b]\nx=1\n");
    assert_eq!(body_line_count(&mut live, "a"), 2);
}

#[test]
fn insertion_after_a_cr_line_keeps_its_line_break_separate() {
    for before in ["\r", "\n", "\r\n"] {
        for (raw, line) in [("", ""), ("x", "x")] {
            // The last line ends with CR, whatever the document's first line ending.
            let source = format!("[s]{before}k=v\r");
            let mut editor = open(&source);
            in_section(&mut editor, "s", |section| {
                section.append_body_lines(&[raw])
            })
            .unwrap();
            // T1: an LF break right after the CR would read together with it.
            let terminator = if raw.is_empty() && before != "\r" {
                "\r\n"
            } else {
                before
            };
            let expected = format!("{source}{line}{terminator}");
            assert_eq!(editor.finish(), expected, "{before:?} {raw:?}");
            assert_eq!(body_line_count(&mut editor, "s"), 2);
        }
    }
}

#[test]
fn insertion_before_an_lf_blank_line_keeps_the_breaks_separate() {
    for blank in ["\n", "\r", "\r\n", " \n"] {
        for raw in ["", "x"] {
            // A CR document; a CRLF line comes before the blank line, so the two stay apart.
            let source = format!("[s]\rk=v\r\n{blank}[t]\rnext=1\r");
            let mut editor = open(&source);
            in_section(&mut editor, "s", |section| {
                section.insert_body_lines(1, &[raw])
            })
            .unwrap();
            // T2: a CR break right before an LF line would read together with it.
            let terminator = if blank == "\n" { "\r\n" } else { "\r" };
            let expected = format!("[s]\rk=v\r\n{raw}{terminator}{blank}[t]\rnext=1\r");
            assert_eq!(editor.finish(), expected, "{blank:?} {raw:?}");
            assert_eq!(body_line_count(&mut editor, "s"), 3);
        }
    }
}

#[test]
fn removing_an_appended_line_matches_reloaded_editing() {
    for source in ["[s]", "[s]\na=1", "[s]\n; note"] {
        type Append = fn(&mut Editor) -> Result<(), EditError>;
        let appends: [Append; 2] = [
            |editor| editor.section("s").unwrap().append_entry("b", "2"),
            |editor| editor.section("s").unwrap().append_body_lines(&["; tail"]),
        ];
        for append in appends {
            let mut live = open(source);
            atomic(&mut live, append).unwrap();
            let mut reloaded = open(&live.finish());
            for editor in [&mut live, &mut reloaded] {
                let end = body_line_count(editor, "s");
                in_section(editor, "s", |section| {
                    section.remove_body_lines(end - 1..end)
                })
                .unwrap();
            }
            assert_eq!(live.finish(), reloaded.finish(), "{source:?}");
            assert_eq!(live.finish(), format!("{source}\n"));
        }
    }
}

#[test]
fn clearing_a_value_keeps_its_comment_out_of_the_saved_value() {
    let parse = options(false, true);
    for &ending in under_miri(&ENDINGS, 2) {
        for &marker in under_miri(&[";", "#"], 1) {
            for separator in ["=", ":"] {
                for policy in all_spacings() {
                    for by_handle in [false, true] {
                        let source = format!("[s]\nk {separator} old \t{marker} keep{ending}");
                        let mut editor = open_with(&source, &parse, &spacing(policy.clone()));
                        in_section(&mut editor, "s", |section| {
                            if by_handle {
                                section.entry_at(0).unwrap().set_value("")
                            } else {
                                section.set("k", "")
                            }
                        })
                        .unwrap();
                        assert_eq!(reopened_value(&editor, "k").as_deref(), Some(""));
                        let (before, after) = match &policy {
                            SeparatorSpacing::Preserve => (" ", " "),
                            SeparatorSpacing::Compact => ("", ""),
                            SeparatorSpacing::Exact { before, after } => {
                                (before.as_str(), after.as_str())
                            }
                        };
                        let comment_ending = if ending.is_empty() { "\n" } else { ending };
                        let comment = format!(" \t{marker} keep{comment_ending}");
                        let entry = format!("k{before}{separator}{after}{ending}");
                        let expected = format!("[s]\n{comment}{entry}");
                        assert_eq!(editor.finish(), expected);
                        in_section(&mut editor, "s", |section| section.set("k", "again")).unwrap();
                        assert_eq!(editor.finish().matches("keep").count(), 1);
                    }
                }
            }
        }
    }
}

#[test]
fn a_continuation_ending_on_an_empty_line_keeps_the_comment_separate() {
    let parse = options(false, true);
    for &ending in under_miri(&ENDINGS[..3], 1) {
        for policy in all_spacings() {
            for marker in [";", "#"] {
                for &source_ending in under_miri(&ENDINGS, 2) {
                    let source = format!("[s]\nk: old \t{marker} keep{source_ending}");
                    let mut editor = open_with(&source, &parse, &spacing(policy.clone()));
                    let value = format!("one \\{ending}");
                    in_section(&mut editor, "s", |section| section.set("k", &value)).unwrap();
                    assert_eq!(
                        reopened_value(&editor, "k").as_deref(),
                        Some(value.as_str())
                    );
                    let output = editor.finish();
                    assert_eq!(output.matches("keep").count(), 1);
                    assert!(output.contains(&format!("\t{marker} keep")));
                }
            }
        }
    }
}

#[test]
fn whitespace_only_values_are_refused_rather_than_trimmed() {
    // 0.4 wrote them and a reader trimmed them; they cannot be read back.
    let parse = options(false, true);
    for value in [" \t", "one \\\n \t"] {
        let mut editor = open_with(
            "[s]\nk=old ; note\n",
            &parse,
            &spacing(SeparatorSpacing::Preserve),
        );
        let result = in_section(&mut editor, "s", |section| section.set("k", value));
        assert_eq!(
            result,
            Err(EditError::InvalidValue {
                value: value.into()
            })
        );
    }
}

#[test]
fn empty_value_spacing_is_stable_across_reloads() {
    for policy in all_spacings() {
        for separator in ["=", ":"] {
            for (before, after, trailing) in [
                ("", "", " \t"),
                ("\t", "  ", "\t "),
                (" ", "\t", ""),
                ("", "", ""),
            ] {
                for &ending in under_miri(&ENDINGS, 2) {
                    let source = format!(
                        "[s]\nuntouched = value  \nκ{before}{separator}{after}old{trailing}{ending}"
                    );
                    let edit = spacing(policy.clone());
                    let mut live = open_with(&source, &options(false, false), &edit);
                    in_section(&mut live, "s", |section| section.set("κ", "")).unwrap();
                    let (expected_before, expected_after) = match &policy {
                        SeparatorSpacing::Preserve => (before, format!("{after}{trailing}")),
                        SeparatorSpacing::Compact => ("", String::new()),
                        SeparatorSpacing::Exact { before, after } => {
                            (before.as_str(), after.as_str().to_owned())
                        }
                    };
                    let entry = format!("κ{expected_before}{separator}{expected_after}{ending}");
                    let expected = format!("[s]\nuntouched = value  \n{entry}");
                    assert_eq!(live.finish(), expected);
                    let mut reopened = open_with(&live.finish(), &options(false, false), &edit);
                    for editor in [&mut live, &mut reopened] {
                        in_section(editor, "s", |section| section.set("κ", "next")).unwrap();
                    }
                    assert_eq!(live.finish(), reopened.finish());
                }
            }
        }
    }
}

#[test]
fn a_new_entry_with_a_cr_value_keeps_the_next_line() {
    let value = "one \\\r";
    for (at_end, expected) in [
        (true, format!("[s]\nk=old\nnext=stay\nadded = {value}\r\n")),
        (false, format!("[s]\nadded = {value}\r\nk=old\nnext=stay\n")),
    ] {
        let mut editor = open("[s]\nk=old\nnext=stay\n");
        in_section(&mut editor, "s", |section| {
            if at_end {
                section.append_entry("added", value)
            } else {
                section.insert_entry(0, "added", value)
            }
        })
        .unwrap();
        assert_eq!(editor.finish(), expected);
        assert_eq!(reopened_value(&editor, "next").as_deref(), Some("stay"));
        assert_eq!(reopened_value(&editor, "added").as_deref(), Some(value));
    }
}

#[test]
fn a_new_section_after_a_cr_continuation_keeps_the_value() {
    let mut editor = open("[s]\nk=one \\\r");
    atomic(&mut editor, |editor| {
        editor.section_or_insert("other")?.set("next", "stay")
    })
    .unwrap();
    assert_eq!(
        editor.finish(),
        "[s]\nk=one \\\r\r\n\n[other]\nnext = stay\n"
    );
    assert_eq!(reopened_value(&editor, "k").as_deref(), Some("one \\\r"));
}

#[test]
fn renames_keep_inline_comments() {
    let parse = options(true, true);
    for (source, old, new, expected) in [
        (
            "[s]\nold = v ; note\n",
            "old",
            "new",
            "[s]\nnew = v ; note\n",
        ),
        ("[s]\nflag ; note\n", "flag", "other", "[s]\nother ; note\n"),
    ] {
        let mut editor = open_with(source, &parse, &spacing(SeparatorSpacing::Preserve));
        assert_eq!(
            in_section(&mut editor, "s", |section| section.rename_key(old, new)),
            Ok(true)
        );
        assert_eq!(editor.finish(), expected);
    }
}

#[test]
fn spacing_policies_apply_only_to_touched_entries() {
    let mut editor = open_with(
        "[s]\n  changed   =   old\nuntouched : keep\ncolon: old\n",
        &options(false, false),
        &spacing(SeparatorSpacing::Compact),
    );
    in_section(&mut editor, "s", |section| section.set("changed", "new")).unwrap();
    in_section(&mut editor, "s", |section| section.set("missing", "added")).unwrap();
    in_section(&mut editor, "s", |section| {
        section.insert_entry(1, "inserted", "here")
    })
    .unwrap();
    in_section(&mut editor, "s", |section| {
        section.entry_at(3).unwrap().set_value("new")
    })
    .unwrap();
    assert_eq!(
        editor.finish(),
        "[s]\n  changed=new\ninserted=here\nuntouched : keep\ncolon:new\nmissing=added\n"
    );
}

#[test]
fn parse_and_edit_options_combine() {
    let mut editor = open_with(
        "[s]\nflag   \nvalue = old   ; keep\nuntouched = yes\n",
        &options(true, true),
        &spacing(SeparatorSpacing::Compact),
    );
    in_section(&mut editor, "s", |section| section.set("flag", "on")).unwrap();
    in_section(&mut editor, "s", |section| section.set("value", "new")).unwrap();
    assert_eq!(
        editor.finish(),
        "[s]\nflag=on\nvalue=new   ; keep\nuntouched = yes\n"
    );
    // Every parse option set reads the same rewritten document.
    for parse in all_options() {
        let reopened = open_with(&editor.finish(), &parse, editor.edit_options());
        assert_eq!(reopened.finish(), editor.finish());
    }
}

#[test]
fn values_with_separators_and_markers_are_written_verbatim() {
    for value in ["a=b=c", "val;ue#here", "http://x/#f", "12:30:00"] {
        let mut editor = open("[s]\nk = old\n");
        in_section(&mut editor, "s", |section| section.set("k", value)).unwrap();
        assert_eq!(editor.finish(), format!("[s]\nk = {value}\n"));
        assert_eq!(reopened_value(&editor, "k").as_deref(), Some(value));
    }
}
