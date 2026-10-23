//! Error precedence: when two checks fail, the first in the documented order wins. Results are
//! deterministic, so a live and a reopened editor return the same error.

use super::support::{atomic, in_section, open, open_with_len_max};
use crate::editor::{EditError, Editor};

/// The variant of `result`, without its fields.
fn variant<T: std::fmt::Debug>(result: &Result<T, EditError>) -> String {
    let text = format!("{result:?}");
    let name = text.trim_start_matches("Err(");
    name.split([' ', '(', ')', '{']).next().unwrap().to_owned()
}

#[test]
fn positions_come_before_everything_else() {
    let mut editor = open("[s]\nk=C:\\d\\");
    let result = in_section(&mut editor, "s", |section| {
        section.insert_entry(9, "a=b", "x\ny")
    });
    let expected = EditError::PositionOutOfRange {
        at: 9,
        body_line_count: 1,
    };
    assert_eq!(result, Err(expected.clone()));
    // An empty slice at a bad position is a bad position, not a no-op.
    let result = in_section(&mut editor, "s", |section| {
        section.insert_body_lines(9, &[])
    });
    assert_eq!(result, Err(expected));
}

#[test]
fn sizes_come_before_text() {
    let mut editor = open_with_len_max("[s]\nk=v\n", 12);
    let result = in_section(&mut editor, "s", |section| {
        section.append_entry("a=b", "x\ny")
    });
    assert_eq!(variant(&result), "DocumentTooLarge");
    // An oversized value ending in a backslash is too large first.
    let result = in_section(&mut editor, "s", |section| section.set("k", "C:\\long\\"));
    assert_eq!(variant(&result), "DocumentTooLarge");
    let result = atomic(&mut editor, |editor| {
        editor.section_or_insert("a]b").map(drop)
    });
    assert_eq!(variant(&result), "DocumentTooLarge");
    let result = in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["[x]", "y"])
    });
    assert_eq!(variant(&result), "DocumentTooLarge");
    // An oversized key is too large before it is checked.
    let result = in_section(&mut editor, "s", |section| {
        section.entry_at(0).unwrap().set_key("k k k k")
    });
    assert_eq!(variant(&result), "DocumentTooLarge");
    let result = in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["[]"])
    });
    assert_eq!(variant(&result), "InvalidLine");
}

#[test]
fn keys_come_before_values_and_text_before_context() {
    let mut editor = open("[s]\nk=C:\\d\\");
    let result = in_section(&mut editor, "s", |section| {
        section.append_entry("a=b", "x\ny")
    });
    assert_eq!(result, Err(EditError::InvalidKey { key: "a=b".into() }));
    let result = in_section(&mut editor, "s", |section| {
        section.append_entry("a", "x\ny")
    });
    assert_eq!(
        result,
        Err(EditError::InvalidValue {
            value: "x\ny".into()
        })
    );
    let result = in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["[x]"])
    });
    let line = "[x]".to_owned();
    assert_eq!(result, Err(EditError::InvalidLine { index: 0, line }));
    let result = atomic(&mut editor, |editor| {
        editor.section_or_insert("a]").map(drop)
    });
    assert_eq!(variant(&result), "InvalidSectionName");
    // Only then the continuing last line.
    let result = in_section(&mut editor, "s", |section| section.append_entry("a", "x"));
    assert_eq!(result, Err(EditError::LastLineContinues));
    let result = in_section(&mut editor, "s", |section| {
        section.append_body_lines(&["; ok"])
    });
    assert_eq!(result, Err(EditError::LastLineContinues));
}

#[test]
fn renames_check_absence_then_duplicates_then_size_then_the_key() {
    let source = "[s]\na=1\nbb=2\n";
    let mut editor = open_with_len_max(source, source.len());
    // An absent old key returns before the new key is checked.
    let result = in_section(&mut editor, "s", |section| {
        section.rename_key("absent", "x=y")
    });
    assert_eq!(result, Ok(false));
    // A duplicate is reported before the size, which a longer key would exceed.
    let result = in_section(&mut editor, "s", |section| section.rename_key("a", "bb"));
    assert_eq!(result, Err(EditError::DuplicateKey { key: "bb".into() }));
    let result = in_section(&mut editor, "s", |section| section.rename_key("a", "cc"));
    assert_eq!(variant(&result), "DocumentTooLarge");
    let result = in_section(&mut editor, "s", |section| section.rename_key("a", "="));
    assert_eq!(result, Err(EditError::InvalidKey { key: "=".into() }));
}

#[test]
fn ranges_check_reversal_before_the_end() {
    let mut editor = open("[s]\na=1\n");
    let reversed = std::ops::Range { start: 9, end: 3 };
    let result = in_section(&mut editor, "s", |section| {
        section.remove_body_lines(reversed)
    });
    let expected = EditError::RangeOutOfRange {
        start: 9,
        end: 3,
        body_line_count: 1,
    };
    assert_eq!(result, Err(expected));
}

#[test]
fn live_and_reopened_editors_return_the_same_errors() {
    type Operation = fn(&mut Editor) -> Result<(), EditError>;
    let operations: [Operation; 4] = [
        |editor| editor.section("s").unwrap().append_entry("a=b", "x\ny"),
        |editor| {
            editor
                .section("s")
                .unwrap()
                .append_body_lines(&["ok", "[x]"])
        },
        |editor| editor.section_or_insert("").map(drop),
        |editor| editor.section("s").unwrap().remove_body_lines(0..9),
    ];
    for operation in operations {
        let mut live = open("[s]\nk=v\n");
        in_section(&mut live, "s", |section| {
            section.append_body_lines(&["x=1"])
        })
        .unwrap();
        let mut reopened = open(&live.finish());
        assert_eq!(
            atomic(&mut live, operation),
            atomic(&mut reopened, operation)
        );
    }
}
