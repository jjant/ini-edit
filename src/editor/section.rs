//! Handles to the preamble or one section.

use std::ops::Range;

use super::Editor;
use super::entry::{self, EntryEditor};
use super::error::EditError;
use super::plan::{self, Container, NewLines, Slot, Step};
use crate::ast::{AstNode, Section};
use crate::syntax_kind::{SyntaxKind, SyntaxNode};

/// An exclusive handle to the preamble or one section.
///
/// While it exists, nothing else can change the editor, so it never observes a stale or removed
/// line; read the whole document through [`editor`](Self::editor). Positions count *body lines*:
/// the lines after the header, or the lines of the preamble. Position `p` is in
/// `0..=body_line_count()` and stands directly before body line `p`.
#[derive(Debug)]
pub struct SectionEditor<'e> {
    editor: &'e mut Editor,
    container: Container,
}

impl<'e> SectionEditor<'e> {
    /// A handle to `container` of `editor`.
    pub(super) fn new(editor: &'e mut Editor, container: Container) -> SectionEditor<'e> {
        if let Container::Section(section) = &container {
            assert_eq!(section.kind(), SyntaxKind::SECTION);
            assert!(section.parent().is_some_and(|root| root == editor.root));
        }
        SectionEditor { editor, container }
    }
}

impl SectionEditor<'_> {
    /// The whole editor, to read the document while this handle is held.
    #[must_use]
    pub fn editor(&self) -> &Editor {
        self.editor
    }

    /// Whether this handle addresses the preamble.
    #[must_use]
    pub fn is_preamble(&self) -> bool {
        matches!(self.container, Container::Preamble)
    }

    /// The header's name: `None` for the preamble and for a nameless `[]` header.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn name(&self) -> Option<String> {
        let name = match &self.container {
            Container::Preamble => None,
            Container::Section(section) => Section::cast(section.clone())
                .expect("a section handle holds a section")
                .name(),
        };
        if let Some(name) = &name {
            // The lexer ends a name at `]` or at the end of its line.
            assert!(!name.contains([']', '\r', '\n']));
        }
        name
    }

    /// The number of body lines; the header is not one. O(lines).
    #[must_use]
    pub fn body_line_count(&self) -> usize {
        self.editor.body_lines(&self.container).count()
    }

    /// The number of entries. O(lines).
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn entry_count(&self) -> usize {
        let entry_count = self.editor.entries(&self.container).count();
        // Entries are body lines.
        #[cfg(all(debug_assertions, not(miri)))]
        assert!(entry_count <= self.body_line_count());
        entry_count
    }

    /// The first entry whose key is exactly `key`. Never changes the document. O(lines).
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn entry(&mut self, key: &str) -> Option<EntryEditor<'_>> {
        let node = self.find_entry(key)?;
        // First match (NEG-12): no entry before it has the key.
        #[cfg(all(debug_assertions, not(miri)))]
        assert!(
            self.editor
                .entries(&self.container)
                .take_while(|entry| *entry != node)
                .all(|entry| plan::key_token(&entry).text() != key)
        );
        Some(EntryEditor::new(self.editor, node))
    }

    /// Entry `index` among this container's entries, duplicates included. O(lines).
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn entry_at(&mut self, index: usize) -> Option<EntryEditor<'_>> {
        let node = self.editor.entries(&self.container).nth(index)?;
        let entry = EntryEditor::new(self.editor, node);
        // Two walks agree: entry `index` reports its index as `index`.
        #[cfg(all(debug_assertions, not(miri)))]
        assert_eq!(entry.index(), index);
        Some(entry)
    }

    /// Assign the first entry keyed `key` (as [`EntryEditor::set_value`]), or, if there is none,
    /// [`append_entry`](Self::append_entry)`(key, value)`.
    ///
    /// # Errors
    ///
    /// As the operation chosen: [`EntryEditor::set_value`] or [`append_entry`](Self::append_entry).
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), EditError> {
        match self.entry(key) {
            Some(mut entry) => entry.set_value(value)?,
            None => self.append_entry(key, value)?,
        }
        // Either way, the first entry keyed `key` now has `value`.
        #[cfg(all(debug_assertions, not(miri)))]
        assert_eq!(
            self.entry(key).and_then(|entry| entry.value()).as_deref(),
            Some(value)
        );
        Ok(())
    }

    /// Insert the entry `key{gap}={gap}value` at the content end: directly after the last body
    /// line that is not blank, so trailing blank lines stay below it. The entry is added even if
    /// `key` exists.
    ///
    /// # Errors
    ///
    /// In this order: [`EditError::DocumentTooLarge`]; [`EditError::InvalidKey`];
    /// [`EditError::InvalidValue`]; [`EditError::LastLineContinues`];
    /// [`EditError::ByteOrderMarkAtStart`].
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn append_entry(&mut self, key: &str, value: &str) -> Result<(), EditError> {
        let slot = self.editor.slot_at_content_end(&self.container);
        self.insert_entry_at(slot, key, value)?;
        // The content end follows every entry, so the new entry is the last one.
        #[cfg(all(debug_assertions, not(miri)))]
        {
            let last = self.editor.entries(&self.container).last();
            let entry = last
                .and_then(crate::ast::Entry::cast)
                .expect("an entry was added");
            assert_eq!(entry.key().as_deref(), Some(key));
            assert_eq!(entry.value().as_deref(), Some(value));
        }
        Ok(())
    }

    /// Insert the entry `key{gap}={gap}value` at body position `at`.
    ///
    /// # Errors
    ///
    /// [`EditError::PositionOutOfRange`] if `at > body_line_count()`, then as
    /// [`append_entry`](Self::append_entry).
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn insert_entry(&mut self, at: usize, key: &str, value: &str) -> Result<(), EditError> {
        let slot = self.editor.slot_at(&self.container, at)?;
        self.insert_entry_at(slot, key, value)?;
        // Positions count body lines: the new entry is body line `at`.
        #[cfg(all(debug_assertions, not(miri)))]
        {
            let line = self.editor.body_lines(&self.container).nth(at);
            let entry = line
                .and_then(crate::ast::Entry::cast)
                .expect("the new entry is body line `at`");
            assert_eq!(entry.key().as_deref(), Some(key));
            assert_eq!(entry.value().as_deref(), Some(value));
        }
        Ok(())
    }

    /// Remove the first entry keyed `key`, returning whether there was one.
    ///
    /// # Errors
    ///
    /// [`EditError::ByteOrderMarkAtStart`] if the entry is the first line of a document without a
    /// byte order mark and the next line starts with U+FEFF.
    pub fn remove_entry(&mut self, key: &str) -> Result<bool, EditError> {
        match self.entry(key) {
            Some(entry) => entry.remove().map(|()| true),
            None => Ok(false),
        }
    }

    /// Give the first entry keyed `old_key` the key `new_key`, returning whether there was one.
    /// Renaming a key to itself succeeds without changing a byte.
    ///
    /// # Errors
    ///
    /// In this order, and only if an entry has `old_key`: [`EditError::DuplicateKey`] if another
    /// entry of this container has `new_key`; then as [`EntryEditor::set_key`].
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn rename_key(&mut self, old_key: &str, new_key: &str) -> Result<bool, EditError> {
        let Some(entry) = self.find_entry(old_key) else {
            return Ok(false);
        };
        // No early return for `old_key == new_key`: the key is rewritten with itself.
        if old_key != new_key && self.find_entry(new_key).is_some() {
            return Err(EditError::DuplicateKey {
                key: new_key.to_owned(),
            });
        }
        let (plan, renamed) = entry::plan_set_key(self.editor, &entry, new_key)?;
        self.editor.commit(plan);
        assert!(renamed.parent().is_some());
        // No other entry had `new_key`, so the renamed entry is the first one with it, and the
        // first entry keyed `old_key`, if any, is another one.
        #[cfg(all(debug_assertions, not(miri)))]
        {
            assert_eq!(self.find_entry(new_key).as_ref(), Some(&renamed));
            if old_key != new_key {
                assert_ne!(self.find_entry(old_key).as_ref(), Some(&renamed));
            }
        }
        Ok(true)
    }

    /// Parse each element as exactly one body line, and insert them at the content end, directly
    /// after the last body line that is not blank. An empty slice changes nothing.
    ///
    /// Each element must be one line without its terminator: the editor writes the terminators.
    /// An element that parses as an entry is an entry right away; only an entry's value may
    /// continue onto further lines within its element (`"k = a \\\n  b"`).
    ///
    /// # Errors
    ///
    /// In this order: [`EditError::DocumentTooLarge`]; [`EditError::InvalidLine`] for the first
    /// element that is not exactly one line, is a section header, or ends with a continuation
    /// backslash; [`EditError::LastLineContinues`]; [`EditError::ByteOrderMarkAtStart`].
    pub fn append_body_lines(&mut self, lines: &[&str]) -> Result<(), EditError> {
        let slot = self.editor.slot_at_content_end(&self.container);
        self.insert_lines_at(slot, lines)
    }

    /// Parse each element as exactly one body line, and insert them at body position `at`, as
    /// [`append_body_lines`](Self::append_body_lines) does at the content end.
    ///
    /// Lines above a section's header are the end of the previous section or of the preamble:
    /// insert them there, at that container's `body_line_count()`.
    ///
    /// # Errors
    ///
    /// [`EditError::PositionOutOfRange`] if `at > body_line_count()`, then as
    /// [`append_body_lines`](Self::append_body_lines).
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn insert_body_lines(&mut self, at: usize, lines: &[&str]) -> Result<(), EditError> {
        let slot = self.editor.slot_at(&self.container, at)?;
        self.insert_lines_at(slot, lines)?;
        // Positions count body lines: the first new line is body line `at`.
        #[cfg(all(debug_assertions, not(miri)))]
        if let Some(first) = lines.first() {
            let line = self.editor.body_lines(&self.container).nth(at);
            let line = line.expect("the first new line is body line `at`");
            assert!(line.to_string().starts_with(first));
        }
        Ok(())
    }

    /// Remove body lines `range`. If the removal leaves a CR line break directly before a blank
    /// line made of one LF, a reader reads the two as one CRLF, so that blank line goes too and
    /// the CR becomes that CRLF in the tree; no byte changes.
    ///
    /// # Errors
    ///
    /// In this order: [`EditError::RangeOutOfRange`] if the range is reversed, then if it ends
    /// past `body_line_count()`; [`EditError::ByteOrderMarkAtStart`] if the removal starts at the
    /// preamble's first line of a document without a byte order mark and exposes a line starting
    /// with U+FEFF.
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn remove_body_lines(&mut self, range: Range<usize>) -> Result<(), EditError> {
        let plan = self.editor.plan_remove(&self.container, range.clone())?;
        #[cfg(all(debug_assertions, not(miri)))]
        let body_line_count = self.body_line_count();
        self.editor.commit(plan);
        // The body loses the range, and at most the one blank line that rule D merges.
        #[cfg(all(debug_assertions, not(miri)))]
        {
            assert!(self.body_line_count() + range.len() <= body_line_count);
            assert!(body_line_count <= self.body_line_count() + range.len() + 1);
        }
        Ok(())
    }

    /// Remove this section, header and body, or every line of the preamble; a byte order mark
    /// stays. This cannot fail: the line after a section or the preamble is a header or nothing,
    /// which no line break or byte order mark rule concerns.
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn remove(self) {
        let plan = match &self.container {
            Container::Preamble => {
                let body_line_count = self.body_line_count();
                self.editor
                    .plan_remove(&Container::Preamble, 0..body_line_count)
                    .expect("the line after the preamble is a header or nothing")
            }
            Container::Section(section) => {
                assert!(section.parent().is_some());
                let removed = usize::from(section.text_range().len());
                plan::Plan {
                    steps: vec![Step::Remove {
                        node: section.clone(),
                    }],
                    len_after: self
                        .editor
                        .check_len(removed, 0)
                        .expect("removals only shrink the document"),
                }
            }
        };
        self.editor.commit(plan);
    }

    /// The first entry of this container keyed exactly `key`.
    fn find_entry(&self, key: &str) -> Option<SyntaxNode> {
        self.editor
            .entries(&self.container)
            .find(|entry| plan::key_token(entry).text() == key)
    }

    /// Insert one new entry at `slot`.
    fn insert_entry_at(&mut self, slot: Slot, key: &str, value: &str) -> Result<(), EditError> {
        let (before, after) = self.editor.edit_options.separator_spacing.new_gap();
        let contents = [vec![key, before, "=", after, value]];
        let expected = NewLines::Entry { key, value };
        let plan = self.editor.plan_insert(slot, &contents, &expected)?;
        #[cfg(all(debug_assertions, not(miri)))]
        let body_line_count = self.body_line_count();
        self.editor.commit(plan);
        // The entry is one new body line; rule C only terminates the line before it.
        #[cfg(all(debug_assertions, not(miri)))]
        assert_eq!(self.body_line_count(), body_line_count + 1);
        Ok(())
    }

    /// Insert raw body lines at `slot`.
    fn insert_lines_at(&mut self, slot: Slot, lines: &[&str]) -> Result<(), EditError> {
        if lines.is_empty() {
            return Ok(());
        }
        let contents: Vec<Vec<&str>> = lines.iter().map(|line| vec![*line]).collect();
        let plan = self
            .editor
            .plan_insert(slot, &contents, &NewLines::Body(lines))?;
        #[cfg(all(debug_assertions, not(miri)))]
        let body_line_count = self.body_line_count();
        self.editor.commit(plan);
        // Each element is exactly one new body line.
        #[cfg(all(debug_assertions, not(miri)))]
        assert_eq!(self.body_line_count(), body_line_count + lines.len());
        Ok(())
    }
}
