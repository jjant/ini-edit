//! Independent identity-based model for edits, duplicate keys and refused input.
//!
//! Handles are exclusive borrows, so every step acquires the ones it needs, by name, by section
//! index, by entry position or by key. Immutable `File` snapshots taken along the way must keep
//! showing the document as it was. Half of the keys and values are hostile: every operation that
//! would write one must fail and leave the model, and the document, unchanged.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditError, EditOptions, Editor, SectionEditor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse_with};

type Contents = Vec<(String, Option<String>)>;
type Document = Vec<(Option<String>, Contents)>;

/// Keys: four that read back, then four that a reader would not.
const KEYS: [&str; 8] = ["dup", "flag", "λ", "new", "a=b", " k", ";k", ""];
/// Keys from this index on are refused.
const HOSTILE_KEYS: usize = 4;
/// Values from this index on are refused.
const HOSTILE_VALUES: usize = 7;

#[derive(Clone)]
struct Entry {
    key: String,
    value: Option<String>,
}

struct Model {
    entries: Vec<Entry>,
    sections: Vec<Vec<usize>>,
    current: Option<usize>,
    after_other: bool,
}

impl Model {
    /// Section `s`, which a step that needs it recreates at the end of the document.
    fn current(&mut self) -> usize {
        if let Some(id) = self.current {
            return id;
        }
        let id = self.sections.len();
        self.sections.push(Vec::new());
        self.current = Some(id);
        self.after_other = true;
        id
    }

    fn new_entry(&mut self, key: &str, value: &str) -> usize {
        let id = self.entries.len();
        self.entries.push(Entry {
            key: key.into(),
            value: Some(value.into()),
        });
        id
    }

    fn find(&self, section: usize, key: &str) -> Option<usize> {
        self.sections[section]
            .iter()
            .copied()
            .find(|&id| self.entries[id].key == key)
    }

    fn remove_entry(&mut self, id: usize) {
        for section in &mut self.sections {
            section.retain(|&entry| entry != id);
        }
    }

    fn contents(&self, section: usize) -> Contents {
        self.sections[section]
            .iter()
            .map(|&id| {
                let entry = &self.entries[id];
                (entry.key.clone(), entry.value.clone())
            })
            .collect()
    }

    fn document(&self) -> Document {
        let mut result = vec![(None, vec![("global".into(), Some("stay".into()))])];
        let other = (
            Some("other".into()),
            vec![("keep".into(), Some("value".into()))],
        );
        if self.after_other {
            result.push(other.clone());
        }
        if let Some(section) = self.current {
            result.push((Some("s".into()), self.contents(section)));
        }
        if !self.after_other {
            result.push(other);
        }
        result
    }

    /// The index of section `s` among the document's sections.
    fn current_index(&self) -> usize {
        usize::from(self.after_other)
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

/// One step's arguments, decoded from four bytes.
struct Action<'a> {
    operation: u8,
    selector: usize,
    key: &'a str,
    key_hostile: bool,
    value: &'a str,
    value_hostile: bool,
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
}

struct Harness {
    editor: Editor,
    model: Model,
    snapshots: Vec<(File, Document)>,
}

/// Section `s`, created at the end of the document if the model removed it. Borrowing only the
/// editor lets callers keep reading the model while they hold the handle.
fn section_s<'e>(editor: &'e mut Editor, model: &mut Model) -> (SectionEditor<'e>, usize) {
    let id = model.current();
    let section = editor.section_or_insert("s").expect("`s` is a valid name");
    (section, id)
}

/// The model's entry for the new entry that `action` writes, if its key and value can be
/// written; otherwise expect the editor's error, `result`, and add no entry.
fn written_entry(
    model: &mut Model,
    action: &Action<'_>,
    result: Result<(), EditError>,
) -> Option<usize> {
    let error = action.new_entry_error();
    assert_eq!(result.err(), error);
    error
        .is_none()
        .then(|| model.new_entry(action.key, action.value))
}

/// Add the new entry that `action` writes, if any, at the end of section `id`.
fn append_entry(model: &mut Model, id: usize, action: &Action<'_>, result: Result<(), EditError>) {
    let entry = written_entry(model, action, result);
    model.sections[id].extend(entry);
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
            model.entries[entry].value = Some(action.value.into());
        }
        None => append_entry(model, id, action, result),
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
                if let Some(entry) = written_entry(&mut self.model, action, result) {
                    self.model.sections[id].insert(0, entry);
                }
            }
            _ => {
                let entry = self.model.find(id, action.key);
                assert_eq!(section.remove_entry(action.key), Ok(entry.is_some()));
                if let Some(entry) = entry {
                    self.model.remove_entry(entry);
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

    fn apply_by_position(&mut self, action: &Action<'_>) {
        let (mut section, id) = section_s(&mut self.editor, &mut self.model);
        let ids = &self.model.sections[id];
        if ids.is_empty() {
            assert!(section.entry_at(0).is_none());
            return;
        }
        let index = action.selector % ids.len();
        let entry_id = ids[index];
        let mut entry = section.entry_at(index).expect("the model has the entry");
        assert_eq!(entry.index(), index);
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
                    self.model.entries[entry_id].value = Some(value.into());
                    // The handle keeps addressing its entry, at the same index.
                    assert_eq!(entry.index(), index);
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
                self.model.remove_entry(entry_id);
            }
        }
    }

    /// The operations through section `s` reached by its index among the sections.
    fn apply_by_section_index(&mut self, action: &Action<'_>) {
        let Some(id) = self.model.current else {
            assert!(self.editor.section("s").is_none());
            return;
        };
        if matches!(action.operation, 10 | 13) {
            self.check_positions();
            return;
        }
        let index = self.model.current_index();
        let mut section = self.editor.section_at(index).expect("the model has `s`");
        assert_eq!(section.name().as_deref(), Some("s"));
        match action.operation {
            11 => {
                let result = section.append_entry(action.key, action.value);
                append_entry(&mut self.model, id, action, result);
            }
            12 => set(&mut self.model, id, &mut section, action),
            14 => {
                section.remove();
                self.model.current = None;
            }
            _ => empty_edits(&mut section),
        }
    }

    /// Every entry of `s` by position: its index and its key and value, as the model says.
    fn check_positions(&mut self) {
        let Some(id) = self.model.current else {
            return;
        };
        let expected = self.model.contents(id);
        let mut section = self.editor.section("s").expect("the model has `s`");
        assert_eq!(section.entry_count(), expected.len());
        for (index, (key, value)) in expected.iter().enumerate() {
            let entry = section.entry_at(index).expect("the model has the entry");
            assert_eq!(entry.index(), index);
            assert_eq!(&entry.key(), key);
            assert_eq!(&entry.value(), value);
        }
        assert!(section.entry_at(expected.len()).is_none());
    }

    fn apply(&mut self, action: &Action<'_>) {
        match action.operation {
            0..=4 => self.apply_by_key(action),
            5 => self.check_positions(),
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
                        handle.remove().unwrap();
                        self.model
                            .remove_entry(entry.expect("the model has the key"));
                    }
                    None => assert!(entry.is_none()),
                }
            }
            _ => self.apply_by_section_index(action),
        }
    }

    fn check(&self, options: &ParseOptions) {
        let expected = self.model.document();
        let live = self.editor.file();
        assert_eq!(read(&live), expected, "live values");
        let output = self.editor.finish();
        let parsed = parse_with(&output, options);
        assert!(
            parsed.errors().is_empty(),
            "{output:?}: {:?}",
            parsed.errors()
        );
        assert_eq!(
            read(&File::cast(parsed.syntax()).unwrap()),
            expected,
            "{output:?}"
        );
        assert_eq!(
            live.syntax().green().into_owned(),
            *parsed.green(),
            "{output:?}"
        );
        for (snapshot, expected) in &self.snapshots {
            assert!(!snapshot.syntax().is_mutable());
            assert_eq!(read(snapshot), *expected, "retained file snapshot");
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

/// The model of the scenario's section `s`: two `dup` entries, the bare key `flag` and the
/// continued entry `multi`.
fn initial_model(options: &ParseOptions, continued: &str) -> Model {
    let entries = vec![
        Entry {
            key: "dup".into(),
            value: Some("first".into()),
        },
        Entry {
            key: "dup".into(),
            value: Some("second".into()),
        },
        Entry {
            key: "flag".into(),
            value: (!options.allow_no_value).then(String::new),
        },
        Entry {
            key: "multi".into(),
            value: Some(continued.to_owned()),
        },
    ];
    Model {
        entries,
        sections: vec![vec![0, 1, 2, 3]],
        current: Some(0),
        after_other: false,
    }
}

fn scenario(data: &[u8]) -> Scenario {
    let options = ParseOptions {
        allow_no_value: data[0] & 1 != 0,
        inline_comments: data[0] & 2 != 0,
    };
    let ending = ["\n", "\r\n", "\r"][usize::from(data[2]) % 3];
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
    let comment = if options.inline_comments {
        " ; keep"
    } else {
        ""
    };
    // A bare key can carry an inline comment too.
    let bare = if options.allow_no_value {
        format!("flag{comment}")
    } else {
        "flag=".to_owned()
    };
    let source = format!(
        "{bom}global=stay{ending}[s]{ending}dup : first{comment}{ending}dup=second  {ending}\
         {bare}{ending}multi={continued}{ending}{ending}[other]{ending}keep=value"
    );
    let model = initial_model(&options, &continued);
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
    let initial = model.document();
    let mut harness = Harness {
        snapshots: vec![(editor.file(), initial)],
        editor,
        model,
    };
    harness.check(&options);
    for (step, bytes) in data[3..].chunks_exact(4).take(128).enumerate() {
        let key = usize::from(bytes[2]) % KEYS.len();
        let value = usize::from(bytes[3]) % values.len();
        let action = Action {
            operation: bytes[0] % 18,
            selector: usize::from(bytes[1]),
            key: KEYS[key],
            key_hostile: key >= HOSTILE_KEYS,
            value: &values[value],
            value_hostile: value >= HOSTILE_VALUES,
        };
        harness.apply(&action);
        harness.check(&options);
        if step % 16 == 0 && harness.snapshots.len() < 8 {
            let snapshot = (harness.editor.file(), harness.model.document());
            harness.snapshots.push(snapshot);
        }
    }
}
