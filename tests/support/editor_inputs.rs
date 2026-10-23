//! A bidirectional oracle for hostile input (NEG-13).
//!
//! For every hostile string at every site, an independent reference renders the documented text
//! by string concatenation and decides whether it is *representable* by a full parse of that
//! text: every untouched line keeps its container, kind, termination and text; the new lines
//! have the requested meaning (key, value, name; for raw lines, their container and text, not a
//! header, terminated); and a value whose last line ends in a backslash is refused by policy. The
//! editor must succeed exactly when the reference says representable (no false accepts and no
//! false rejects), write exactly the reference's bytes, keep its tree equal to a parse of its
//! text, and change nothing when it refuses.

use std::collections::BTreeSet;

use ini_edit::ast::{AstNode, Entry, Section, SectionHeader};
use ini_edit::editor::{EditError, EditOptions, Editor, SectionEditor, SeparatorSpacing};
use ini_edit::{ParseOptions, SyntaxKind, SyntaxNode, parse_with};

/// One line of a parsed document, as the reference compares it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    /// 0 for the preamble, `k + 1` for the `k`-th section.
    container: usize,
    kind: SyntaxKind,
    text: String,
    terminated: bool,
}

fn line(container: usize, node: &SyntaxNode) -> Line {
    let terminated = node
        .last_child_or_token()
        .is_some_and(|element| element.kind() == SyntaxKind::NEWLINE);
    Line {
        container,
        kind: node.kind(),
        text: node.to_string(),
        terminated,
    }
}

/// The lines of `text`, with their containers.
fn lines_of(text: &str, options: &ParseOptions) -> Vec<Line> {
    let root = parse_with(text, options).syntax();
    let mut lines = Vec::new();
    let mut container = 0;
    for child in root.children() {
        if child.kind() == SyntaxKind::SECTION {
            container += 1;
            lines.extend(child.children().map(|node| line(container, &node)));
        } else {
            lines.push(line(0, &child));
        }
    }
    lines
}

fn byte_order_mark_of(text: &str) -> &'static str {
    if text.starts_with('\u{FEFF}') {
        "\u{FEFF}"
    } else {
        ""
    }
}

/// The first line break of `text`, or LF.
fn ending_of(text: &str) -> &'static str {
    match text.find(['\r', '\n']) {
        Some(start) if text[start..].starts_with("\r\n") => "\r\n",
        Some(start) if text[start..].starts_with('\r') => "\r",
        _ => "\n",
    }
}

/// The documented trailing-backslash policy, stated independently.
fn last_line_continues(value: &str) -> bool {
    let last = &value[value.rfind(['\r', '\n']).map_or(0, |start| start + 1)..];
    last.trim_end_matches([' ', '\t']).ends_with('\\')
}

fn join(lines: &[Line]) -> String {
    lines.iter().map(|line| line.text.as_str()).collect()
}

/// A line break after text ending with `tail`: CRLF where a CR would join an LF ending.
fn terminator_after(tail: &str, ending: &'static str) -> &'static str {
    if tail.ends_with('\r') && ending.starts_with('\n') {
        "\r\n"
    } else {
        ending
    }
}

/// The reference's rendering of an edit and the lines a reader must find in it.
struct Reference {
    text: String,
    expected: Vec<Line>,
    /// Indices into `expected` of the new lines.
    new_lines: Vec<usize>,
    /// For raw lines, compare only container and text of the new lines.
    loose: bool,
}

/// Insert `contents` before flattened line `index`, in `container`.
fn reference_insert(
    document: &str,
    options: &ParseOptions,
    (index, container): (usize, usize),
    contents: &[String],
) -> Reference {
    let lines = lines_of(document, options);
    let ending = ending_of(document);
    let mut before: Vec<Line> = lines[..index].to_vec();
    let after: Vec<Line> = lines[index..].to_vec();
    if let Some(previous) = before.last_mut().filter(|line| !line.terminated) {
        let terminator = terminator_after(&previous.text, ending);
        previous.text.push_str(terminator);
        previous.terminated = true;
    }
    let mut text = format!("{}{}", byte_order_mark_of(document), join(&before));
    let next_first = after
        .first()
        .map(|line| line.text.clone())
        .unwrap_or_default();
    let mut new = Vec::new();
    for (position, content) in contents.iter().enumerate() {
        let tail = if content.is_empty() {
            text.as_str()
        } else {
            content
        };
        let mut terminator = terminator_after(tail, ending);
        if position + 1 == contents.len() && terminator == "\r" && next_first.starts_with('\n') {
            terminator = "\r\n";
        }
        let line_text = format!("{content}{terminator}");
        text.push_str(&line_text);
        new.push(Line {
            container,
            kind: SyntaxKind::ENTRY,
            text: line_text,
            terminated: true,
        });
    }
    text.push_str(&join(&after));
    let new_lines = (before.len()..before.len() + new.len()).collect();
    let mut expected = before;
    expected.extend(new);
    expected.extend(after);
    Reference {
        text,
        expected,
        new_lines,
        loose: false,
    }
}

/// The lines a reader finds in the reference's text, if they are the expected ones.
fn matches(reference: &Reference, options: &ParseOptions) -> Option<Vec<Line>> {
    let actual = lines_of(&reference.text, options);
    if actual.len() != reference.expected.len() {
        return None;
    }
    for (index, (actual, expected)) in actual.iter().zip(&reference.expected).enumerate() {
        if actual.container != expected.container || actual.text != expected.text {
            return None;
        }
        if reference.loose && reference.new_lines.contains(&index) {
            if actual.kind == SyntaxKind::SECTION_HEADER || !actual.terminated {
                return None;
            }
        } else if actual.kind != expected.kind || actual.terminated != expected.terminated {
            return None;
        }
    }
    Some(actual)
}

/// The entry of one line read in the middle of a document, where U+FEFF is text.
fn entry_of(text: &str, options: &ParseOptions) -> Option<Entry> {
    let root = parse_with(&format!("\n{text}"), options).syntax();
    let node = root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::ENTRY)?;
    Entry::cast(node)
}

/// Where a hostile string goes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Site {
    AppendKey,
    InsertKeyAtStart,
    SetKey,
    AppendValue,
    SetValue,
    Section,
    AppendLine,
    InsertLineAtStart,
    AppendLineThenEntry,
    AppendEntryThenLine,
}

pub(crate) const SITES: [Site; 10] = [
    Site::AppendKey,
    Site::InsertKeyAtStart,
    Site::SetKey,
    Site::AppendValue,
    Site::SetValue,
    Site::Section,
    Site::AppendLine,
    Site::InsertLineAtStart,
    Site::AppendLineThenEntry,
    Site::AppendEntryThenLine,
];

/// The last container and the flattened index of its content end.
fn last_container_end(lines: &[Line]) -> (usize, usize) {
    let container = lines.last().map_or(0, |line| line.container);
    let end = lines
        .iter()
        .rposition(|line| line.container == container && line.kind != SyntaxKind::BLANK_LINE)
        .map_or(0, |index| index + 1);
    (end, container)
}

fn gaps(spacing: &SeparatorSpacing) -> (&str, &str) {
    match spacing {
        SeparatorSpacing::Preserve => (" ", " "),
        SeparatorSpacing::Compact => ("", ""),
        SeparatorSpacing::Exact { before, after } => (before.as_str(), after.as_str()),
    }
}

/// One case of the product: `text` written at `site` of `document`, read with `options`, with
/// new entries written with `spacing`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Case<'case> {
    pub(crate) document: &'case str,
    pub(crate) options: &'case ParseOptions,
    pub(crate) spacing: &'case SeparatorSpacing,
    pub(crate) site: Site,
    pub(crate) text: &'case str,
}

/// Whether the edit of `case` is representable, and the reference's text; `None` when the site
/// does not apply to the document.
pub(crate) fn reference(case: Case<'_>) -> Option<(bool, String)> {
    let Case {
        document,
        options,
        spacing,
        site,
        text,
    } = case;
    let lines = lines_of(document, options);
    let (before_gap, after_gap) = gaps(spacing);
    match site {
        Site::AppendKey | Site::AppendValue => {
            let (key, value) = if matches!(site, Site::AppendKey) {
                (text, "v")
            } else {
                ("n", text)
            };
            let content = format!("{key}{before_gap}={after_gap}{value}");
            let reference =
                reference_insert(document, options, last_container_end(&lines), &[content]);
            let representable = matches(&reference, options).is_some_and(|actual| {
                let entry = entry_of(&actual[reference.new_lines[0]].text, options);
                let entry = entry.as_ref();
                entry.and_then(Entry::key).as_deref() == Some(key)
                    && entry.and_then(Entry::value).as_deref() == Some(value)
            }) && !last_line_continues(value);
            Some((representable, reference.text))
        }
        Site::InsertKeyAtStart => {
            let content = format!("{text}{before_gap}={after_gap}v");
            let reference = reference_insert(document, options, (0, 0), &[content]);
            // The new line is the document's first: read it in place.
            let representable = matches(&reference, options).is_some_and(|_| {
                let root = parse_with(&reference.text, options).syntax();
                let first = root.children().next().and_then(Entry::cast);
                first.and_then(|entry| entry.key()).as_deref() == Some(text)
            });
            Some((representable, reference.text))
        }
        Site::AppendLine
        | Site::AppendLineThenEntry
        | Site::AppendEntryThenLine
        | Site::InsertLineAtStart => Some(reference_lines(case, &lines)),
        Site::Section => Some(reference_section(document, options, &lines, text)),
        Site::SetKey | Site::SetValue => reference_rewrite(case, &lines),
    }
}

fn reference_lines(case: Case<'_>, lines: &[Line]) -> (bool, String) {
    let Case {
        document,
        options,
        site,
        text,
        ..
    } = case;
    let contents: Vec<String> = match site {
        Site::AppendLineThenEntry => vec![text.into(), "z=9".into()],
        Site::AppendEntryThenLine => vec!["z=9".into(), text.into()],
        _ => vec![text.into()],
    };
    let at = if matches!(site, Site::InsertLineAtStart) {
        (0, 0)
    } else {
        last_container_end(lines)
    };
    let mut reference = reference_insert(document, options, at, &contents);
    reference.loose = true;
    let representable = matches(&reference, options).is_some();
    (representable, reference.text)
}

fn reference_section(
    document: &str,
    options: &ParseOptions,
    lines: &[Line],
    name: &str,
) -> (bool, String) {
    // The documented lookup comes first: the editor returns a section that already has the
    // name, and writes nothing.
    let root = parse_with(document, options).syntax();
    if root
        .children()
        .filter_map(Section::cast)
        .any(|section| section.name().as_deref() == Some(name))
    {
        return (true, document.to_owned());
    }
    let ending = ending_of(document);
    let mut before = lines.to_vec();
    if let Some(last) = before.last_mut().filter(|line| !line.terminated) {
        let terminator = terminator_after(&last.text, ending);
        last.text.push_str(terminator);
        last.terminated = true;
    }
    let mut text = format!("{}{}", byte_order_mark_of(document), join(&before));
    let last_container = before.last().map_or(0, |line| line.container);
    let new_container = before.iter().map(|line| line.container).max().unwrap_or(0) + 1;
    let mut expected = before.clone();
    let mut new_lines = Vec::new();
    if before
        .last()
        .is_some_and(|line| line.kind != SyntaxKind::BLANK_LINE)
    {
        let terminator = terminator_after(&text, ending);
        text.push_str(terminator);
        new_lines.push(expected.len());
        expected.push(Line {
            container: last_container,
            kind: SyntaxKind::BLANK_LINE,
            text: terminator.into(),
            terminated: true,
        });
    }
    let header = format!("[{name}]{ending}");
    text.push_str(&header);
    new_lines.push(expected.len());
    expected.push(Line {
        container: new_container,
        kind: SyntaxKind::SECTION_HEADER,
        text: header,
        terminated: true,
    });
    let reference = Reference {
        text: text.clone(),
        expected,
        new_lines,
        loose: false,
    };
    let representable = matches(&reference, options).is_some_and(|_| {
        let root = parse_with(&text, options).syntax();
        let header = root.descendants().filter_map(SectionHeader::cast).last();
        header.and_then(|header| header.name()).as_deref() == Some(name)
    });
    (representable, text)
}

/// The lines that replace entry line `line` when the site of `case` writes its text into it: the
/// line with its key replaced, or the moved comment line, if any, and the line with its value
/// assigned.
fn rewritten_texts(case: Case<'_>, line: &Line) -> Vec<String> {
    let Case {
        document,
        options,
        spacing,
        site,
        text,
    } = case;
    let root = parse_with(&format!("\n{}", line.text), options).syntax();
    let entry_node = root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::ENTRY)
        .expect("an entry line");
    if !matches!(site, Site::SetKey) {
        return rewrite(&entry_node, text, spacing, ending_of(document));
    }
    let key = entry_node
        .children()
        .find(|node| node.kind() == SyntaxKind::KEY)
        .expect("an entry has a key");
    let start =
        usize::from(key.text_range().start()) - usize::from(entry_node.text_range().start());
    let end = start + usize::from(key.text_range().len());
    vec![format!(
        "{}{text}{}",
        &line.text[..start],
        &line.text[end..]
    )]
}

/// Whether the rewritten entry line `actual` reads back what `site` wrote: the new key with the
/// old value and comment, or the old key with the new value.
fn rewrite_reads_back(actual: &Entry, old: &Entry, site: Site, text: &str) -> bool {
    if matches!(site, Site::SetKey) {
        actual.key().as_deref() == Some(text)
            && actual.value() == old.value()
            && actual.inline_comment() == old.inline_comment()
    } else {
        actual.key() == old.key() && actual.value().as_deref() == Some(text)
    }
}

fn reference_rewrite(case: Case<'_>, lines: &[Line]) -> Option<(bool, String)> {
    let Case {
        document,
        options,
        site,
        text,
        ..
    } = case;
    let index = lines
        .iter()
        .position(|line| line.kind == SyntaxKind::ENTRY)?;
    let line = &lines[index];
    let old = entry_of(&line.text, options).expect("an entry line");
    let new_texts = rewritten_texts(case, line);
    let mut rendered = format!("{}{}", byte_order_mark_of(document), join(&lines[..index]));
    rendered.extend(new_texts.iter().map(String::as_str));
    rendered.push_str(&join(&lines[index + 1..]));
    let mut expected: Vec<Line> = lines[..index].to_vec();
    let mut new_lines = Vec::new();
    for (position, new_text) in new_texts.iter().enumerate() {
        let entry_line = position + 1 == new_texts.len();
        new_lines.push(expected.len());
        expected.push(Line {
            container: line.container,
            kind: if entry_line {
                SyntaxKind::ENTRY
            } else {
                SyntaxKind::COMMENT_LINE
            },
            text: new_text.clone(),
            terminated: !entry_line || line.terminated,
        });
    }
    expected.extend(lines[index + 1..].iter().cloned());
    let reference = Reference {
        text: rendered.clone(),
        expected,
        new_lines,
        loose: false,
    };
    let representable = matches(&reference, options).is_some_and(|actual| {
        let last = *reference.new_lines.last().expect("the entry line");
        let entry = entry_of(&actual[last].text, options).expect("an entry line");
        rewrite_reads_back(&entry, &old, site, text)
    }) && !(matches!(site, Site::SetValue) && last_line_continues(text));
    Some((representable, rendered))
}

/// The parts of an entry line: indent, key, gap, separator, trailing spacing, comment (with the
/// spacing before it) and terminator.
struct Parts {
    indent: String,
    key: String,
    gap: String,
    separator: Option<char>,
    trailing: String,
    comment: Option<String>,
    terminator: Option<String>,
}

fn parts(entry: &SyntaxNode) -> Parts {
    let mut parts = Parts {
        indent: String::new(),
        key: String::new(),
        gap: String::new(),
        separator: None,
        trailing: String::new(),
        comment: None,
        terminator: None,
    };
    let (mut seen_key, mut seen_value) = (false, false);
    for element in entry.children_with_tokens() {
        let text = element.to_string();
        match element.kind() {
            SyntaxKind::KEY => {
                seen_key = true;
                parts.key = text;
            }
            SyntaxKind::VALUE => seen_value = true,
            SyntaxKind::NEWLINE => parts.terminator = Some(text),
            SyntaxKind::COMMENT => {
                parts.comment = Some(format!("{}{text}", std::mem::take(&mut parts.trailing)));
            }
            SyntaxKind::EQ | SyntaxKind::COLON => {
                parts.separator = Some(if element.kind() == SyntaxKind::EQ {
                    '='
                } else {
                    ':'
                });
                parts.gap.push_str(&text);
            }
            _ if !seen_key => parts.indent.push_str(&text),
            _ if !seen_value => parts.gap.push_str(&text),
            _ => parts.trailing.push_str(&text),
        }
    }
    parts
}

/// The documented rendering of `set_value`, stated independently.
fn rewrite(
    entry: &SyntaxNode,
    value: &str,
    spacing: &SeparatorSpacing,
    ending: &'static str,
) -> Vec<String> {
    let parts = parts(entry);
    let preserve = matches!(spacing, SeparatorSpacing::Preserve);
    let gap = match parts.separator {
        Some(_) if preserve => parts.gap.clone(),
        separator => {
            let (before, after) = gaps(spacing);
            format!("{before}{}{after}", separator.unwrap_or('='))
        }
    };
    let last = &value[value.rfind(['\r', '\n']).map_or(0, |start| start + 1)..];
    let mut content = format!("{}{}{gap}{value}", parts.indent, parts.key);
    let mut moved = None;
    match parts.comment {
        Some(comment) if !last.is_empty() => content.push_str(&comment),
        Some(comment) => moved = Some(comment),
        None if value.is_empty() && !preserve => {}
        None => content.push_str(&parts.trailing),
    }
    let terminator = parts.terminator.map(|terminator| {
        if content.ends_with('\r') && terminator == "\n" {
            "\r\n".to_owned()
        } else {
            terminator
        }
    });
    let mut texts = Vec::new();
    if let Some(comment) = moved {
        texts.push(format!(
            "{comment}{}",
            terminator.as_deref().unwrap_or(ending)
        ));
    }
    texts.push(format!("{content}{}", terminator.unwrap_or_default()));
    texts
}

/// The container that holds the first entry in document order.
fn first_entry_container(editor: &mut Editor) -> Option<SectionEditor<'_>> {
    if editor.preamble().entry_count() > 0 {
        return Some(editor.preamble());
    }
    let index = (0..editor.section_count()).find(|&index| {
        editor
            .section_at(index)
            .is_some_and(|section| section.entry_count() > 0)
    })?;
    editor.section_at(index)
}

/// The last container: the last section, or the preamble.
fn last_container(editor: &mut Editor) -> SectionEditor<'_> {
    match editor.section_count() {
        0 => editor.preamble(),
        count => editor
            .section_at(count - 1)
            .expect("the last section exists"),
    }
}

/// Apply the edit of `site` with `text`.
pub(crate) fn apply(editor: &mut Editor, site: Site, text: &str) -> Result<(), EditError> {
    match site {
        Site::AppendKey => last_container(editor).append_entry(text, "v"),
        Site::AppendValue => last_container(editor).append_entry("n", text),
        Site::InsertKeyAtStart => editor.preamble().insert_entry(0, text, "v"),
        Site::AppendLine => last_container(editor).append_body_lines(&[text]),
        Site::AppendLineThenEntry => last_container(editor).append_body_lines(&[text, "z=9"]),
        Site::AppendEntryThenLine => last_container(editor).append_body_lines(&["z=9", text]),
        Site::InsertLineAtStart => editor.preamble().insert_body_lines(0, &[text]),
        Site::Section => editor.section_or_insert(text).map(drop),
        Site::SetKey | Site::SetValue => {
            let mut container = first_entry_container(editor).expect("the site applies");
            let mut entry = container.entry_at(0).expect("the container has an entry");
            if matches!(site, Site::SetKey) {
                entry.set_key(text)
            } else {
                entry.set_value(text)
            }
        }
    }
}

/// Every string of up to `count` pieces.
pub(crate) fn strings(pieces: &[&str], count: usize) -> Vec<String> {
    let mut all = BTreeSet::new();
    let mut layer = vec![String::new()];
    all.insert(String::new());
    for _ in 0..count {
        layer = layer
            .iter()
            .flat_map(|prefix| pieces.iter().map(move |piece| format!("{prefix}{piece}")))
            .collect();
        all.extend(layer.iter().cloned());
    }
    all.into_iter().collect()
}

/// The counts of one run: cases, false accepts, false rejects and byte mismatches.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub cases: u64,
    pub false_accepts: u64,
    pub false_rejects: u64,
    pub byte_mismatches: u64,
}

/// Check one string at one site of one document.
fn check_case(case: Case<'_>, counts: &mut Counts) {
    let Some((representable, reference)) = reference(case) else {
        return;
    };
    let edit = EditOptions {
        separator_spacing: case.spacing.clone(),
    };
    let mut editor = Editor::new(case.document, case.options, &edit).expect("a small document");
    let before = editor.finish();
    let tree = editor.file().syntax().green().into_owned();
    let result = apply(&mut editor, case.site, case.text);
    counts.cases += 1;
    match &result {
        Ok(()) if !representable => {
            counts.false_accepts += 1;
            eprintln!("false accept: {case:?} -> {:?}", editor.finish());
        }
        Ok(()) if editor.finish() != reference => {
            counts.byte_mismatches += 1;
            eprintln!(
                "bytes: {case:?}\n  editor    {:?}\n  reference {reference:?}",
                editor.finish()
            );
        }
        Ok(()) => {}
        Err(error) => {
            assert_eq!(editor.finish(), before, "C2: {case:?}");
            assert_eq!(
                editor.file().syntax().green().into_owned(),
                tree,
                "C2: {case:?}"
            );
            if representable {
                counts.false_rejects += 1;
                eprintln!("false reject: {case:?} -> {error:?}, reference {reference:?}");
            }
        }
    }
    let reparse = parse_with(&editor.finish(), case.options);
    assert_eq!(
        editor.file().syntax().green().into_owned(),
        *reparse.green(),
        "C1: {case:?}"
    );
}

/// The inputs of a run: every string of up to `count` of `pieces`, at every site of every
/// document, under every option set and spacing.
#[derive(Debug)]
pub(crate) struct Product<'product> {
    pub(crate) count: usize,
    pub(crate) pieces: &'product [&'product str],
    pub(crate) documents: &'product [&'product str],
    pub(crate) options: &'product [ParseOptions],
    pub(crate) spacings: &'product [SeparatorSpacing],
}

/// Check every case of `product`.
pub(crate) fn run(product: &Product<'_>) -> Counts {
    let strings = strings(product.pieces, product.count);
    let mut counts = Counts::default();
    for options in product.options {
        for spacing in product.spacings {
            for document in product.documents {
                for site in SITES {
                    for text in &strings {
                        let case = Case {
                            document,
                            options,
                            spacing,
                            site,
                            text,
                        };
                        check_case(case, &mut counts);
                    }
                }
            }
        }
    }
    counts
}

/// Separates the strings of a fuzz input. UTF-8 never uses this byte, so a string never holds it.
const FIELD_SEPARATOR: u8 = 0xFF;

/// The oracle of the `inputs` fuzz target: one arbitrary string at every site of a document.
///
/// The first byte selects the parser options (bit 0 `allow_no_value`, bit 1 `inline_comments`)
/// and the spacing (the byte shifted right by two, modulo 3: `Preserve`, `Compact` or `Exact`),
/// and the second byte, modulo 14, one of the [`DOCUMENTS`]. The other bytes are up to three
/// strings separated by `0xFF`: the text written at every site, then the `Exact` whitespace
/// before and after the separator. Missing bytes are zero and missing strings empty, invalid
/// UTF-8 decodes lossily, and later strings are ignored, so every byte string is a case and
/// committed inputs stay valid.
///
/// One document per input, not all 14: the fuzzer checks as many cases per second either way,
/// but it steers each string to the documents where it finds new behavior, and each committed
/// input replays in a fourteenth of the time.
pub(crate) fn check(data: &[u8]) {
    let selector = data.first().copied().unwrap_or(0);
    let document = DOCUMENTS[usize::from(data.get(1).copied().unwrap_or(0)) % DOCUMENTS.len()];
    let mut fields = data
        .get(2..)
        .unwrap_or_default()
        .split(|&byte| byte == FIELD_SEPARATOR)
        .map(String::from_utf8_lossy);
    let text = fields.next().unwrap_or_default();
    let before = fields.next().unwrap_or_default();
    let after = fields.next().unwrap_or_default();
    let options = ParseOptions {
        allow_no_value: selector & 1 != 0,
        inline_comments: selector & 2 != 0,
    };
    let spacing = match (selector >> 2) % 3 {
        0 => SeparatorSpacing::Preserve,
        1 => SeparatorSpacing::Compact,
        _ => match exact_spacing(&before, &after) {
            Some(spacing) => spacing,
            None => return,
        },
    };
    let mut counts = Counts::default();
    for site in SITES {
        let case = Case {
            document,
            options: &options,
            spacing: &spacing,
            site,
            text: &text,
        };
        check_case(case, &mut counts);
    }
    assert_eq!(
        (
            counts.false_accepts,
            counts.false_rejects,
            counts.byte_mismatches
        ),
        (0, 0, 0),
        "{counts:?}"
    );
}

/// `Exact` spacing from two arbitrary strings, or `None` when the editor refuses them. It must
/// refuse exactly when one of them is more than spaces and tabs, a rule stated here
/// independently, and name the first such string.
fn exact_spacing(before: &str, after: &str) -> Option<SeparatorSpacing> {
    let is_spacing = |text: &str| {
        text.chars()
            .all(|character| matches!(character, ' ' | '\t'))
    };
    let refused = [before, after].into_iter().find(|text| !is_spacing(text));
    match (SeparatorSpacing::exact(before, after), refused) {
        (Ok(spacing), None) => {
            assert_eq!(gaps(&spacing), (before, after));
            Some(spacing)
        }
        (Err(EditError::InvalidSpacing { text }), Some(refused)) if text == refused => None,
        (result, refused) => {
            panic!("exact({before:?}, {after:?}) is {result:?}; expected to refuse {refused:?}")
        }
    }
}

/// The hostile pieces.
pub(crate) const PIECES: [&str; 16] = [
    "", " ", "\t", "\n", "\r", "\r\n", "=", ":", "[", "]", ";", "#", "\\", "\u{FEFF}", "a", "b",
];

/// Documents for the full products: every line ending, byte order marks, a preamble, a
/// continuing and an unterminated last line, inline comments, and empty documents.
pub(crate) const DOCUMENTS: [&str; 14] = [
    "[s]\na=1\nb=2\n",
    "[s]\r\na=1\r\nb=2\r\n",
    "[s]\ra=1\rb=2\r",
    "\u{FEFF}[s]\na=1\n",
    "a=1\n[s]\nb=2\n",
    "[s]\na=1",
    "[s]\na=C:\\x\\",
    "[s]\na=1 ; c\nb=2 ; d\n",
    "",
    "\u{FEFF}",
    "[s]\ra=1\n\n",
    "[s]\nk=v\r",
    "[s]\na=one \\\r",
    "g=1 ; c\n",
];
