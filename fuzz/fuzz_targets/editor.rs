#![no_main]

use arbitrary::Arbitrary;
use ini_edit::ParseOptions;
use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};
use libfuzzer_sys::fuzz_target;

type Entries = Vec<(Option<String>, Option<String>)>;

fn contents(file: &File) -> Vec<(Option<String>, Entries)> {
    let entries = |items: Vec<ini_edit::ast::Entry>| {
        items
            .into_iter()
            .map(|entry| (entry.key(), entry.value()))
            .collect()
    };
    let mut result = vec![(None, entries(file.preamble_entries().collect()))];
    result.extend(
        file.sections()
            .map(|section| (section.name(), entries(section.entries().collect()))),
    );
    result
}

// Raw operations can intentionally create invalid syntax. Before those
// operations, valid input lets us check an actual semantic edit against a
// small independent model: clear one value and preserve every other entry.
fn check_clearing(source: &str, parse_options: &ParseOptions, edit_options: &EditOptions) {
    let parsed = ini_edit::parse_with(source, parse_options);
    if !parsed.errors().is_empty() {
        return;
    }
    let editor = Editor::with_options(source, parse_options, edit_options);
    let Some(name) = editor
        .file()
        .sections()
        .next()
        .and_then(|section| section.name())
    else {
        return;
    };
    let entries = editor.section(&name).entries_mut();
    let Some(entry) = entries.first() else {
        return;
    };
    let mut expected = contents(&editor.file());
    expected[1].1[0].1 = Some(String::new());
    entry.set_value("");
    assert_eq!(contents(&editor.file()), expected);
    let output = editor.finish();
    let saved = ini_edit::parse_with(&output, parse_options);
    assert!(saved.errors().is_empty(), "{output:?}");
    assert_eq!(contents(&File::cast(saved.syntax()).unwrap()), expected);
}

#[derive(Debug, Arbitrary)]
enum Op<'a> {
    Set {
        section: &'a str,
        key: &'a str,
        value: &'a str,
    },
    Append {
        section: &'a str,
        key: &'a str,
        value: &'a str,
    },
    Remove {
        section: &'a str,
        key: &'a str,
    },
    Rename {
        section: &'a str,
        old: &'a str,
        new: &'a str,
    },
    RemoveSection {
        section: &'a str,
    },
    AppendRaw {
        section: &'a str,
        line: &'a str,
    },
    InsertRawAt {
        section: &'a str,
        index: u8,
        line: &'a str,
    },
    InsertEntry {
        section: &'a str,
        index: u8,
        key: &'a str,
        value: &'a str,
    },
    RemoveLines {
        section: &'a str,
        start: u8,
        end: u8,
    },
    ClearAndInsert {
        section: &'a str,
        index: u8,
        key: &'a str,
        value: &'a str,
    },
    AppendNothing {
        section: &'a str,
    },
}

#[derive(Debug, Arbitrary)]
struct FuzzInput<'a> {
    parse_flags: u8,
    spacing_style: u8,
    source: &'a str,
    ops: Vec<Op<'a>>,
}

fuzz_target!(|input: FuzzInput<'_>| {
    let parse_options = ParseOptions {
        allow_no_value: input.parse_flags & 1 != 0,
        inline_comments: input.parse_flags & 2 != 0,
    };
    let edit_options = EditOptions {
        separator_spacing: match input.spacing_style % 3 {
            0 => SeparatorSpacing::Preserve,
            1 => SeparatorSpacing::Compact,
            _ => SeparatorSpacing::exact("\t", "  "),
        },
    };
    check_clearing(input.source, &parse_options, &edit_options);
    let ed = Editor::with_options(input.source, &parse_options, &edit_options);

    for op in &input.ops {
        match op {
            Op::Set {
                section,
                key,
                value,
            } => {
                ed.section(section).set(key, value);
            }
            Op::Append {
                section,
                key,
                value,
            } => {
                ed.section(section).append_entry(key, value);
            }
            Op::Remove { section, key } => {
                let _ = ed.section(section).remove_entry(key);
            }
            Op::Rename { section, old, new } => {
                let _ = ed.section(section).rename_key(old, new);
            }
            Op::RemoveSection { section } => {
                ed.section(section).remove();
            }
            Op::AppendRaw { section, line } => {
                ed.section(section).append_raw_lines(&[line]);
            }
            Op::InsertRawAt {
                section,
                index,
                line,
            } => {
                ed.section(section)
                    .insert_raw_lines_at(*index as usize, &[line]);
            }
            Op::InsertEntry {
                section,
                index,
                key,
                value,
            } => {
                ed.section(section)
                    .insert_entry_at_line(*index as usize, key, value);
            }
            Op::RemoveLines {
                section,
                start,
                end,
            } => {
                let s = *start as usize;
                let e = *end as usize;
                ed.section(section).remove_lines(s..e);
            }
            Op::ClearAndInsert {
                section,
                index,
                key,
                value,
            } => {
                // Keep the handle after removing its header. Looking the
                // section up again would create a new header and hide bugs.
                let handle = ed.section(section);
                handle.remove_lines(0..usize::MAX);
                handle.insert_entry_at_line(*index as usize, key, value);
            }
            Op::AppendNothing { section } => {
                let handle = ed.section(section);
                let before = ed.finish();
                handle.append_raw_lines(&[]);
                handle.insert_raw_lines_at(usize::MAX, &[]);
                assert_eq!(ed.finish(), before, "empty insertion changed the document");
            }
        }
    }

    // Parsing must stay lossless even when raw edits produce malformed input.
    let output = ed.finish();
    let re_parsed = ini_edit::parse_with(&output, &parse_options);
    assert_eq!(
        re_parsed.syntax().text().to_string(),
        output,
        "Editor output does not round-trip"
    );
});
