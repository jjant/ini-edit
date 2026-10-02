//! Generate syntax and its meaning together, without asking the parser for
//! expected values or text ranges. Keep every unchanged byte in the reference.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse_with};

type Contents = Vec<(String, Option<String>)>;
type Meaning = Vec<(Option<String>, Contents)>;

struct Bytes<'a> {
    input: &'a [u8],
    offset: usize,
}

impl Bytes<'_> {
    fn next(&mut self) -> u8 {
        let value = self.input.get(self.offset).copied().unwrap_or(0);
        self.offset += 1;
        value
    }

    fn choose<'a>(&mut self, choices: &'a [&'a str]) -> &'a str {
        choices[usize::from(self.next()) % choices.len()]
    }

    fn space(&mut self) -> String {
        self.choose(&["", " ", "\t", " \t", "\t  "]).into()
    }

    fn ending(&mut self) -> String {
        self.choose(&["\n", "\r\n", "\r"]).into()
    }

    fn word(&mut self, prefix: &str) -> String {
        let mut result = prefix.to_owned();
        for _ in 0..=self.next() % 8 {
            result.push_str(self.choose(&[
                "a", "b", "λ", "é", "e\u{301}", "🙂", "-", "_", ".", "\u{FEFF}",
            ]));
        }
        result
    }

    fn value(&mut self) -> String {
        let mut value = self.word("v");
        value.push_str(self.choose(&["", "#fff", "a;b#c", " two words", "=[x]:y", "\u{a0}"]));
        for _ in 0..self.next() % 4 {
            value.push_str(" \\");
            value.push_str(&self.space());
            value.push_str(&self.ending());
            value.push_str(&self.space());
            value.push_str(&self.word("tail"));
        }
        value
    }
}

#[derive(Clone)]
struct Entry {
    indent: String,
    key: String,
    before: String,
    separator: Option<char>,
    after: String,
    value: Option<String>,
    suffix: String,
    ending: String,
}

impl Entry {
    fn render(&self) -> String {
        let mut text = format!("{}{}{}", self.indent, self.key, self.before);
        if let Some(separator) = self.separator {
            text.push(separator);
            text.push_str(&self.after);
            text.push_str(self.value.as_deref().unwrap());
        }
        text.push_str(&self.suffix);
        text.push_str(&self.ending);
        text
    }

    // The replacement has a nonblank final line. Comments and trailing
    // whitespace therefore stay at their exact original positions relative
    // to the value. Blank-final-line edits have their own stateful oracle.
    fn assign(&mut self, value: &str, spacing: &SeparatorSpacing) {
        let gap = match spacing {
            SeparatorSpacing::Preserve if self.separator.is_some() => None,
            SeparatorSpacing::Preserve => Some((" ", " ")),
            SeparatorSpacing::Compact => Some(("", "")),
            SeparatorSpacing::Exact { before, after } => Some((before.as_str(), after.as_str())),
        };
        if let Some((before, after)) = gap {
            self.before = before.into();
            self.after = after.into();
        }
        self.separator.get_or_insert('=');
        self.value = Some(value.into());
    }
}

#[derive(Clone)]
enum Line {
    Entry(Entry),
    Trivia(String),
}

#[derive(Clone)]
struct Section {
    name: Option<String>,
    header: String,
    lines: Vec<Line>,
}

impl Section {
    fn entries(&self) -> Vec<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| matches!(line, Line::Entry(_)).then_some(index))
            .collect()
    }

    fn entry(&self, index: usize) -> &Entry {
        let Line::Entry(entry) = &self.lines[index] else {
            unreachable!()
        };
        entry
    }

    fn entry_mut(&mut self, index: usize) -> &mut Entry {
        let Line::Entry(entry) = &mut self.lines[index] else {
            unreachable!()
        };
        entry
    }

    fn first_key(&self, key: &str) -> usize {
        self.entries()
            .into_iter()
            .find(|&index| self.entry(index).key == key)
            .unwrap()
    }
}

#[derive(Clone)]
struct Document {
    bom: String,
    sections: Vec<Section>,
}

impl Document {
    fn render(&self) -> String {
        let mut text = self.bom.clone();
        for section in &self.sections {
            text.push_str(&section.header);
            for line in &section.lines {
                match line {
                    Line::Entry(entry) => text.push_str(&entry.render()),
                    Line::Trivia(trivia) => text.push_str(trivia),
                }
            }
        }
        text
    }

    fn separate_blank_lines(&mut self) {
        let mut previous_cr = false;
        for section in &mut self.sections {
            if !section.header.is_empty() {
                previous_cr = section.header.ends_with('\r');
            }
            for line in &mut section.lines {
                match line {
                    Line::Entry(entry) => previous_cr = entry.render().ends_with('\r'),
                    Line::Trivia(text) => {
                        // Preserve two physical line breaks when deleting a
                        // line brings a CR and an LF blank line together.
                        // Store the chosen ending so later edits retain it.
                        if previous_cr && text.starts_with('\n') {
                            text.insert(0, '\r');
                        }
                        previous_cr = text.ends_with('\r');
                    }
                }
            }
        }
    }

    fn meaning(&self) -> Meaning {
        self.sections
            .iter()
            .map(|section| {
                let entries = section
                    .entries()
                    .into_iter()
                    .map(|index| {
                        let entry = section.entry(index);
                        (entry.key.clone(), entry.value.clone())
                    })
                    .collect();
                (section.name.clone(), entries)
            })
            .collect()
    }
}

fn entry(bytes: &mut Bytes<'_>, keys: &[String], options: &ParseOptions) -> Entry {
    let indent = bytes.space();
    let key = keys[usize::from(bytes.next()) % keys.len()].clone();
    let before = bytes.space();
    let separator = if bytes.next() % 2 == 0 { '=' } else { ':' };
    let mut after = bytes.space();
    let kind = bytes.next() % 4;
    let value = match kind {
        0 => Some(String::new()),
        1 if options.allow_no_value => None,
        _ => Some(bytes.value()),
    };
    let mut suffix = bytes.space();
    if value.as_deref().is_some_and(|value| !value.is_empty()) {
        if options.inline_comments && bytes.next() % 2 == 0 {
            suffix.push_str(bytes.choose(&[" ; note λ", "\t# note é"]));
        }
    } else if value.is_some() {
        // An empty value has one separator gap, not value-trailing trivia.
        after.push_str(&suffix);
        suffix.clear();
    } else {
        suffix.clear();
    }
    Entry {
        indent,
        key,
        before,
        separator: value.as_ref().map(|_| separator),
        after,
        value,
        suffix,
        ending: bytes.ending(),
    }
}

fn lines(bytes: &mut Bytes<'_>, keys: &[String], options: &ParseOptions) -> Vec<Line> {
    let mut lines = Vec::new();
    for _ in 0..=bytes.next() % 4 {
        match bytes.next() % 3 {
            0 => lines.push(Line::Trivia(format!("{}{}", bytes.space(), bytes.ending()))),
            1 => lines.push(Line::Trivia(format!(
                "{}; retained 🙂{}",
                bytes.space(),
                bytes.ending()
            ))),
            _ => {}
        }
        lines.push(Line::Entry(entry(bytes, keys, options)));
    }
    if bytes.next() % 2 == 0 {
        lines.push(Line::Trivia(format!("\t# tail{}", bytes.ending())));
    }
    lines
}

fn generate(bytes: &mut Bytes<'_>, options: &ParseOptions) -> Document {
    let bom = if bytes.next() % 2 == 0 {
        ""
    } else {
        "\u{FEFF}"
    }
    .into();
    let keys: Vec<_> = (0..4).map(|_| bytes.word("k")).collect();
    let names: Vec<_> = (0..3).map(|_| bytes.word("s")).collect();
    let mut sections = vec![Section {
        name: None,
        header: String::new(),
        lines: lines(bytes, &keys, options),
    }];
    for _ in 0..=bytes.next() % 4 {
        let name = names[usize::from(bytes.next()) % names.len()].clone();
        let header = format!(
            "{}[{}{}{}]{}; header{}",
            bytes.space(),
            bytes.space(),
            name,
            bytes.space(),
            bytes.space(),
            bytes.ending()
        );
        sections.push(Section {
            name: Some(name),
            header,
            lines: lines(bytes, &keys, options),
        });
    }
    if bytes.next() % 2 == 0 {
        match sections.last_mut().unwrap().lines.last_mut().unwrap() {
            Line::Entry(entry) => entry.ending.clear(),
            Line::Trivia(text) => *text = text.trim_end_matches(['\r', '\n']).into(),
        }
    }
    let mut document = Document { bom, sections };
    document.separate_blank_lines();
    document
}

fn read(file: &File) -> Meaning {
    let entries = |entries: Vec<ini_edit::ast::Entry>| -> Contents {
        entries
            .into_iter()
            .map(|entry| (entry.key().unwrap(), entry.value()))
            .collect()
    };
    let mut result = vec![(None, entries(file.preamble_entries().collect()))];
    result.extend(
        file.sections()
            .map(|section| (section.name(), entries(section.entries().collect()))),
    );
    result
}

struct Edit<'a> {
    operation: u8,
    section: usize,
    position: usize,
    replacement_key: &'a str,
    replacement_value: &'a str,
    spacing: &'a SeparatorSpacing,
}

fn apply(editor: &Editor, expected: &mut Document, edit: &Edit<'_>) {
    let section = &mut expected.sections[edit.section];
    let name = section.name.as_deref().unwrap();
    let actual = editor.section(name);
    let index = section.entries()[edit.position];
    let key = section.entry(index).key.clone();
    let mut handles = actual.entries_mut();
    assert_eq!(handles.len(), section.entries().len());
    match edit.operation % 8 {
        0 => {
            actual.set(&key, edit.replacement_value);
            let first = section.first_key(&key);
            section
                .entry_mut(first)
                .assign(edit.replacement_value, edit.spacing);
        }
        1 => {
            handles[edit.position].set_value(edit.replacement_value);
            section
                .entry_mut(index)
                .assign(edit.replacement_value, edit.spacing);
        }
        2 => {
            let allowed = key == edit.replacement_key
                || !section
                    .entries()
                    .iter()
                    .any(|&i| section.entry(i).key == edit.replacement_key);
            assert_eq!(actual.rename_key(&key, edit.replacement_key), allowed);
            if allowed {
                let first = section.first_key(&key);
                section.entry_mut(first).key = edit.replacement_key.into();
            }
        }
        3 => {
            handles[edit.position].set_key(edit.replacement_key);
            section.entry_mut(index).key = edit.replacement_key.into();
        }
        4 => {
            assert!(actual.remove_entry(&key));
            section.lines.remove(section.first_key(&key));
        }
        5 => {
            handles.remove(edit.position).remove();
            section.lines.remove(index);
        }
        6 => {
            actual.remove();
            expected.sections.remove(edit.section);
        }
        _ => {
            actual.append_raw_lines(&[]);
            actual.insert_raw_lines_at(usize::MAX, &[]);
            actual.remove_lines(usize::MAX..usize::MAX);
        }
    }
    expected.separate_blank_lines();
}

/// Generate a whole document and compare edits against independent text and
/// meaning. Input size, line counts, word lengths, and continuations are bounded.
pub fn check(input: &[u8]) {
    let mut bytes = Bytes {
        input: &input[..input.len().min(4096)],
        offset: 0,
    };
    let operation = bytes.next();
    let section_selector = usize::from(bytes.next());
    let entry_selector = usize::from(bytes.next());
    let style = bytes.next() % 3;
    let flags = bytes.next();
    let options = ParseOptions {
        allow_no_value: flags & 1 != 0,
        inline_comments: flags & 2 != 0,
    };
    let spacing = match style {
        0 => SeparatorSpacing::Preserve,
        1 => SeparatorSpacing::Compact,
        _ => SeparatorSpacing::exact(bytes.space(), bytes.space()),
    };
    let mut expected = generate(&mut bytes, &options);
    let original = expected.render();
    let parsed = parse_with(&original, &options);
    assert!(
        parsed.errors().is_empty(),
        "{original:?}: {:?}",
        parsed.errors()
    );
    assert_eq!(
        read(&File::cast(parsed.syntax()).unwrap()),
        expected.meaning(),
        "{original:?}"
    );
    let selected = 1 + section_selector % (expected.sections.len() - 1);
    let name = expected.sections[selected].name.clone();
    // Named APIs intentionally address the first matching duplicate section.
    let section = expected
        .sections
        .iter()
        .position(|section| section.name == name)
        .unwrap();
    let positions = expected.sections[section].entries();
    let position = entry_selector % positions.len();
    let replacement_key = if bytes.next() % 2 == 0 {
        expected.sections[section]
            .entry(positions[(position + 1) % positions.len()])
            .key
            .clone()
    } else {
        bytes.word("renamed")
    };
    let replacement_value = bytes.value();
    let editor = Editor::with_options(
        &original,
        &options,
        &EditOptions {
            separator_spacing: spacing.clone(),
        },
    );
    let snapshot = editor.file();
    apply(
        &editor,
        &mut expected,
        &Edit {
            operation,
            section,
            position,
            replacement_key: &replacement_key,
            replacement_value: &replacement_value,
            spacing: &spacing,
        },
    );
    let output = editor.finish();
    assert_eq!(
        output,
        expected.render(),
        "operation={operation} source={original:?}"
    );
    assert_eq!(read(&editor.file()), expected.meaning());
    let reparsed = parse_with(&output, &options);
    assert!(
        reparsed.errors().is_empty(),
        "{output:?}: {:?}",
        reparsed.errors()
    );
    assert_eq!(
        read(&File::cast(reparsed.syntax()).unwrap()),
        expected.meaning()
    );
    assert_eq!(
        editor.file().syntax().green().into_owned(),
        *reparsed.green()
    );
    assert_eq!(snapshot.syntax().text().to_string(), original);
}
