//! Handles to one entry, and the renderings that rewrite an entry's line.

use super::Editor;
use super::error::EditError;
use super::fragment::{self, CRLF, LINE_BREAKS};
use super::options::SeparatorSpacing;
use super::plan::{self, Plan, Step};
use crate::ast::{AstNode, Entry};
use crate::syntax_kind::{SyntaxKind, SyntaxNode};

/// An exclusive handle to one entry of the preamble or of a section.
///
/// It addresses its entry by identity, so it is unambiguous among duplicate keys, and it keeps
/// addressing the entry across its own [`set_key`](Self::set_key) and
/// [`set_value`](Self::set_value). While it exists, nothing else can change the document.
#[derive(Debug)]
pub struct EntryEditor<'s> {
    editor: &'s mut Editor,
    /// The `ENTRY` node, in the editor's tree.
    node: SyntaxNode,
}

impl<'s> EntryEditor<'s> {
    /// A handle to `node`, an `ENTRY` in `editor`'s tree.
    pub(super) fn new(editor: &'s mut Editor, node: SyntaxNode) -> EntryEditor<'s> {
        assert_eq!(node.kind(), SyntaxKind::ENTRY);
        assert!(
            node.ancestors()
                .last()
                .is_some_and(|root| root == editor.root)
        );
        EntryEditor { editor, node }
    }
}

impl EntryEditor<'_> {
    /// The whole editor, to read the document while this handle is held.
    #[must_use]
    pub fn editor(&self) -> &Editor {
        self.editor
    }

    /// The entry's position among its section's (or the preamble's) entries, computed from the
    /// tree on each call. O(lines).
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn index(&self) -> usize {
        let container = plan::container_of(&self.node);
        let index = self
            .editor
            .entries(&container)
            .position(|entry| entry == self.node)
            .expect("an entry handle's entry is in its container");
        // Entries are body lines, so an entry's index never exceeds its body line.
        #[cfg(all(debug_assertions, not(miri)))]
        assert!(index <= self.body_line());
        index
    }

    /// The entry's body-line position in its section or the preamble, computed from the tree on
    /// each call. O(lines).
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn body_line(&self) -> usize {
        let container = plan::container_of(&self.node);
        let body_line = self
            .editor
            .body_lines(&container)
            .position(|line| line == self.node);
        body_line.expect("an entry handle's entry is in its container")
    }

    /// The key.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn key(&self) -> String {
        let key = plan::key_token(&self.node).text().to_owned();
        // The lexer ends a key at a separator, spacing or a line break, after one character.
        assert_ne!(key, "");
        assert!(!key.contains(['=', ':', ' ', '\t', '\r', '\n']));
        key
    }

    /// The value, or `None` for a bare key without a separator.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn value(&self) -> Option<String> {
        let value = self.typed().value();
        if let Some(value) = &value {
            // Spacing around a value belongs to the separator gap or to the trailing spacing.
            assert!(!value.starts_with([' ', '\t']));
            assert!(!value.ends_with([' ', '\t']));
        }
        value
    }

    /// The inline comment with its marker (`; note`), as
    /// [`Entry::inline_comment`](crate::ast::Entry::inline_comment).
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn inline_comment(&self) -> Option<String> {
        let comment = self.typed().inline_comment();
        if let Some(comment) = &comment {
            assert!(comment.starts_with([';', '#']));
        }
        comment
    }

    /// Replace the key; the rest of the line stays byte for byte. There is no duplicate check: a
    /// handle edits by position, so callers can rename through transient duplicates.
    ///
    /// # Errors
    ///
    /// In this order: [`EditError::DocumentTooLarge`]; [`EditError::InvalidKey`] unless the line
    /// reads back with exactly `key`; [`EditError::ByteOrderMarkAtStart`] if the entry is the
    /// first line of a document without a byte order mark and `key` starts with U+FEFF.
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn set_key(&mut self, key: &str) -> Result<(), EditError> {
        let (plan, node) = plan_set_key(self.editor, &self.node, key)?;
        self.editor.commit(plan);
        // The handle now holds the entry's new node, which the commit attached.
        assert!(node.parent().is_some());
        self.node = node;
        assert_eq!(self.key(), key);
        Ok(())
    }

    /// Assign the value. The key, the indentation and the separator character stay; the
    /// separator spacing follows the editor's [`SeparatorSpacing`].
    ///
    /// - With [`Preserve`](SeparatorSpacing::Preserve) and a separator, the whole gap between key
    ///   and value stays. A bare key, or a `key junk` line, gets a new gap; the junk is replaced.
    /// - Spaces and tabs after the old value stay, except when the new value is empty under
    ///   `Compact` or `Exact`, where they would widen the gap the policy sets. Under `Preserve`
    ///   they join the gap.
    /// - An inline comment stays after the value when the value's last line is not empty.
    ///   Otherwise a reader would take its marker as value text, so the comment moves to a line
    ///   of its own directly above the entry, keeping every byte.
    /// - The line keeps its terminator; an LF terminator after a value ending in CR becomes CRLF,
    ///   so the two stay separate line breaks. An unterminated last line stays unterminated.
    ///
    /// # Errors
    ///
    /// In this order: [`EditError::DocumentTooLarge`]; [`EditError::InvalidValue`] if the value's
    /// last line ends in a backslash, or if the line would not read back with exactly `value`
    /// (see the [module documentation](super#what-the-editor-refuses)).
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn set_value(&mut self, value: &str) -> Result<(), EditError> {
        let (plan, node) = plan_set_value(self.editor, &self.node, value)?;
        self.editor.commit(plan);
        // The handle now holds the entry's new node, which the commit attached.
        assert!(node.parent().is_some());
        self.node = node;
        assert_eq!(self.value().as_deref(), Some(value));
        Ok(())
    }

    /// Remove the entry's line. On `Err` the document is unchanged and the handle is gone;
    /// acquire a new one to retry.
    ///
    /// # Errors
    ///
    /// [`EditError::ByteOrderMarkAtStart`] if the entry is the first line of a document without a
    /// byte order mark and the next line starts with U+FEFF.
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn remove(self) -> Result<(), EditError> {
        let container = plan::container_of(&self.node);
        let position = self.body_line();
        let plan = self
            .editor
            .plan_remove(&container, position..position + 1)?;
        self.editor.commit(plan);
        // The entry's own node is the one the commit detached.
        assert!(self.node.parent().is_none());
        Ok(())
    }

    /// The typed view of the entry.
    fn typed(&self) -> Entry {
        // A handle's entry stays in the tree while the handle lives.
        assert!(self.node.parent().is_some());
        Entry::cast(self.node.clone()).expect("an entry handle holds an entry")
    }
}

/// The parts of an entry line in the order the parser groups them:
/// `[indent] KEY gap VALUE [trailing] [comment] [terminator]`.
#[derive(Default)]
struct EntryParts {
    /// Spaces and tabs before the key.
    indent: String,
    /// The key.
    key: String,
    /// Everything between the key and the value: `[spacing] (= or :) [spacing]`, or for a bare
    /// key `[spacing] [junk]`.
    gap: String,
    /// `EQ` or `COLON`, or `None` for a bare key.
    separator: Option<SyntaxKind>,
    /// Spaces and tabs after the value, when no inline comment follows.
    trailing: String,
    /// The inline comment with the spaces and tabs before it.
    comment: Option<String>,
    /// The line's terminator, or `None` for the document's unterminated last line.
    terminator: Option<String>,
}

/// Where in an entry line a token stands.
#[derive(Clone, Copy)]
enum Region {
    BeforeKey,
    BeforeValue,
    AfterValue,
}

/// Split an `ENTRY` node into its parts.
fn entry_parts(entry: &SyntaxNode) -> EntryParts {
    let mut parts = EntryParts::default();
    let mut region = Region::BeforeKey;
    let mut after_value = String::new();
    for element in entry.children_with_tokens() {
        match element.kind() {
            SyntaxKind::KEY => {
                parts.key = element.to_string();
                region = Region::BeforeValue;
            }
            SyntaxKind::VALUE => region = Region::AfterValue,
            SyntaxKind::NEWLINE => parts.terminator = Some(element.to_string()),
            // The spaces and tabs before an inline comment belong to it.
            SyntaxKind::COMMENT => {
                parts.comment = Some(std::mem::take(&mut after_value) + &element.to_string());
            }
            SyntaxKind::EQ | SyntaxKind::COLON => {
                parts.separator = Some(element.kind());
                parts.gap.push_str(&element.to_string());
            }
            _ => match region {
                Region::BeforeKey => parts.indent.push_str(&element.to_string()),
                Region::BeforeValue => parts.gap.push_str(&element.to_string()),
                Region::AfterValue => after_value.push_str(&element.to_string()),
            },
        }
    }
    parts.trailing = after_value;
    // The parts, with the value between the gap and the trailing spacing, are the whole line.
    let value = entry
        .children()
        .find(|node| node.kind() == SyntaxKind::VALUE)
        .map(|value| value.to_string())
        .unwrap_or_default();
    if parts.separator.is_none() {
        assert_eq!(value, "");
    }
    // Every entry has a key, and its comment and terminator are what the lexer made of them.
    assert_ne!(parts.key, "");
    if let Some(comment) = &parts.comment {
        assert!(
            comment
                .trim_start_matches([' ', '\t'])
                .starts_with([';', '#'])
        );
    }
    if let Some(terminator) = &parts.terminator {
        assert!(LINE_BREAKS.contains(&terminator.as_str()));
    }
    let line = [
        parts.indent.as_str(),
        &parts.key,
        &parts.gap,
        &value,
        &parts.trailing,
        parts.comment.as_deref().unwrap_or(""),
        parts.terminator.as_deref().unwrap_or(""),
    ];
    assert_eq!(line.concat(), entry.to_string());
    parts
}

/// The pieces of the gap between key and value when the value is assigned under `spacing`.
fn assigned_gap<'a>(parts: &'a EntryParts, spacing: &'a SeparatorSpacing) -> Vec<&'a str> {
    let gap = match (spacing, parts.separator) {
        (SeparatorSpacing::Preserve, Some(_)) => vec![parts.gap.as_str()],
        (_, separator) => {
            let (before, after) = spacing.new_gap();
            let separator = if separator == Some(SyntaxKind::COLON) {
                ":"
            } else {
                "="
            };
            vec![before, separator, after]
        }
    };
    // Exactly one separator, with nothing but spacing around it.
    assert_eq!(gap.concat().matches(['=', ':']).count(), 1);
    gap
}

/// The lines that replace an entry's line when its value is assigned: the moved comment line, if
/// any, and the entry line, each as pieces with their terminators.
struct Rewrite<'a> {
    /// The comment line above the entry, with its terminator.
    moved: Option<[&'a str; 2]>,
    /// The entry line, its terminator included.
    entry: Vec<&'a str>,
}

/// Render the assignment of `value` to the entry with `parts`, by the rules of
/// [`EntryEditor::set_value`].
fn rewrite<'a>(
    parts: &'a EntryParts,
    value: &'a str,
    spacing: &'a SeparatorSpacing,
    ending: &'static str,
) -> Rewrite<'a> {
    assert!(LINE_BREAKS.contains(&ending));
    // An inline comment can follow only a value whose last line has text.
    let keep = !fragment::final_line(value).is_empty();
    let (tail, moved) = match parts.comment.as_deref() {
        Some(comment) if keep => (comment, None),
        Some(comment) => ("", Some(comment)),
        // The old trailing spacing would widen the gap that the policy sets.
        None if value.is_empty() && !matches!(spacing, SeparatorSpacing::Preserve) => ("", None),
        None => (parts.trailing.as_str(), None),
    };
    if moved.is_some() {
        // A moved comment leaves nothing after the value.
        assert_eq!(tail, "");
    }
    if keep {
        assert!(moved.is_none());
    }
    let mut entry = vec![parts.indent.as_str(), parts.key.as_str()];
    entry.extend(assigned_gap(parts, spacing));
    entry.extend([value, tail]);
    // T1: a CR at the end of the content would read together with an LF terminator.
    let old_terminator = parts.terminator.as_deref().map(fragment::line_break_at);
    let terminator = old_terminator.map(|terminator| {
        let content_end = plan::content_end(&entry);
        if fragment::joins_line_breaks(content_end, terminator.chars().next()) {
            CRLF
        } else {
            terminator
        }
    });
    entry.push(terminator.unwrap_or(""));
    if let Some(terminator) = terminator {
        // T1, paired: the terminator stays a line break of its own, apart from the content, which
        // is every piece but the terminator.
        assert!(!fragment::joins_line_breaks(
            plan::content_end(&entry[..entry.len() - 1]),
            terminator.chars().next()
        ));
    }
    Rewrite {
        moved: moved.map(|comment| [comment, terminator.unwrap_or(ending)]),
        entry,
    }
}

/// Plan assigning `value` to `entry`, and return its new node with the plan.
fn plan_set_value(
    editor: &Editor,
    entry: &SyntaxNode,
    value: &str,
) -> Result<(Plan, SyntaxNode), EditError> {
    assert_eq!(entry.kind(), SyntaxKind::ENTRY);
    assert!(entry.parent().is_some());
    let parts = entry_parts(entry);
    let spacing = &editor.edit_options.separator_spacing;
    let rewrite = rewrite(&parts, value, spacing, editor.line_ending());
    let len_moved = rewrite
        .moved
        .map_or(0, |moved| plan::rendered_len(&[], &moved));
    let added = plan::rendered_len(&[], &rewrite.entry).saturating_add(len_moved);
    let len_after = editor.check_len(usize::from(entry.text_range().len()), added)?;
    // Policy: the editor never writes a value whose last line ends in a backslash, even where a
    // reader would read it back, so that no later edit can turn it into a continuation.
    if fragment::line_continues(fragment::final_line(value)) {
        return Err(EditError::InvalidValue {
            value: value.to_owned(),
        });
    }
    let mut text = rewrite
        .moved
        .map(|moved| moved.concat())
        .unwrap_or_default();
    text.push_str(&rewrite.entry.concat());
    // The rendering is exactly as long as the size check predicted.
    assert_eq!(text.len(), added);
    let nodes = editor.fragment_lines(&text);
    let read_back = nodes
        .last()
        .cloned()
        .and_then(Entry::cast)
        .and_then(|entry| entry.value());
    if read_back.as_deref() != Some(value) {
        return Err(EditError::InvalidValue {
            value: value.to_owned(),
        });
    }
    let new_entry = rewritten_entry(&parts, &nodes, rewrite.moved.is_some());
    let plan = Plan {
        steps: vec![Step::Replace {
            old: entry.clone().into(),
            new: nodes.into_iter().map(Into::into).collect(),
        }],
        len_after,
    };
    Ok((plan, new_entry))
}

/// The rewritten entry, the last of `nodes`, after asserting what a value that reads back implies
/// about the rewritten lines.
fn rewritten_entry(parts: &EntryParts, nodes: &[SyntaxNode], moved: bool) -> SyntaxNode {
    assert_eq!(nodes.len(), 1 + usize::from(moved));
    let entry = nodes
        .last()
        .cloned()
        .and_then(Entry::cast)
        .expect("a value that reads back is an entry's");
    assert_eq!(entry.key().as_deref(), Some(parts.key.as_str()));
    assert_eq!(
        plan::ends_with_newline(entry.syntax()),
        parts.terminator.is_some()
    );
    if moved {
        assert_eq!(nodes[0].kind(), SyntaxKind::COMMENT_LINE);
        assert!(entry.inline_comment().is_none());
    } else {
        let comment = parts
            .comment
            .as_deref()
            .map(|comment| comment.trim_start_matches([' ', '\t']));
        assert_eq!(entry.inline_comment().as_deref(), comment);
    }
    entry.syntax().clone()
}

/// Plan replacing the key of `entry` with `key`, and return its new node with the plan.
pub(super) fn plan_set_key(
    editor: &Editor,
    entry: &SyntaxNode,
    key: &str,
) -> Result<(Plan, SyntaxNode), EditError> {
    let line = entry.to_string();
    let old_key = plan::key_token(entry);
    let start = usize::from(old_key.text_range().start() - entry.text_range().start());
    let end = start + old_key.text().len();
    let pieces = [&line[..start], key, &line[end..]];
    let len_after = editor.check_len(line.len(), plan::rendered_len(&[], &pieces))?;
    let text = pieces.concat();
    // The rendering is exactly as long as the size check predicted.
    assert_eq!(editor.len() - line.len() + text.len(), len_after);
    // A key ends where the lexer ends a key, so the rest of the line reads as before. An empty
    // rendering (the unterminated bare key `flag` renamed to "") parses to no line at all.
    let mut nodes = editor.fragment_lines(&text);
    let read_back = nodes
        .first()
        .cloned()
        .and_then(Entry::cast)
        .and_then(|entry| entry.key());
    if read_back.as_deref() != Some(key) {
        return Err(EditError::InvalidKey {
            key: key.to_owned(),
        });
    }
    // Only the preamble's first line has no line before it.
    editor.check_document_start(entry.prev_sibling().as_ref(), text.chars().next())?;
    assert_eq!(nodes.len(), 1);
    let new_entry = nodes.pop().expect("one line");
    let (before, after) = (
        Entry::cast(entry.clone()).expect("an entry"),
        Entry::cast(new_entry.clone()).expect("an entry"),
    );
    assert_eq!(after.value(), before.value());
    assert_eq!(after.inline_comment(), before.inline_comment());
    assert_eq!(
        plan::ends_with_newline(&new_entry),
        plan::ends_with_newline(entry)
    );
    let plan = Plan {
        steps: vec![Step::Replace {
            old: entry.clone().into(),
            new: vec![new_entry.clone().into()],
        }],
        len_after,
    };
    Ok((plan, new_entry))
}
