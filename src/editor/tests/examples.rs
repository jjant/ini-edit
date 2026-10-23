//! The worked examples of the editor's contract: one test per example, each checking the exact
//! bytes, plus C1 and C2 through `atomic`.

use super::support::{
    atomic, body_line_count, in_preamble, in_section, open, open_with, open_with_len_max, options,
    spacing,
};
use crate::editor::{EditError, SeparatorSpacing};

#[test]
fn a_value_with_a_header_is_refused() {
    let mut editor = open("[s]\na=1\nb=2\n");
    let result = in_section(&mut editor, "s", |section| {
        section.set("a", "a\n[evil]\nx=1")
    });
    assert_eq!(
        result,
        Err(EditError::InvalidValue {
            value: "a\n[evil]\nx=1".into()
        })
    );
    assert_eq!(editor.finish(), "[s]\na=1\nb=2\n");
}

#[test]
fn raw_entries_are_entries_and_a_raw_blank_stays_below() {
    let mut editor = open("[s]\na=1\n");
    in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["k=2", "=bad", ""])
    })
    .unwrap();
    in_section(&mut editor, "s", |section| section.set("k", "3")).unwrap();
    in_section(&mut editor, "s", |section| section.append_entry("n", "1")).unwrap();
    assert_eq!(editor.finish(), "[s]\na=1\nk=3\n=bad\nn = 1\n\n");
}

#[test]
fn lines_above_a_header_are_the_end_of_the_previous_section() {
    let mut editor = open("[a]\nx=1\n[b]\ny=2\n");
    in_section(&mut editor, "a", |section| {
        section.insert_body_lines(1, &["; c"])
    })
    .unwrap();
    assert_eq!(editor.finish(), "[a]\nx=1\n; c\n[b]\ny=2\n");
    assert_eq!(body_line_count(&mut editor, "a"), 2);
    assert_eq!(body_line_count(&mut editor, "b"), 1);
}

#[test]
fn lines_above_the_first_header_are_the_preamble() {
    let mut editor = open("[s]\na=1\n");
    in_preamble(&mut editor, |preamble| {
        preamble.insert_body_lines(0, &["; top"])
    })
    .unwrap();
    assert_eq!(editor.finish(), "; top\n[s]\na=1\n");
}

#[test]
fn preamble_entries_are_editable() {
    let mut editor = open("root = true\n[*]\nx=1\n");
    in_preamble(&mut editor, |preamble| preamble.set("root", "false")).unwrap();
    assert_eq!(editor.finish(), "root = false\n[*]\nx=1\n");
}

#[test]
fn an_empty_value_moves_the_inline_comment_above_the_entry() {
    let mut editor = open_with(
        "[s]\nk = v ; note\n",
        &options(false, true),
        &spacing(SeparatorSpacing::Preserve),
    );
    in_section(&mut editor, "s", |section| section.set("k", "")).unwrap();
    assert_eq!(editor.finish(), "[s]\n ; note\nk = \n");
}

#[test]
fn a_value_keeps_the_inline_comment_after_it() {
    let mut editor = open_with(
        "[s]\nk = v ; note\n",
        &options(false, true),
        &spacing(SeparatorSpacing::Preserve),
    );
    in_section(&mut editor, "s", |section| section.set("k", "w")).unwrap();
    assert_eq!(editor.finish(), "[s]\nk = w ; note\n");
}

#[test]
fn a_bare_key_gains_a_separator_before_its_comment() {
    let mut editor = open_with(
        "[s]\nflag ; note\n",
        &options(true, true),
        &spacing(SeparatorSpacing::Preserve),
    );
    in_section(&mut editor, "s", |section| section.set("flag", "on")).unwrap();
    assert_eq!(editor.finish(), "[s]\nflag = on ; note\n");
}

#[test]
fn junk_after_a_key_is_replaced() {
    let mut editor = open_with(
        "[s]\nkey junk\n",
        &options(true, true),
        &spacing(SeparatorSpacing::Preserve),
    );
    in_section(&mut editor, "s", |section| section.set("key", "v")).unwrap();
    assert_eq!(editor.finish(), "[s]\nkey = v\n");
}

#[test]
fn preserve_moves_old_trailing_spacing_into_the_gap() {
    let mut editor = open("[s]\nk = old  \n");
    in_section(&mut editor, "s", |section| section.set("k", "")).unwrap();
    assert_eq!(editor.finish(), "[s]\nk =   \n");
}

#[test]
fn compact_drops_old_trailing_spacing_of_an_empty_value() {
    let mut editor = open_with(
        "[s]\nk = old  \n",
        &options(false, false),
        &spacing(SeparatorSpacing::Compact),
    );
    in_section(&mut editor, "s", |section| section.set("k", "")).unwrap();
    assert_eq!(editor.finish(), "[s]\nk=\n");
}

#[test]
fn nothing_is_added_after_a_continuing_last_line() {
    let source = "[s]\npath=C:\\dir\\";
    let mut editor = open(source);
    let result = in_section(&mut editor, "s", |section| section.append_entry("n", "1"));
    assert_eq!(result, Err(EditError::LastLineContinues));
    let result = atomic(&mut editor, |editor| {
        editor.section_or_insert("t").map(drop)
    });
    assert_eq!(result, Err(EditError::LastLineContinues));
    assert_eq!(editor.finish(), source);
}

#[test]
fn a_value_ending_in_a_backslash_is_refused() {
    let mut editor = open("[s]\npath=old");
    let result = in_section(&mut editor, "s", |section| section.set("path", "C:\\dir\\"));
    assert_eq!(
        result,
        Err(EditError::InvalidValue {
            value: "C:\\dir\\".into()
        })
    );
}

#[test]
fn a_cr_continuation_at_the_end_is_terminated_with_crlf() {
    let mut editor = open("[s]\nk=one \\\r");
    in_section(&mut editor, "s", |section| section.append_entry("n", "1")).unwrap();
    assert_eq!(editor.finish(), "[s]\nk=one \\\r\r\nn = 1\n");
}

#[test]
fn a_blank_line_after_a_cr_gets_crlf() {
    let mut editor = open("[s]\nk=v\r");
    in_section(&mut editor, "s", |section| section.append_body_lines(&[""])).unwrap();
    assert_eq!(editor.finish(), "[s]\nk=v\r\r\n");
    assert_eq!(body_line_count(&mut editor, "s"), 2);
}

#[test]
fn a_line_before_an_lf_blank_line_gets_crlf() {
    let mut editor = open("[s]\ra=1\r\n\nb=2\r");
    in_section(&mut editor, "s", |section| {
        section.insert_body_lines(1, &["x"])
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\ra=1\r\nx\r\n\nb=2\r");
}

#[test]
fn a_removal_between_cr_and_an_lf_blank_line_merges_them() {
    let mut editor = open("[s]\ra=1\rb=2\n\nc=3\n");
    assert_eq!(
        in_section(&mut editor, "s", |section| section.remove_entry("b")),
        Ok(true)
    );
    assert_eq!(editor.finish(), "[s]\ra=1\r\nc=3\n");
    assert_eq!(body_line_count(&mut editor, "s"), 2);
}

#[test]
fn a_removal_merges_into_the_header() {
    let mut editor = open("[s]\rx=1\n\nk=2\n");
    in_section(&mut editor, "s", |section| section.remove_body_lines(0..1)).unwrap();
    assert_eq!(editor.finish(), "[s]\r\nk=2\n");
    assert_eq!(body_line_count(&mut editor, "s"), 1);
}

#[test]
fn a_removal_may_not_expose_a_byte_order_mark() {
    let mut editor = open("a=1\n\u{FEFF}b=2\n");
    let result = in_preamble(&mut editor, |preamble| preamble.remove_entry("a"));
    assert_eq!(result, Err(EditError::ByteOrderMarkAtStart));
}

#[test]
fn a_key_may_start_with_u_feff_only_after_a_byte_order_mark() {
    let mut editor = open("");
    let result = in_preamble(&mut editor, |preamble| {
        preamble.append_entry("\u{FEFF}k", "1")
    });
    assert_eq!(result, Err(EditError::ByteOrderMarkAtStart));
    assert_eq!(editor.finish(), "");

    let mut editor = open_with_len_max("\u{FEFF}", 100);
    in_preamble(&mut editor, |preamble| {
        preamble.append_entry("\u{FEFF}k", "1")
    })
    .unwrap();
    assert_eq!(editor.finish(), "\u{FEFF}\u{FEFF}k = 1\n");
}

#[test]
fn set_key_may_not_start_the_document_with_u_feff() {
    let mut editor = open("k=1\n");
    let result = in_preamble(&mut editor, |preamble| {
        preamble.entry_at(0).unwrap().set_key("\u{FEFF}k")
    });
    assert_eq!(result, Err(EditError::ByteOrderMarkAtStart));

    let mut indented = super::support::open(" k=1\n");
    in_preamble(&mut indented, |preamble| {
        preamble.entry_at(0).unwrap().set_key("\u{FEFF}k")
    })
    .unwrap();
    assert_eq!(indented.finish(), " \u{FEFF}k=1\n");
}

#[test]
fn a_terminated_blank_line_separates_a_new_section() {
    let mut editor = open("[s]\nk=1\n \t");
    atomic(&mut editor, |editor| {
        editor.section_or_insert("t").map(drop)
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\nk=1\n \t\n[t]\n");
}

#[test]
fn a_new_section_after_a_cr_gets_a_crlf_blank_line() {
    let mut editor = open("[s]\nk=v\r");
    atomic(&mut editor, |editor| {
        editor.section_or_insert("next").map(drop)
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\nk=v\r\r\n[next]\n");
}

#[test]
fn inserting_after_an_unterminated_header_terminates_it() {
    let mut editor = open("[s]");
    in_section(&mut editor, "s", |section| {
        section.insert_entry(0, "k", "v")
    })
    .unwrap();
    assert_eq!(editor.finish(), "[s]\nk = v\n");
}

#[test]
fn the_size_limit_is_exact() {
    let mut editor = open_with_len_max("[s]\n", 10);
    in_section(&mut editor, "s", |section| section.append_entry("k", "1")).unwrap();
    assert_eq!(editor.finish(), "[s]\nk = 1\n");
    assert_eq!(editor.finish().len(), 10);
    let result = in_section(&mut editor, "s", |section| section.append_entry("k", ""));
    assert_eq!(
        result,
        Err(EditError::DocumentTooLarge { len: 15, max: 10 })
    );
}

#[test]
fn names_use_the_first_match_and_positions_reach_the_rest() {
    let mut editor = open("[a]\nx=1\n[a]\ny=2\n");
    in_section(&mut editor, "a", |section| section.set("y", "3")).unwrap();
    atomic(&mut editor, |editor| {
        editor.section_at(1).unwrap().set("y", "4")
    })
    .unwrap();
    assert_eq!(editor.finish(), "[a]\nx=1\ny = 3\n[a]\ny=4\n");
}

#[test]
fn remove_entry_removes_the_first_duplicate() {
    let mut editor = open("[s]\nk=1\nk=2\n");
    assert_eq!(
        in_section(&mut editor, "s", |section| section.remove_entry("k")),
        Ok(true)
    );
    assert_eq!(editor.finish(), "[s]\nk=2\n");
}
