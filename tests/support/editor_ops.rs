//! Arbitrary editor operations on arbitrary text. Raw operations may make the
//! document malformed, so the final check is that parsing stays lossless.
//! Before any operation runs, valid input also checks one semantic edit
//! against a small independent model.
//!
//! Every byte sequence is a valid input: a missing byte reads as zero, and text
//! stops at the end of the input.
//!
//! ```text
//! input     = parse-flags spacing source operation*   (at most 128 operations)
//! source    = length:u16-le byte*
//! operation = kind section field*                     (fields in `Operation` order)
//! text      = length:u8 byte*                         (section, keys, values, lines)
//! ```
//!
//! Text is decoded as lossy UTF-8. Line indices are single bytes.

use ini_edit::ParseOptions;
use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};

/// Number of distinct operations; a kind byte selects one modulo this.
const OPERATION_KINDS: u8 = 11;
/// Later operations are ignored, which bounds the work per input.
const OPERATIONS_MAX: usize = 128;

type Entries = Vec<(Option<String>, Option<String>)>;

#[derive(Debug)]
enum Operation {
    Set {
        section: String,
        key: String,
        value: String,
    },
    Append {
        section: String,
        key: String,
        value: String,
    },
    Remove {
        section: String,
        key: String,
    },
    Rename {
        section: String,
        old: String,
        new: String,
    },
    RemoveSection {
        section: String,
    },
    AppendRaw {
        section: String,
        line: String,
    },
    InsertRawAt {
        section: String,
        index: usize,
        line: String,
    },
    InsertEntry {
        section: String,
        index: usize,
        key: String,
        value: String,
    },
    RemoveLines {
        section: String,
        start: usize,
        end: usize,
    },
    ClearAndInsert {
        section: String,
        index: usize,
        key: String,
        value: String,
    },
    AppendNothing {
        section: String,
    },
}

#[derive(Debug)]
struct Input {
    parse_options: ParseOptions,
    edit_options: EditOptions,
    source: String,
    operations: Vec<Operation>,
}

struct Bytes<'a> {
    rest: &'a [u8],
}

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((&first, rest)) = self.rest.split_first() else {
            return 0;
        };
        self.rest = rest;
        first
    }

    /// A line index, or one bound of a line range.
    fn index(&mut self) -> usize {
        usize::from(self.byte())
    }

    fn take(&mut self, length: usize) -> String {
        let (text, rest) = self.rest.split_at(length.min(self.rest.len()));
        self.rest = rest;
        String::from_utf8_lossy(text).into_owned()
    }

    fn text(&mut self) -> String {
        let length = usize::from(self.byte());
        self.take(length)
    }

    fn operation(&mut self) -> Operation {
        let kind = self.byte() % OPERATION_KINDS;
        let section = self.text();
        // Struct expressions evaluate their fields in the order written.
        match kind {
            0 => Operation::Set {
                section,
                key: self.text(),
                value: self.text(),
            },
            1 => Operation::Append {
                section,
                key: self.text(),
                value: self.text(),
            },
            2 => Operation::Remove {
                section,
                key: self.text(),
            },
            3 => Operation::Rename {
                section,
                old: self.text(),
                new: self.text(),
            },
            4 => Operation::RemoveSection { section },
            5 => Operation::AppendRaw {
                section,
                line: self.text(),
            },
            6 => Operation::InsertRawAt {
                section,
                index: self.index(),
                line: self.text(),
            },
            7 => Operation::InsertEntry {
                section,
                index: self.index(),
                key: self.text(),
                value: self.text(),
            },
            8 => Operation::RemoveLines {
                section,
                start: self.index(),
                end: self.index(),
            },
            9 => Operation::ClearAndInsert {
                section,
                index: self.index(),
                key: self.text(),
                value: self.text(),
            },
            10 => Operation::AppendNothing { section },
            _ => unreachable!("kind is reduced modulo OPERATION_KINDS"),
        }
    }
}

fn decode(data: &[u8]) -> Input {
    let mut bytes = Bytes { rest: data };
    let flags = bytes.byte();
    let spacing = bytes.byte();
    let parse_options = ParseOptions {
        allow_no_value: flags & 1 != 0,
        inline_comments: flags & 2 != 0,
    };
    let edit_options = EditOptions {
        separator_spacing: match spacing % 3 {
            0 => SeparatorSpacing::Preserve,
            1 => SeparatorSpacing::Compact,
            _ => SeparatorSpacing::exact("\t", "  "),
        },
    };
    let length = u16::from_le_bytes([bytes.byte(), bytes.byte()]);
    let source = bytes.take(usize::from(length));
    let mut operations = Vec::new();
    while !bytes.rest.is_empty() && operations.len() < OPERATIONS_MAX {
        operations.push(bytes.operation());
    }
    Input {
        parse_options,
        edit_options,
        source,
        operations,
    }
}

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
fn check_clearing(input: &Input) {
    let parsed = ini_edit::parse_with(&input.source, &input.parse_options);
    if !parsed.errors().is_empty() {
        return;
    }
    let editor = Editor::with_options(&input.source, &input.parse_options, &input.edit_options);
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
    let saved = ini_edit::parse_with(&output, &input.parse_options);
    assert!(saved.errors().is_empty(), "{output:?}");
    assert_eq!(contents(&File::cast(saved.syntax()).unwrap()), expected);
}

fn apply(editor: &Editor, operation: &Operation) {
    match operation {
        Operation::Set {
            section,
            key,
            value,
        } => editor.section(section).set(key, value),
        Operation::Append {
            section,
            key,
            value,
        } => editor.section(section).append_entry(key, value),
        Operation::Remove { section, key } => {
            let _ = editor.section(section).remove_entry(key);
        }
        Operation::Rename { section, old, new } => {
            let _ = editor.section(section).rename_key(old, new);
        }
        Operation::RemoveSection { section } => editor.section(section).remove(),
        Operation::AppendRaw { section, line } => {
            editor.section(section).append_raw_lines(&[line.as_str()]);
        }
        Operation::InsertRawAt {
            section,
            index,
            line,
        } => editor
            .section(section)
            .insert_raw_lines_at(*index, &[line.as_str()]),
        Operation::InsertEntry {
            section,
            index,
            key,
            value,
        } => editor
            .section(section)
            .insert_entry_at_line(*index, key, value),
        Operation::RemoveLines {
            section,
            start,
            end,
        } => editor.section(section).remove_lines(*start..*end),
        Operation::ClearAndInsert {
            section,
            index,
            key,
            value,
        } => {
            // Keep the handle after removing its header. Looking the
            // section up again would create a new header and hide bugs.
            let handle = editor.section(section);
            handle.remove_lines(0..usize::MAX);
            handle.insert_entry_at_line(*index, key, value);
        }
        Operation::AppendNothing { section } => {
            let handle = editor.section(section);
            let before = editor.finish();
            handle.append_raw_lines(&[]);
            handle.insert_raw_lines_at(usize::MAX, &[]);
            assert_eq!(
                editor.finish(),
                before,
                "empty insertion changed the document"
            );
        }
    }
}

/// Decode `data` as described in the module documentation and check it.
pub fn check(data: &[u8]) {
    let input = decode(data);
    check_clearing(&input);
    let editor = Editor::with_options(&input.source, &input.parse_options, &input.edit_options);
    for operation in &input.operations {
        apply(&editor, operation);
    }
    // Parsing must stay lossless even when raw edits produce malformed input.
    let output = editor.finish();
    let reparsed = ini_edit::parse_with(&output, &input.parse_options);
    assert_eq!(
        reparsed.syntax().text().to_string(),
        output,
        "Editor output does not round-trip: {input:?}"
    );
}

/// Each kind byte below [`OPERATION_KINDS`] must select a different operation.
#[test]
fn every_operation_kind_decodes_to_its_own_operation() {
    let operations: std::collections::HashSet<_> = (0..OPERATION_KINDS)
        .map(|kind| std::mem::discriminant(&Bytes { rest: &[kind] }.operation()))
        .collect();
    assert_eq!(operations.len(), usize::from(OPERATION_KINDS));
}
