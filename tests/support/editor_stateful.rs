//! Independent identity-based model for edits, duplicate keys, and retained handles.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditOptions, Editor, EntryEditor, SectionEditor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse_with};

type Contents = Vec<(String, Option<String>)>;
type Document = Vec<(Option<String>, Contents)>;
const KEYS: [&str; 4] = ["dup", "flag", "λ", "new"];

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

    fn append(&mut self, section: usize, key: &str, value: &str) -> usize {
        let id = self.entries.len();
        self.entries.push(Entry {
            key: key.into(),
            value: Some(value.into()),
        });
        self.sections[section].push(id);
        id
    }

    fn find(&self, section: usize, key: &str) -> Option<usize> {
        self.sections[section]
            .iter()
            .copied()
            .find(|&id| self.entries[id].key == key)
    }

    fn set(&mut self, section: usize, key: &str, value: &str) {
        if let Some(id) = self.find(section, key) {
            self.entries[id].value = Some(value.into());
        } else {
            self.append(section, key, value);
        }
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

struct Harness<'a> {
    editor: &'a Editor,
    model: Model,
    handles: Vec<(EntryEditor, usize)>,
    sections: Vec<(SectionEditor<'a>, usize)>,
    snapshots: Vec<(File, Document)>,
}

impl Harness<'_> {
    fn capture_entries(&mut self, handles: Vec<EntryEditor>, section: usize) {
        let ids = &self.model.sections[section];
        assert_eq!(handles.len(), ids.len());
        for (index, (handle, &id)) in handles.into_iter().zip(ids).enumerate() {
            assert_eq!(handle.index(), index);
            if self.handles.len() < 64 {
                self.handles.push((handle, id));
            }
        }
    }

    fn apply_current(&mut self, operation: u8, selector: usize, key: &str, value: &str) {
        let id = self.model.current();
        let section = self.editor.section("s");
        match operation {
            0 => {
                section.set(key, value);
                self.model.set(id, key, value);
            }
            1 => {
                section.append_entry(key, value);
                self.model.append(id, key, value);
            }
            2 => {
                section.insert_entry_at_line(0, key, value);
                let entry = self.model.append(id, key, value);
                self.model.sections[id].pop();
                self.model.sections[id].insert(0, entry);
            }
            3 => {
                let entry = self.model.find(id, key);
                assert_eq!(section.remove_entry(key), entry.is_some());
                if let Some(entry) = entry {
                    self.model.remove_entry(entry);
                }
            }
            4 => {
                let new = KEYS[selector % KEYS.len()];
                let entry = self.model.find(id, key);
                let allowed = entry.is_some() && (key == new || self.model.find(id, new).is_none());
                assert_eq!(section.rename_key(key, new), allowed);
                if allowed {
                    self.model.entries[entry.unwrap()].key = new.into();
                }
            }
            5 => self.capture_entries(section.entries_mut(), id),
            8 => {
                let entries = &self.model.sections[id];
                if !entries.is_empty() {
                    let index = selector % entries.len();
                    let entry = entries[index];
                    section
                        .entries_mut()
                        .into_iter()
                        .nth(index)
                        .unwrap()
                        .remove();
                    self.model.remove_entry(entry);
                }
            }
            9 => {
                section.remove();
                self.model.current = None;
            }
            10 => {
                if self.sections.len() < 16 {
                    self.sections.push((section, id));
                }
            }
            _ => unreachable!(),
        }
    }

    fn apply_retained(&mut self, operation: u8, selector: usize, key: &str, value: &str) {
        match operation {
            6 | 7 | 16 | 17 if !self.handles.is_empty() => {
                let index = selector % self.handles.len();
                let (handle, id) = &self.handles[index];
                match operation {
                    6 => {
                        handle.set_value(value);
                        self.model.entries[*id].value = Some(value.into());
                    }
                    7 => {
                        handle.set_key(key);
                        self.model.entries[*id].key = key.into();
                    }
                    16 => {
                        handle.set_value("");
                        self.model.entries[*id].value = Some(String::new());
                    }
                    _ => {
                        let (handle, id) = self.handles.swap_remove(index);
                        handle.remove();
                        self.model.remove_entry(id);
                    }
                }
            }
            11..=15 if !self.sections.is_empty() => {
                let index = selector % self.sections.len();
                let (section, id) = &self.sections[index];
                match operation {
                    11 => {
                        section.append_entry(key, value);
                        self.model.append(*id, key, value);
                    }
                    12 => {
                        section.set(key, value);
                        self.model.set(*id, key, value);
                    }
                    13 => self.capture_entries(section.entries_mut(), *id),
                    14 => {
                        let (section, id) = self.sections.swap_remove(index);
                        section.remove();
                        if self.model.current == Some(id) {
                            self.model.current = None;
                        }
                    }
                    _ => {
                        let before = self.editor.finish();
                        section.append_raw_lines(&[]);
                        section.insert_raw_lines_at(usize::MAX, &[]);
                        assert_eq!(self.editor.finish(), before);
                    }
                }
            }
            _ => {}
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
        for (handle, id) in &self.handles {
            assert_eq!(
                handle.key().as_deref(),
                Some(self.model.entries[*id].key.as_str())
            );
            assert_eq!(handle.value(), self.model.entries[*id].value);
        }
        for (section, id) in &self.sections {
            let actual: Contents = section
                .entries_mut()
                .iter()
                .map(|entry| (entry.key().unwrap(), entry.value()))
                .collect();
            assert_eq!(actual, self.model.contents(*id), "retained section {id}");
        }
        for (snapshot, expected) in &self.snapshots {
            assert!(!snapshot.syntax().is_mutable());
            assert_eq!(read(snapshot), *expected, "retained file snapshot");
        }
    }
}

struct Scenario {
    options: ParseOptions,
    edit: EditOptions,
    source: String,
    values: [String; 7],
    model: Model,
}

fn scenario(data: &[u8]) -> Scenario {
    let options = ParseOptions {
        allow_no_value: data[0] & 1 != 0,
        inline_comments: data[0] & 2 != 0,
    };
    let ending = ["\n", "\r\n", "\r"][usize::from(data[2]) % 3];
    let continued = format!("head \\{ending}  tail");
    let blank_continued = format!("head \\{ending}");
    let values = [
        "",
        "next",
        "λ",
        "#fff",
        &continued,
        &blank_continued,
        "a;b#c",
    ]
    .map(str::to_owned);
    let edit = EditOptions {
        separator_spacing: match data[1] % 3 {
            0 => SeparatorSpacing::Preserve,
            1 => SeparatorSpacing::Compact,
            _ => SeparatorSpacing::exact("\t", "  "),
        },
    };
    let bom = if data[0] & 4 != 0 { "\u{FEFF}" } else { "" };
    let comment = if options.inline_comments {
        " ; keep"
    } else {
        ""
    };
    let bare = if options.allow_no_value {
        "flag"
    } else {
        "flag="
    };
    let source = format!(
        "{bom}global=stay{ending}[s]{ending}dup : first{comment}{ending}dup=second  {ending}{bare}{ending}multi={continued}{ending}{ending}[other]{ending}keep=value"
    );
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
            value: Some(continued.clone()),
        },
    ];
    let model = Model {
        entries,
        sections: vec![vec![0, 1, 2, 3]],
        current: Some(0),
        after_other: false,
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
pub fn check(data: &[u8]) {
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
    let editor = Editor::with_options(&source, &options, &edit);
    let initial = model.document();
    let mut harness = Harness {
        editor: &editor,
        model,
        handles: Vec::new(),
        sections: vec![(editor.section("s"), 0)],
        snapshots: vec![(editor.file(), initial)],
    };
    harness.capture_entries(editor.section("s").entries_mut(), 0);
    harness.check(&options);
    for (step, action) in data[3..].chunks_exact(4).take(128).enumerate() {
        let operation = action[0] % 18;
        let selector = usize::from(action[1]);
        let key = KEYS[usize::from(action[2]) % KEYS.len()];
        let value = &values[usize::from(action[3]) % values.len()];
        if matches!(operation, 0..=5 | 8..=10) {
            harness.apply_current(operation, selector, key, value);
        } else {
            harness.apply_retained(operation, selector, key, value);
        }
        harness.check(&options);
        if step % 16 == 0 && harness.snapshots.len() < 8 {
            harness
                .snapshots
                .push((editor.file(), harness.model.document()));
        }
    }
}
