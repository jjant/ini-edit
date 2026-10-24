//! Independent identity-based model for edits, duplicate keys, refused input and body lines.
//!
//! Handles are exclusive borrows, so every step acquires the ones it needs, by name, by section
//! index, by entry position or by key. Immutable `File` snapshots taken along the way must keep
//! showing the document as it was. Half of the keys and values are hostile: every operation that
//! would write one must fail and leave the model, and the document, unchanged.
//!
//! The model also lays out the body lines of every container itself, never by asking the editor
//! or the parser: entries, comment, blank and malformed lines, raw lines where they were inserted,
//! the comment line that `set_value` moves above its entry, and the LF blank line that a removal
//! merges into the CR line break before it (rule D). Rule D depends on line breaks, so the model
//! also predicts the line break of every new line (rule T). After every step, every container's
//! `body_line_count()`, and `index()` and `body_line()` of each of its entries, must be what the
//! model says.

use std::ops::Range;

use ini_edit::SyntaxKind;
use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditError, EditOptions, Editor, SectionEditor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse_with};

type Contents = Vec<(String, Option<String>)>;
type Document = Vec<(Option<String>, Contents)>;
/// The kinds of the body lines of each container, the preamble first.
type Layout = Vec<Vec<SyntaxKind>>;

/// Keys: four that read back, then four that a reader would not.
const KEYS: [&str; 8] = ["dup", "flag", "λ", "new", "a=b", " k", ";k", ""];
/// Keys from this index on are refused.
const HOSTILE_KEYS: usize = 4;
/// Values from this index on are refused.
const HOSTILE_VALUES: usize = 7;

/// A line break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Break {
    Lf,
    CrLf,
    Cr,
}

impl Break {
    fn text(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

/// What a body line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Content {
    /// The model's entry with this id.
    Entry(usize),
    Comment,
    /// A blank line; `bare` if it is its line break alone, without spaces or tabs.
    Blank {
        bare: bool,
    },
    /// A malformed line, which has one diagnostic.
    Error,
}

/// A body line of section `s`.
#[derive(Clone, Copy, Debug)]
struct Line {
    content: Content,
    /// Every line of `s` is terminated: `[other]` follows the first `s`, and the editor
    /// terminates every line it writes.
    end: Break,
}

impl Line {
    fn kind(self) -> SyntaxKind {
        match self.content {
            Content::Entry(_) => SyntaxKind::ENTRY,
            Content::Comment => SyntaxKind::COMMENT_LINE,
            Content::Blank { .. } => SyntaxKind::BLANK_LINE,
            Content::Error => SyntaxKind::ERROR_LINE,
        }
    }

    /// Whether the line is one LF, which reads together with a CR before it as one CRLF.
    fn is_lf(self) -> bool {
        self.content == Content::Blank { bare: true } && self.end == Break::Lf
    }
}

/// One instance of section `s`.
struct Section {
    /// The header's line break, which rule D can turn into CRLF.
    header: Break,
    lines: Vec<Line>,
}

#[derive(Clone)]
struct Entry {
    key: String,
    value: Option<String>,
    /// Whether the entry has an inline comment.
    comment: bool,
}

/// What a reader makes of a raw line, written out by hand.
#[derive(Clone)]
enum Raw {
    /// A comment, blank or malformed line.
    Line(Content),
    Entry(Entry),
    /// Not exactly one body line: the editor refuses it.
    Refused,
}

struct RawLine {
    text: String,
    raw: Raw,
}

/// Raw body lines: comment, blank and malformed lines, two entries, then four that the editor
/// refuses: a header, a malformed header, a line ending in a continuation, and two lines.
fn raw_lines(options: &ParseOptions, ending: &str) -> Vec<RawLine> {
    let entry = |key: &str, value: &str, comment| {
        let value = Some(value.to_owned());
        Raw::Entry(Entry {
            key: key.to_owned(),
            value,
            comment,
        })
    };
    let (value, comment) = if options.inline_comments {
        ("raw", true)
    } else {
        ("raw ; c", false)
    };
    let continued = format!("a \\{ending}  b");
    [
        ("; note".to_owned(), Raw::Line(Content::Comment)),
        ("  # indented".to_owned(), Raw::Line(Content::Comment)),
        (String::new(), Raw::Line(Content::Blank { bare: true })),
        (" \t".to_owned(), Raw::Line(Content::Blank { bare: false })),
        ("=bad".to_owned(), Raw::Line(Content::Error)),
        ("\t: x".to_owned(), Raw::Line(Content::Error)),
        ("dup = raw ; c".to_owned(), entry("dup", value, comment)),
        (
            format!("new = {continued}"),
            entry("new", &continued, false),
        ),
        ("[x]".to_owned(), Raw::Refused),
        ("[x".to_owned(), Raw::Refused),
        ("k = v \\".to_owned(), Raw::Refused),
        (format!("a=1{ending}b=2"), Raw::Refused),
    ]
    .into_iter()
    .map(|(text, raw)| RawLine { text, raw })
    .collect()
}

struct Model {
    entries: Vec<Entry>,
    sections: Vec<Section>,
    current: Option<usize>,
    after_other: bool,
    /// Whether `[other]` ends with the blank line that separates a recreated `[s]` from it.
    other_blank: bool,
    /// The document's line ending: the line break of its first line, which no step edits.
    ending: Break,
    raw: Vec<RawLine>,
}

impl Model {
    /// Section `s`, which a step that needs it recreates at the end of the document.
    fn current(&mut self) -> usize {
        if let Some(id) = self.current {
            return id;
        }
        let id = self.sections.len();
        // A new header ends with the document's line ending: no line follows it yet.
        self.sections.push(Section {
            header: self.ending,
            lines: Vec::new(),
        });
        self.current = Some(id);
        self.after_other = true;
        // `[other]` was the last section; a blank line now separates `[s]` from its last line.
        self.other_blank = true;
        id
    }

    fn new_entry(&mut self, entry: Entry) -> usize {
        self.entries.push(entry);
        self.entries.len() - 1
    }

    /// The ids of the entries of `section`, in order.
    fn entry_ids(&self, section: usize) -> impl Iterator<Item = usize> + '_ {
        self.sections[section]
            .lines
            .iter()
            .filter_map(|line| match line.content {
                Content::Entry(id) => Some(id),
                _ => None,
            })
    }

    fn find(&self, section: usize, key: &str) -> Option<usize> {
        self.entry_ids(section)
            .find(|&id| self.entries[id].key == key)
    }

    /// The body line of entry `id` of `section`.
    fn position(&self, section: usize, id: usize) -> usize {
        self.sections[section]
            .lines
            .iter()
            .position(|line| line.content == Content::Entry(id))
            .expect("the entry is in the section")
    }

    fn line_count(&self, section: usize) -> usize {
        self.sections[section].lines.len()
    }

    /// The position after the last body line that is not blank.
    fn content_end(&self, section: usize) -> usize {
        let lines = &self.sections[section].lines;
        lines
            .iter()
            .rposition(|line| !matches!(line.content, Content::Blank { .. }))
            .map_or(0, |last| last + 1)
    }

    /// Insert lines at body position `at` of `section`, each given by its content and the last
    /// character of its text, `None` for an empty line. Their line breaks follow rule T.
    fn insert(&mut self, section: usize, at: usize, new: &[(Content, Option<char>)]) {
        let ending = self.ending;
        let section = &mut self.sections[section];
        let mut before = match at {
            0 => section.header,
            _ => section.lines[at - 1].end,
        };
        let mut lines = Vec::with_capacity(new.len());
        for &(content, last) in new {
            // T1: an LF directly after a CR, the line's last character or, for an empty line,
            // the end of the line before it, would read together with that CR.
            let after_cr = last.map_or(before == Break::Cr, |last| last == '\r');
            let end = if after_cr && ending == Break::Lf {
                Break::CrLf
            } else {
                ending
            };
            lines.push(Line { content, end });
            before = end;
        }
        // T2: a CR directly before an LF blank line would read together with it.
        let next_is_lf = section.lines.get(at).is_some_and(|line| line.is_lf());
        let last = lines.last_mut().expect("an insertion adds a line");
        if last.end == Break::Cr && next_is_lf {
            last.end = Break::CrLf;
        }
        section.lines.splice(at..at, lines);
    }

    /// Remove body lines `range` of `section`. Rule D: a CR line break that the removal puts
    /// directly before an LF blank line reads together with it as one CRLF, so that blank line
    /// goes too.
    fn remove(&mut self, section: usize, range: Range<usize>) {
        let section = &mut self.sections[section];
        section.lines.drain(range.clone());
        // An empty range leaves two lines that were adjacent already.
        if range.is_empty() {
            return;
        }
        let next_is_lf = section
            .lines
            .get(range.start)
            .is_some_and(|line| line.is_lf());
        let before = match range.start {
            0 => &mut section.header,
            start => &mut section.lines[start - 1].end,
        };
        if *before == Break::Cr && next_is_lf {
            *before = Break::CrLf;
            section.lines.remove(range.start);
        }
    }

    fn remove_entry(&mut self, section: usize, id: usize) {
        let at = self.position(section, id);
        self.remove(section, at..at + 1);
    }

    /// Assign `value` to entry `id` of `section`. An inline comment stays only after text on the
    /// value's last line; otherwise it moves to a line of its own directly above the entry, which
    /// ends with the entry's line break.
    fn assign(&mut self, section: usize, id: usize, value: &str) {
        let at = self.position(section, id);
        let end = self.sections[section].lines[at].end;
        // The line keeps its line break: only an LF after a value ending in CR would change, and
        // only documents with CR line endings have such values, where no entry ends with LF.
        if end == Break::Lf {
            assert!(!value.ends_with('\r'));
        }
        let entry = &mut self.entries[id];
        entry.value = Some(value.to_owned());
        let last_line = value.rsplit(['\r', '\n']).next().unwrap_or_default();
        if entry.comment && last_line.is_empty() {
            entry.comment = false;
            let comment = Line {
                content: Content::Comment,
                end,
            };
            self.sections[section].lines.insert(at, comment);
        }
    }

    fn contents(&self, section: usize) -> Contents {
        self.entry_ids(section)
            .map(|id| {
                let entry = &self.entries[id];
                (entry.key.clone(), entry.value.clone())
            })
            .collect()
    }

    /// The containers in document order, from what each one is.
    fn in_order<T: Clone>(&self, preamble: T, other: T, current: impl Fn(usize) -> T) -> Vec<T> {
        let mut result = vec![preamble];
        if self.after_other {
            result.push(other.clone());
        }
        result.extend(self.current.map(current));
        if !self.after_other {
            result.push(other);
        }
        result
    }

    fn document(&self) -> Document {
        let preamble = (None, vec![("global".into(), Some("stay".into()))]);
        let other = (
            Some("other".into()),
            vec![("keep".into(), Some("value".into()))],
        );
        self.in_order(preamble, other, |id| (Some("s".into()), self.contents(id)))
    }

    fn layout(&self) -> Layout {
        let mut other = vec![SyntaxKind::ENTRY];
        if self.other_blank {
            other.push(SyntaxKind::BLANK_LINE);
        }
        let current = |id: usize| {
            self.sections[id]
                .lines
                .iter()
                .map(|line| line.kind())
                .collect()
        };
        self.in_order(vec![SyntaxKind::ENTRY], other, current)
    }

    /// The diagnostics of the document: one per malformed line, all of them in `s`.
    fn error_count(&self) -> usize {
        self.current.map_or(0, |id| {
            let lines = &self.sections[id].lines;
            lines
                .iter()
                .filter(|line| line.content == Content::Error)
                .count()
        })
    }

    /// The model's own invariant: no CR line break directly precedes an LF blank line, which a
    /// reader would read together with it.
    fn assert_line_breaks(&self) {
        let Some(id) = self.current else {
            return;
        };
        let section = &self.sections[id];
        let mut before = section.header;
        for line in &section.lines {
            if before == Break::Cr {
                assert!(!line.is_lf());
            }
            before = line.end;
        }
    }
}

fn read(file: &File) -> Document {
    let contents = |entries: Vec<ini_edit::ast::Entry>| -> Contents {
        entries
            .into_iter()
            .map(|entry| (entry.key().unwrap(), entry.value()))
            .collect()
    };
    let mut result = vec![(None, contents(file.preamble_entries().collect()))];
    result.extend(
        file.sections()
            .map(|section| (section.name(), contents(section.entries().collect()))),
    );
    result
}

/// The kinds of the body lines of every container, as the typed tree lists them.
fn read_layout(file: &File) -> Layout {
    let mut layout = vec![file.preamble_lines().map(|line| line.kind()).collect()];
    layout.extend(
        file.sections()
            .map(|section| section.body_lines().map(|line| line.kind()).collect()),
    );
    layout
}

/// One step's arguments, decoded from four bytes.
struct Action<'a> {
    operation: u8,
    selector: usize,
    key: &'a str,
    key_hostile: bool,
    value: &'a str,
    value_hostile: bool,
    /// Indices into the raw lines: the fourth byte's, and the third byte's if its bit 3 is set.
    lines: Vec<usize>,
    /// The fourth byte, which the removal of a range reads as its length.
    span: u8,
}

impl Action<'_> {
    /// The error of writing a new entry with this key and value, if any.
    fn new_entry_error(&self) -> Option<EditError> {
        if self.key_hostile {
            Some(EditError::InvalidKey {
                key: self.key.into(),
            })
        } else if self.value_hostile {
            Some(EditError::InvalidValue {
                value: self.value.into(),
            })
        } else {
            None
        }
    }

    /// The new entry that this action writes, and the last character of its line's text: the
    /// value's, or the separator's for an empty value. Only a CR matters (rule T1).
    fn new_entry(&self) -> (Entry, Option<char>) {
        let entry = Entry {
            key: self.key.into(),
            value: Some(self.value.into()),
            comment: false,
        };
        (entry, Some(self.value.chars().next_back().unwrap_or('=')))
    }
}

struct Harness {
    editor: Editor,
    model: Model,
    snapshots: Vec<(File, Document, Layout)>,
}

/// Section `s`, created at the end of the document if the model removed it. Borrowing only the
/// editor lets callers keep reading the model while they hold the handle.
fn section_s<'e>(editor: &'e mut Editor, model: &mut Model) -> (SectionEditor<'e>, usize) {
    let id = model.current();
    let section = editor.section_or_insert("s").expect("`s` is a valid name");
    (section, id)
}

/// The model's line for the new entry that `action` writes, if its key and value can be
/// written; otherwise expect the editor's error, `result`, and add no entry.
fn written_entry(
    model: &mut Model,
    action: &Action<'_>,
    result: Result<(), EditError>,
) -> Option<(Content, Option<char>)> {
    let error = action.new_entry_error();
    assert_eq!(result.err(), error);
    error.is_none().then(|| {
        let (entry, last) = action.new_entry();
        (Content::Entry(model.new_entry(entry)), last)
    })
}

/// The model's lines for the raw lines `lines`, if the editor accepts them all; otherwise expect
/// the editor's error for the first one it refuses, `result`, and add nothing.
fn written_lines(
    model: &mut Model,
    lines: &[usize],
    result: Result<(), EditError>,
) -> Option<Vec<(Content, Option<char>)>> {
    let refused = lines
        .iter()
        .position(|&line| matches!(model.raw[line].raw, Raw::Refused));
    let error = refused.map(|index| EditError::InvalidLine {
        index,
        line: model.raw[lines[index]].text.clone(),
    });
    assert_eq!(result.err(), error);
    if error.is_some() {
        return None;
    }
    let new = lines.iter().map(|&line| {
        let RawLine { text, raw } = &model.raw[line];
        let last = text.chars().next_back();
        match raw.clone() {
            Raw::Line(content) => (content, last),
            Raw::Entry(entry) => (Content::Entry(model.new_entry(entry)), last),
            Raw::Refused => unreachable!("refused above"),
        }
    });
    Some(new.collect())
}

/// Add the new entry that `action` writes, if any, at the content end of section `id`.
fn append_entry(model: &mut Model, id: usize, action: &Action<'_>, result: Result<(), EditError>) {
    if let Some(line) = written_entry(model, action, result) {
        let at = model.content_end(id);
        model.insert(id, at, &[line]);
    }
}

/// Raw lines at the content end of `section`, the model's section `id`.
fn append_lines(
    model: &mut Model,
    id: usize,
    section: &mut SectionEditor<'_>,
    action: &Action<'_>,
) {
    let texts: Vec<&str> = action
        .lines
        .iter()
        .map(|&line| model.raw[line].text.as_str())
        .collect();
    let result = section.append_body_lines(&texts);
    if let Some(new) = written_lines(model, &action.lines, result) {
        let at = model.content_end(id);
        model.insert(id, at, &new);
    }
}

/// `set` on `section`, the model's section `id`: an assignment or an append.
fn set(model: &mut Model, id: usize, section: &mut SectionEditor<'_>, action: &Action<'_>) {
    let existing = model.find(id, action.key);
    let result = section.set(action.key, action.value);
    match existing {
        Some(_) if action.value_hostile => {
            let error = EditError::InvalidValue {
                value: action.value.into(),
            };
            assert_eq!(result, Err(error));
        }
        Some(entry) => {
            result.unwrap();
            model.assign(id, entry, action.value);
        }
        None => append_entry(model, id, action, result),
    }
}

/// Remove a range of body lines of `section`, the model's section `id`: the range starts at
/// a position the selector picks and is 0 to 3 lines long or reversed. A reversed range and one
/// that ends past the last body line are refused.
fn remove_range(
    model: &mut Model,
    id: usize,
    section: &mut SectionEditor<'_>,
    action: &Action<'_>,
) {
    let count = model.line_count(id);
    let start = action.selector % (count + 1);
    let range = match usize::from(action.span % 5) {
        4 => start + 1..start,
        length => start..start + length,
    };
    let result = section.remove_body_lines(range.clone());
    if range.start > range.end || range.end > count {
        let error = EditError::RangeOutOfRange {
            start: range.start,
            end: range.end,
            body_line_count: count,
        };
        assert_eq!(result, Err(error));
    } else {
        result.unwrap();
        model.remove(id, range);
    }
}

impl Harness {
    fn apply_by_key(&mut self, action: &Action<'_>) {
        if action.operation == 4 {
            self.rename(action);
            return;
        }
        let (mut section, id) = section_s(&mut self.editor, &mut self.model);
        match action.operation {
            0 => set(&mut self.model, id, &mut section, action),
            1 => {
                let result = section.append_entry(action.key, action.value);
                append_entry(&mut self.model, id, action, result);
            }
            2 => {
                let result = section.insert_entry(0, action.key, action.value);
                if let Some(line) = written_entry(&mut self.model, action, result) {
                    self.model.insert(id, 0, &[line]);
                }
            }
            _ => {
                let entry = self.model.find(id, action.key);
                assert_eq!(section.remove_entry(action.key), Ok(entry.is_some()));
                if let Some(entry) = entry {
                    self.model.remove_entry(id, entry);
                }
            }
        }
    }

    /// `rename_key`: absent, a duplicate, a refused key, or renamed.
    fn rename(&mut self, action: &Action<'_>) {
        let new_index = action.selector % KEYS.len();
        let new = KEYS[new_index];
        let (mut section, id) = section_s(&mut self.editor, &mut self.model);
        let entry = self.model.find(id, action.key);
        let collides = action.key != new && self.model.find(id, new).is_some();
        let expected = match entry {
            None => Ok(false),
            Some(_) if collides => Err(EditError::DuplicateKey { key: new.into() }),
            Some(_) if new_index >= HOSTILE_KEYS => Err(EditError::InvalidKey { key: new.into() }),
            Some(_) => Ok(true),
        };
        assert_eq!(section.rename_key(action.key, new), expected);
        if expected == Ok(true) {
            self.model.entries[entry.unwrap()].key = new.into();
        }
    }

    /// An entry (even selectors) or raw lines (odd ones) at a body position that the selector
    /// picks, one past the last position included, which is refused.
    fn insert_at(&mut self, action: &Action<'_>) {
        let (mut section, id) = section_s(&mut self.editor, &mut self.model);
        let count = self.model.line_count(id);
        let at = action.selector / 2 % (count + 2);
        let entry = action.selector % 2 == 0;
        let result = if entry {
            section.insert_entry(at, action.key, action.value)
        } else {
            let raw = &self.model.raw;
            let texts: Vec<&str> = action.lines.iter().map(|&line| &*raw[line].text).collect();
            section.insert_body_lines(at, &texts)
        };
        if at > count {
            let error = EditError::PositionOutOfRange {
                at,
                body_line_count: count,
            };
            assert_eq!(result, Err(error));
            return;
        }
        let new = if entry {
            written_entry(&mut self.model, action, result).map(|line| vec![line])
        } else {
            written_lines(&mut self.model, &action.lines, result)
        };
        if let Some(new) = new {
            self.model.insert(id, at, &new);
        }
    }

    fn apply_by_position(&mut self, action: &Action<'_>) {
        let (mut section, id) = section_s(&mut self.editor, &mut self.model);
        let ids: Vec<usize> = self.model.entry_ids(id).collect();
        if ids.is_empty() {
            assert!(section.entry_at(0).is_none());
            return;
        }
        let index = action.selector % ids.len();
        let entry_id = ids[index];
        let mut entry = section.entry_at(index).expect("the model has the entry");
        assert_eq!(entry.index(), index);
        assert_eq!(entry.body_line(), self.model.position(id, entry_id));
        match action.operation {
            6 | 16 => {
                let (value, hostile) = if action.operation == 6 {
                    (action.value, action.value_hostile)
                } else {
                    ("", false)
                };
                let result = entry.set_value(value);
                assert_eq!(result.is_err(), hostile);
                if !hostile {
                    self.model.assign(id, entry_id, value);
                }
            }
            7 => {
                let result = entry.set_key(action.key);
                assert_eq!(result.is_err(), action.key_hostile);
                if !action.key_hostile {
                    self.model.entries[entry_id].key = action.key.into();
                    assert_eq!(entry.key(), action.key);
                }
            }
            _ => {
                entry.remove().unwrap();
                self.model.remove_entry(id, entry_id);
                return;
            }
        }
        // The handle keeps addressing its entry, at the same index; a moved comment is above it.
        assert_eq!(entry.index(), index);
        assert_eq!(entry.body_line(), self.model.position(id, entry_id));
    }

    /// The operations through section `s` reached by its index among the sections.
    fn apply_by_section_index(&mut self, action: &Action<'_>) {
        let Some(id) = self.model.current else {
            assert!(self.editor.section("s").is_none());
            return;
        };
        let index = usize::from(self.model.after_other);
        let mut section = self.editor.section_at(index).expect("the model has `s`");
        assert_eq!(section.name().as_deref(), Some("s"));
        match action.operation {
            10 => remove_range(&mut self.model, id, &mut section, action),
            11 => {
                let result = section.append_entry(action.key, action.value);
                append_entry(&mut self.model, id, action, result);
            }
            12 => set(&mut self.model, id, &mut section, action),
            13 => append_lines(&mut self.model, id, &mut section, action),
            14 => {
                section.remove();
                self.model.current = None;
            }
            _ => empty_edits(&mut section),
        }
    }

    fn apply(&mut self, action: &Action<'_>) {
        match action.operation {
            0..=4 => self.apply_by_key(action),
            5 => self.insert_at(action),
            6..=8 | 16 => self.apply_by_position(action),
            9 => {
                if let Some(section) = self.editor.section("s") {
                    section.remove();
                }
                self.model.current = None;
            }
            17 => {
                // A removal through a handle found by key.
                let (mut section, id) = section_s(&mut self.editor, &mut self.model);
                let entry = self.model.find(id, action.key);
                match section.entry(action.key) {
                    Some(handle) => {
                        let entry = entry.expect("the model has the key");
                        assert_eq!(handle.body_line(), self.model.position(id, entry));
                        handle.remove().unwrap();
                        self.model.remove_entry(id, entry);
                    }
                    None => assert!(entry.is_none()),
                }
            }
            _ => self.apply_by_section_index(action),
        }
    }

    fn check(&mut self, options: &ParseOptions) {
        self.model.assert_line_breaks();
        let expected = self.model.document();
        let layout = self.model.layout();
        let live = self.editor.file();
        assert_eq!(read(&live), expected, "live values");
        assert_eq!(read_layout(&live), layout, "live body lines");
        let output = self.editor.finish();
        let parsed = parse_with(&output, options);
        // Only the malformed lines have diagnostics, one each.
        assert_eq!(
            parsed.errors().len(),
            self.model.error_count(),
            "{output:?}: {:?}",
            parsed.errors()
        );
        assert_eq!(
            read(&File::cast(parsed.syntax()).unwrap()),
            expected,
            "{output:?}"
        );
        assert_eq!(
            live.syntax().green().to_owned(),
            *parsed.green(),
            "{output:?}"
        );
        for (snapshot, expected, layout) in &self.snapshots {
            assert_eq!(read(snapshot), *expected, "retained file snapshot");
            assert_eq!(read_layout(snapshot), *layout, "retained snapshot lines");
        }
        self.check_positions(&expected, &layout);
    }

    /// The editor's positions in every container, reached by section index, against the model's
    /// layout: `body_line_count()`, and every entry's key, value, `index()` and `body_line()`.
    fn check_positions(&mut self, expected: &Document, layout: &Layout) {
        assert_eq!(self.editor.section_count(), layout.len() - 1);
        for (container, ((name, contents), lines)) in expected.iter().zip(layout).enumerate() {
            let mut section = match container {
                0 => self.editor.preamble(),
                _ => self
                    .editor
                    .section_at(container - 1)
                    .expect("the model has it"),
            };
            assert_eq!(&section.name(), name);
            assert_eq!(section.body_line_count(), lines.len());
            let body_lines: Vec<usize> = (0..lines.len())
                .filter(|&line| lines[line] == SyntaxKind::ENTRY)
                .collect();
            assert_eq!(body_lines.len(), contents.len());
            assert_eq!(section.entry_count(), contents.len());
            for (index, ((key, value), body_line)) in contents.iter().zip(body_lines).enumerate() {
                let entry = section.entry_at(index).expect("the model has the entry");
                assert_eq!(entry.index(), index);
                assert_eq!(entry.body_line(), body_line);
                assert_eq!(&entry.key(), key);
                assert_eq!(&entry.value(), value);
            }
            assert!(section.entry_at(contents.len()).is_none());
        }
    }
}

/// Empty edits change nothing; one position past the end is an error.
fn empty_edits(section: &mut SectionEditor<'_>) {
    let before = section.editor().finish();
    let end = section.body_line_count();
    section.append_body_lines(&[]).unwrap();
    section.insert_body_lines(end, &[]).unwrap();
    section.remove_body_lines(end..end).unwrap();
    let position = section.insert_body_lines(end + 1, &[]);
    assert!(matches!(
        position,
        Err(EditError::PositionOutOfRange { .. })
    ));
    let range = section.remove_body_lines(end + 1..end + 1);
    assert!(matches!(range, Err(EditError::RangeOutOfRange { .. })));
    assert_eq!(section.editor().finish(), before);
}

struct Scenario {
    options: ParseOptions,
    edit: EditOptions,
    source: String,
    values: [String; 10],
    model: Model,
}

/// The values that edits assign: valid ones first, then the refused ones, from
/// [`HOSTILE_VALUES`] on. `continued` is a value continued over two lines.
fn values(continued: &str, ending: &str) -> [String; 10] {
    let blank_continued = format!("head \\{ending}");
    [
        "",
        "next",
        "λ",
        "#fff",
        continued,
        &blank_continued,
        "a;b#c",
        // Refused: a line break, a trailing space, a trailing backslash.
        "a\nb",
        "x ",
        "v\\",
    ]
    .map(str::to_owned)
}

/// The body of section `s`: its source text, its entries and the model's lines, built together
/// from one list. Two `dup` entries, the bare key `flag`, the continued entry `multi` and a blank
/// line. Mixed line endings end `dup=second` with a lone CR, put a blank line after `flag`, and
/// make both blank lines a lone LF, so that removing `flag` merges the first one into that CR
/// (rule D). Where the line ending is CR, the lines before them then end with CRLF: a CR directly
/// before the LF would read together with it.
fn body(options: &ParseOptions, ending: Break, mixed: bool, continued: &str) -> Body {
    let comment = if options.inline_comments {
        " ; keep"
    } else {
        ""
    };
    // A bare key can carry an inline comment too; without bare keys, `flag=` has an empty value.
    let (flag, flag_value) = if options.allow_no_value {
        (format!("flag{comment}"), None)
    } else {
        ("flag=".to_owned(), Some(String::new()))
    };
    let entry = |key: &str, value: Option<String>, comment| Entry {
        key: key.into(),
        value,
        comment,
    };
    let entries = vec![
        entry("dup", Some("first".into()), options.inline_comments),
        entry("dup", Some("second".into()), false),
        entry(
            "flag",
            flag_value,
            options.allow_no_value && options.inline_comments,
        ),
        entry("multi", Some(continued.to_owned()), false),
    ];
    let (second, before_blank, blank_end) = match (mixed, ending) {
        (false, _) => (ending, ending, ending),
        (true, Break::Cr) => (Break::Cr, Break::CrLf, Break::Lf),
        (true, _) => (Break::Cr, ending, Break::Lf),
    };
    let blank = Content::Blank { bare: true };
    let mut lines = vec![
        (format!("dup : first{comment}"), Content::Entry(0), ending),
        ("dup=second  ".to_owned(), Content::Entry(1), second),
        (flag, Content::Entry(2), before_blank),
    ];
    if mixed {
        lines.push((String::new(), blank, blank_end));
    }
    lines.push((
        format!("multi={continued}"),
        Content::Entry(3),
        before_blank,
    ));
    lines.push((String::new(), blank, blank_end));
    Body {
        text: lines
            .iter()
            .flat_map(|(text, _, end)| [text.as_str(), end.text()])
            .collect(),
        entries,
        lines: lines
            .into_iter()
            .map(|(_, content, end)| Line { content, end })
            .collect(),
    }
}

/// The body of section `s` in the source.
struct Body {
    text: String,
    entries: Vec<Entry>,
    lines: Vec<Line>,
}

fn scenario(data: &[u8]) -> Scenario {
    let options = ParseOptions {
        allow_no_value: data[0] & 1 != 0,
        inline_comments: data[0] & 2 != 0,
    };
    let line_ending = [Break::Lf, Break::CrLf, Break::Cr][usize::from(data[2]) % 3];
    let ending = line_ending.text();
    let continued = format!("head \\{ending}  tail");
    let values = values(&continued, ending);
    let edit = EditOptions {
        separator_spacing: match data[1] % 3 {
            0 => SeparatorSpacing::Preserve,
            1 => SeparatorSpacing::Compact,
            _ => SeparatorSpacing::exact("\t", "  ").expect("spaces and tabs"),
        },
    };
    let bom = if data[0] & 4 != 0 { "\u{FEFF}" } else { "" };
    let Body {
        text,
        entries,
        lines,
    } = body(&options, line_ending, data[0] & 8 != 0, &continued);
    let source = format!("{bom}global=stay{ending}[s]{ending}{text}[other]{ending}keep=value");
    let model = Model {
        entries,
        sections: vec![Section {
            header: line_ending,
            lines,
        }],
        current: Some(0),
        after_other: false,
        other_blank: false,
        ending: line_ending,
        raw: raw_lines(&options, ending),
    };
    Scenario {
        options,
        edit,
        source,
        values,
        model,
    }
}

/// Run at most 128 operations; every byte sequence describes valid API inputs.
pub(crate) fn check(data: &[u8]) {
    if data.len() < 3 {
        return;
    }
    let Scenario {
        options,
        edit,
        source,
        values,
        model,
    } = scenario(&data[..3]);
    let editor = Editor::new(&source, &options, &edit).expect("a small source");
    let initial = (editor.file(), model.document(), model.layout());
    let mut harness = Harness {
        snapshots: vec![initial],
        editor,
        model,
    };
    harness.check(&options);
    let raw_count = harness.model.raw.len();
    for (step, bytes) in data[3..].chunks_exact(4).take(128).enumerate() {
        let key = usize::from(bytes[2]) % KEYS.len();
        let value = usize::from(bytes[3]) % values.len();
        let mut lines = vec![usize::from(bytes[3]) % raw_count];
        if bytes[2] & 8 != 0 {
            lines.push(usize::from(bytes[2] >> 4) % raw_count);
        }
        let action = Action {
            operation: bytes[0] % 18,
            selector: usize::from(bytes[1]),
            key: KEYS[key],
            key_hostile: key >= HOSTILE_KEYS,
            value: &values[value],
            value_hostile: value >= HOSTILE_VALUES,
            lines,
            span: bytes[3],
        };
        harness.apply(&action);
        harness.check(&options);
        if step % 16 == 0 && harness.snapshots.len() < 8 {
            let model = &harness.model;
            let snapshot = (harness.editor.file(), model.document(), model.layout());
            harness.snapshots.push(snapshot);
        }
    }
}
