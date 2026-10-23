//! Arbitrary editor operations, with arbitrary strings, positions and ranges, on arbitrary text.
//!
//! Every operation goes through [`step`], which checks the editor's guarantees after it: the tree
//! equals a parse of the text (C1); a refused edit changes nothing (C2); an insertion or a
//! removal changes one contiguous run of bytes (C3); and an editor reopened from the text before
//! the step returns the same result, text and tree, so saving never changes the next edit's
//! meaning. Before any operation runs, valid input also checks one semantic edit against a small
//! independent model.
//!
//! Every byte sequence is a valid input: a missing byte reads as zero, and text stops at the end
//! of the input.
//!
//! ```text
//! input     = parse-flags spacing source operation*   (at most 128 operations)
//! spacing   = style [before after]                    (both only for `Exact`)
//! source    = length:u16-le byte*
//! operation = kind target field*                      (fields in `Operation` order)
//! target    = selector name                           (preamble, by name, or by index)
//! text      = length:u8 byte*                         (names, keys, values, lines)
//! lines     = count:u8 text*                          (count modulo 4)
//! ```
//!
//! Text is decoded as lossy UTF-8. Positions are single bytes, 255 standing for `usize::MAX`.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditError, EditOptions, Editor, SectionEditor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse_with};

/// Number of distinct operations; a kind byte selects one modulo this.
pub const OPERATION_KINDS: u8 = 17;
/// Later operations are ignored, which bounds the work per input.
const OPERATIONS_MAX: usize = 128;

/// The container an operation addresses.
#[derive(Debug, Clone)]
pub enum Target {
    /// [`Editor::preamble`].
    Preamble,
    /// [`Editor::section`]: arbitrary names rarely match.
    Name(String),
    /// [`Editor::section_at`].
    Index(usize),
}

/// One public call, or a group of reads.
#[derive(Debug, Clone)]
pub enum Operation {
    Set {
        key: String,
        value: String,
    },
    AppendEntry {
        key: String,
        value: String,
    },
    InsertEntry {
        at: usize,
        key: String,
        value: String,
    },
    RemoveEntry {
        key: String,
    },
    RenameKey {
        old: String,
        new: String,
    },
    AppendBodyLines {
        lines: Vec<String>,
    },
    InsertBodyLines {
        at: usize,
        lines: Vec<String>,
    },
    RemoveBodyLines {
        start: usize,
        end: usize,
    },
    RemoveContainer,
    SetKeyAt {
        index: usize,
        key: String,
    },
    SetValueAt {
        index: usize,
        value: String,
    },
    RemoveEntryAt {
        index: usize,
    },
    /// [`Editor::section_or_insert`] with the target's name.
    SectionOrInsert,
    /// Every read of the target and of its entry `index`, checked against the tree.
    Read {
        index: usize,
    },
    /// The empty insertions and removals, which change nothing, and their past-the-end errors.
    EmptyEdits,
    /// [`SectionEditor::insert_body_lines`] at `body_line_count()`: after the last line, or
    /// above the next header.
    InsertBodyLinesAtEnd {
        lines: Vec<String>,
    },
    /// [`SectionEditor::remove_body_lines`] of every body line.
    ClearBody,
}

impl Operation {
    /// Whether a successful call inserts one run of bytes.
    fn inserts(&self) -> bool {
        matches!(
            self,
            Operation::AppendEntry { .. }
                | Operation::InsertEntry { .. }
                | Operation::AppendBodyLines { .. }
                | Operation::InsertBodyLines { .. }
                | Operation::InsertBodyLinesAtEnd { .. }
                | Operation::SectionOrInsert
        )
    }

    /// Whether a successful call removes one run of bytes.
    fn removes(&self) -> bool {
        matches!(
            self,
            Operation::RemoveEntry { .. }
                | Operation::RemoveBodyLines { .. }
                | Operation::RemoveContainer
                | Operation::RemoveEntryAt { .. }
                | Operation::ClearBody
        )
    }
}

/// What a call returned, without the handles it borrowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// The target container does not exist.
    NoTarget,
    /// The entry at the requested index does not exist.
    NoEntry,
    Done,
    Found(bool),
    /// The reads of [`Operation::Read`].
    Reads(Vec<String>),
}

/// The result of one step.
pub type Outcome = Result<Value, EditError>;

/// The container `target` addresses, if it exists.
fn container<'e>(editor: &'e mut Editor, target: &Target) -> Option<SectionEditor<'e>> {
    match target {
        Target::Preamble => Some(editor.preamble()),
        Target::Name(name) => editor.section(name),
        Target::Index(index) => editor.section_at(*index),
    }
}

/// Run `operation` on `target` of `editor`.
pub fn apply(editor: &mut Editor, target: &Target, operation: &Operation) -> Outcome {
    if let Operation::SectionOrInsert = operation {
        let Target::Name(name) = target else {
            return Ok(Value::NoTarget);
        };
        return editor.section_or_insert(name).map(|_| Value::Done);
    }
    let file = editor.file();
    let Some(mut container) = container(editor, target) else {
        return Ok(Value::NoTarget);
    };
    let done = |result: Result<(), EditError>| result.map(|()| Value::Done);
    match operation {
        Operation::Set { key, value } => done(container.set(key, value)),
        Operation::AppendEntry { key, value } => done(container.append_entry(key, value)),
        Operation::InsertEntry { at, key, value } => done(container.insert_entry(*at, key, value)),
        Operation::RemoveEntry { key } => container.remove_entry(key).map(Value::Found),
        Operation::RenameKey { old, new } => container.rename_key(old, new).map(Value::Found),
        Operation::AppendBodyLines { lines } => {
            done(container.append_body_lines(&line_slices(lines)))
        }
        Operation::InsertBodyLines { at, lines } => {
            done(container.insert_body_lines(*at, &line_slices(lines)))
        }
        Operation::RemoveBodyLines { start, end } => {
            done(container.remove_body_lines(*start..*end))
        }
        Operation::RemoveContainer => {
            container.remove();
            Ok(Value::Done)
        }
        Operation::SetKeyAt { index, key } => match container.entry_at(*index) {
            Some(mut entry) => done(entry.set_key(key)),
            None => Ok(Value::NoEntry),
        },
        Operation::SetValueAt { index, value } => match container.entry_at(*index) {
            Some(mut entry) => done(entry.set_value(value)),
            None => Ok(Value::NoEntry),
        },
        Operation::RemoveEntryAt { index } => match container.entry_at(*index) {
            Some(entry) => done(entry.remove()),
            None => Ok(Value::NoEntry),
        },
        Operation::SectionOrInsert => unreachable!("handled above"),
        Operation::Read { index } => Ok(Value::Reads(reads(&mut container, &file, target, *index))),
        Operation::EmptyEdits => empty_edits(&mut container),
        Operation::InsertBodyLinesAtEnd { lines } => {
            let end = container.body_line_count();
            done(container.insert_body_lines(end, &line_slices(lines)))
        }
        Operation::ClearBody => {
            let end = container.body_line_count();
            done(container.remove_body_lines(0..end))
        }
    }
}

/// The element slices of `lines`.
fn line_slices(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}

/// Every read of `container` and of its entry `index`, checked against `file`, a snapshot of
/// the same document.
fn reads(
    container: &mut SectionEditor<'_>,
    file: &File,
    target: &Target,
    index: usize,
) -> Vec<String> {
    let lines: Vec<ini_edit::SyntaxNode> = match target {
        Target::Preamble => file.preamble_lines().collect(),
        Target::Name(name) => file
            .sections()
            .find(|section| section.name().as_deref() == Some(name.as_str()))
            .expect("the handle's section is in the file")
            .body_lines()
            .collect(),
        Target::Index(index) => file
            .sections()
            .nth(*index)
            .expect("the handle's section is in the file")
            .body_lines()
            .collect(),
    };
    assert_eq!(container.body_line_count(), lines.len());
    assert_eq!(container.is_preamble(), matches!(target, Target::Preamble));
    let section_count = container.editor().section_count();
    assert_eq!(section_count, file.sections().count());
    let entry_count = container.entry_count();
    let entries: Vec<ini_edit::ast::Entry> = lines
        .iter()
        .filter_map(|line| ini_edit::ast::Entry::cast(line.clone()))
        .collect();
    assert_eq!(entry_count, entries.len());
    let mut values = vec![format!(
        "{:?} {} {entry_count} {section_count}",
        container.name(),
        lines.len()
    )];
    let Some(entry) = container.entry_at(index) else {
        return values;
    };
    assert!(index < entry_count);
    assert_eq!(entry.index(), index);
    // The handle reads the same document.
    assert_eq!(entry.editor().finish(), file.syntax().text().to_string());
    // The entry is the file's line at its body-line position.
    let line = ini_edit::ast::Entry::cast(lines[entry.body_line()].clone())
        .expect("an entry's body line is an entry");
    let key = entry.key();
    assert_eq!(line.key().as_deref(), Some(key.as_str()));
    assert_eq!(line.value(), entry.value());
    assert_eq!(line.inline_comment(), entry.inline_comment());
    values.push(format!("{key} {:?} {}", entry.value(), entry.body_line()));
    // A lookup by key finds the first entry with that key, at or before this one.
    let first = entries
        .iter()
        .position(|entry| entry.key().as_deref() == Some(key.as_str()));
    let found = container.entry(&key).expect("the entry's own key is found");
    assert_eq!(Some(found.index()), first);
    assert_eq!(found.key(), key);
    values
}

/// The identity of empty edits, and the errors one position past the end.
fn empty_edits(container: &mut SectionEditor<'_>) -> Outcome {
    let before = container.editor().finish();
    let end = container.body_line_count();
    container.append_body_lines(&[])?;
    container.insert_body_lines(end, &[])?;
    container.remove_body_lines(end..end)?;
    assert_eq!(
        container.editor().finish(),
        before,
        "an empty edit changed the document"
    );
    let past = end + 1;
    let position = container.insert_body_lines(past, &[]);
    assert!(matches!(
        position,
        Err(EditError::PositionOutOfRange { .. })
    ));
    let range = container.remove_body_lines(past..past);
    assert!(matches!(range, Err(EditError::RangeOutOfRange { .. })));
    assert_eq!(container.editor().finish(), before);
    Ok(Value::Done)
}

/// Whether two snapshots hold the same tree.
fn same_tree(left: &File, right: &File) -> bool {
    left.syntax().green() == right.syntax().green()
}

/// C1: the tree equals a parse of the text with the editor's options.
pub fn assert_agrees_with_reparse(editor: &Editor, context: &dyn std::fmt::Debug) {
    let text = editor.finish();
    let reparse = parse_with(&text, editor.parse_options());
    let tree = editor.file().syntax().green().into_owned();
    assert_eq!(tree, *reparse.green(), "C1: {context:?} {text:?}");
}

/// Apply one operation to `live` and to an editor reopened from `live`'s text, and check the
/// guarantees after it. Returns the outcome.
pub fn step(live: &mut Editor, target: &Target, operation: &Operation) -> Outcome {
    let before = live.finish();
    let tree = live.file();
    let mut reopened = Editor::new(&before, live.parse_options(), live.edit_options())
        .expect("a document the editor holds reopens");
    let outcome = apply(live, target, operation);
    let replay = apply(&mut reopened, target, operation);
    let context = (&before, target, operation, &outcome);
    assert_eq!(outcome, replay, "reload: {context:?}");
    assert_eq!(live.finish(), reopened.finish(), "reload text: {context:?}");
    let same = same_tree(&live.file(), &reopened.file());
    assert!(same, "reload tree: {context:?}");
    assert_agrees_with_reparse(live, &context);
    if outcome.is_err() {
        assert_eq!(live.finish(), before, "C2 text: {context:?}");
        assert!(same_tree(&live.file(), &tree), "C2 tree: {context:?}");
    }
    if matches!(outcome, Ok(Value::Done | Value::Found(true))) {
        assert_contiguous(&before, &live.finish(), operation);
    }
    outcome
}

/// C3 at the byte level: an insertion adds one contiguous run of bytes and a removal deletes one,
/// rule C's terminator and rule D's merge included; no other byte changes.
fn assert_contiguous(before: &str, after: &str, operation: &Operation) {
    let prefix = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = before.as_bytes()[prefix..]
        .iter()
        .rev()
        .zip(after.as_bytes()[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    if operation.inserts() {
        assert_eq!(
            prefix + suffix,
            before.len(),
            "C3 insertion: {before:?} -> {after:?}"
        );
    }
    if operation.removes() {
        assert_eq!(
            prefix + suffix,
            after.len(),
            "C3 removal: {before:?} -> {after:?}"
        );
    }
}

/// The spacing alphabet: mostly spaces and tabs, so that `Exact` spacing is often valid.
const SPACING_PIECES: [&str; 8] = [" ", "\t", " ", "\t", "x", ";", "\n", "\u{a0}"];

/// The harness's own statement of valid spacing: a run of spaces and tabs.
fn is_spacing(text: &str) -> bool {
    text.chars()
        .all(|character| character == ' ' || character == '\t')
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

    /// A position, or one bound of a range.
    fn position(&mut self) -> usize {
        match self.byte() {
            u8::MAX => usize::MAX,
            byte => usize::from(byte),
        }
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

    fn lines(&mut self) -> Vec<String> {
        let count = self.byte() % 4;
        (0..count).map(|_| self.text()).collect()
    }

    /// Short spacing over [`SPACING_PIECES`].
    fn spacing_text(&mut self) -> String {
        let count = self.byte() % 4;
        (0..count)
            .map(|_| SPACING_PIECES[usize::from(self.byte()) % SPACING_PIECES.len()])
            .collect()
    }

    fn spacing(&mut self) -> SeparatorSpacing {
        match self.byte() % 3 {
            0 => SeparatorSpacing::Preserve,
            1 => SeparatorSpacing::Compact,
            _ => {
                let (before, after) = (self.spacing_text(), self.spacing_text());
                let spacing = SeparatorSpacing::exact(&before, &after);
                // Refused exactly when the harness says one side is not spacing.
                assert_eq!(spacing.is_ok(), is_spacing(&before) && is_spacing(&after));
                spacing.unwrap_or(SeparatorSpacing::Preserve)
            }
        }
    }

    fn target(&mut self) -> Target {
        let selector = self.byte() % 3;
        let name = self.text();
        match selector {
            0 => Target::Preamble,
            1 => Target::Name(name),
            _ => Target::Index(name.len() % 4),
        }
    }

    fn operation(&mut self) -> (Target, Operation) {
        let kind = self.byte() % OPERATION_KINDS;
        let target = self.target();
        // Struct expressions evaluate their fields in the order written.
        let operation = match kind {
            0 => Operation::Set {
                key: self.text(),
                value: self.text(),
            },
            1 => Operation::AppendEntry {
                key: self.text(),
                value: self.text(),
            },
            2 => Operation::InsertEntry {
                at: self.position(),
                key: self.text(),
                value: self.text(),
            },
            3 => Operation::RemoveEntry { key: self.text() },
            4 => Operation::RenameKey {
                old: self.text(),
                new: self.text(),
            },
            5 => Operation::AppendBodyLines {
                lines: self.lines(),
            },
            6 => Operation::InsertBodyLines {
                at: self.position(),
                lines: self.lines(),
            },
            7 => Operation::RemoveBodyLines {
                start: self.position(),
                end: self.position(),
            },
            8 => Operation::RemoveContainer,
            9 => Operation::SetKeyAt {
                index: self.position(),
                key: self.text(),
            },
            10 => Operation::SetValueAt {
                index: self.position(),
                value: self.text(),
            },
            11 => Operation::RemoveEntryAt {
                index: self.position(),
            },
            12 => Operation::SectionOrInsert,
            13 => Operation::Read {
                index: self.position(),
            },
            14 => Operation::EmptyEdits,
            15 => Operation::InsertBodyLinesAtEnd {
                lines: self.lines(),
            },
            16 => Operation::ClearBody,
            _ => unreachable!("kind is reduced modulo OPERATION_KINDS"),
        };
        (target, operation)
    }
}

#[derive(Debug)]
struct Input {
    parse_options: ParseOptions,
    edit_options: EditOptions,
    source: String,
    operations: Vec<(Target, Operation)>,
}

fn decode(data: &[u8]) -> Input {
    let mut bytes = Bytes { rest: data };
    let flags = bytes.byte();
    let parse_options = ParseOptions {
        allow_no_value: flags & 1 != 0,
        inline_comments: flags & 2 != 0,
    };
    let edit_options = EditOptions {
        separator_spacing: bytes.spacing(),
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

/// On valid input, check one semantic edit against a small independent model: clear the first
/// value of the first section and preserve every other entry.
fn check_clearing(input: &Input, editor: &mut Editor) {
    let parsed = parse_with(&input.source, &input.parse_options);
    if !parsed.errors().is_empty() {
        return;
    }
    let mut expected = contents(&editor.file());
    let Some(mut section) = editor.section_at(0) else {
        return;
    };
    let Some(mut entry) = section.entry_at(0) else {
        return;
    };
    expected[1].1[0].1 = Some(String::new());
    entry.set_value("").expect("an empty value is always valid");
    assert_eq!(contents(&editor.file()), expected);
    let output = editor.finish();
    let saved = parse_with(&output, &input.parse_options);
    assert!(saved.errors().is_empty(), "{output:?}");
    assert_eq!(contents(&File::cast(saved.syntax()).unwrap()), expected);
}

/// Decode `data` as described in the module documentation and check it.
pub fn check(data: &[u8]) {
    let input = decode(data);
    let open = || Editor::new(&input.source, &input.parse_options, &input.edit_options);
    let mut editor = open().expect("a decoded source is small");
    check_clearing(&input, &mut editor);
    let mut editor = open().expect("a decoded source is small");
    assert_agrees_with_reparse(&editor, &input.source);
    for (target, operation) in &input.operations {
        // The step checks every guarantee itself; its outcome only reports what happened.
        let _ = step(&mut editor, target, operation);
    }
}

/// Each kind byte below [`OPERATION_KINDS`] must select a different operation.
#[test]
fn every_operation_kind_decodes_to_its_own_operation() {
    let operations: std::collections::HashSet<_> = (0..OPERATION_KINDS)
        .map(|kind| std::mem::discriminant(&Bytes { rest: &[kind] }.operation().1))
        .collect();
    assert_eq!(operations.len(), usize::from(OPERATION_KINDS));
}
