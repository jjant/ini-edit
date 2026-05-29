//! Format-preserving mutation API using rowan's mutable tree.
//!
//! The [`Editor`] uses `clone_for_update` to get a mutable view of the
//! syntax tree, then applies mutations via `splice_children` and
//! `replace_with`. Untouched subtrees are structurally shared.
//!
//! # Example
//!
//! ```
//! use ini_edit::editor::Editor;
//!
//! let src = "[server]\nhost = 0.0.0.0\nport = 8080\n";
//! let mut editor = Editor::new(src);
//!
//! editor.section("server").set("port", "9090");
//! editor.section("server").append_entry("timeout", "30");
//!
//! let output = editor.finish();
//! assert!(output.contains("port = 9090"));
//! assert!(output.contains("timeout = 30"));
//! assert!(output.contains("host = 0.0.0.0")); // untouched
//! ```

use crate::ast::{AstNode, Entry, File, Section};
use crate::green_builders;
use crate::parse;
use crate::syntax_kind::{SyntaxKind, SyntaxNode};

/// A format-preserving editor for INI files.
#[derive(Debug)]
pub struct Editor {
    root: SyntaxNode,
}

impl Editor {
    /// Create an editor from source text.
    #[must_use]
    pub fn new(src: &str) -> Self {
        let p = parse(src);
        let root = p.syntax().clone_for_update();
        Self { root }
    }

    /// Get a handle to a section. Creates the section at the end of the
    /// file if it doesn't exist.
    #[must_use]
    #[allow(clippy::missing_panics_doc)] // Panic is unreachable: we just spliced the section in.
    pub fn section(&self, name: &str) -> SectionEditor<'_> {
        if let Some(section) = self.find_section(name) {
            return SectionEditor {
                editor: self,
                node: section.syntax().clone(),
            };
        }

        // Create new section at end of root.
        let new_section_green = green_builders::empty_section_node(name);
        let new_section = SyntaxNode::new_root(new_section_green).clone_for_update();

        let child_count = self.root.children_with_tokens().count();
        self.root
            .splice_children(child_count..child_count, vec![new_section.clone().into()]);

        let section = self.find_section(name).expect("just inserted");
        SectionEditor {
            editor: self,
            node: section.syntax().clone(),
        }
    }

    /// Render the final output.
    #[must_use]
    pub fn finish(&self) -> String {
        self.root.text().to_string()
    }

    fn find_section(&self, name: &str) -> Option<Section> {
        let file = File::cast(self.root.clone())?;
        file.sections().find(|s| s.name().as_deref() == Some(name))
    }
}

/// Handle for editing a specific section.
pub struct SectionEditor<'a> {
    #[allow(dead_code)]
    editor: &'a Editor,
    node: SyntaxNode,
}

impl SectionEditor<'_> {
    /// Set a key's value. Updates in-place if exists, appends if not.
    #[allow(clippy::missing_panics_doc)] // Parser always creates a VALUE node inside ENTRY.
    pub fn set(&self, key: &str, value: &str) {
        if let Some(entry) = self.find_entry(key) {
            let value_node = entry.value_node().expect("entry has VALUE node");
            let value_syntax = value_node.syntax().clone();

            // splice_children: replace all children of VALUE with new content.
            let old_count = value_syntax.children_with_tokens().count();
            let new_children: Vec<crate::SyntaxElement> = if value.is_empty() {
                vec![]
            } else {
                let fresh =
                    SyntaxNode::new_root(green_builders::value_node(value)).clone_for_update();
                fresh.children_with_tokens().collect()
            };
            value_syntax.splice_children(0..old_count, new_children);
        } else {
            self.append_entry(key, value);
        }
    }

    /// Append a new entry at the end of this section (canonical format).
    pub fn append_entry(&self, key: &str, value: &str) {
        let new_entry_green = green_builders::entry_node(key, value);
        let new_entry = SyntaxNode::new_root(new_entry_green).clone_for_update();
        let child_count = self.node.children_with_tokens().count();
        self.node
            .splice_children(child_count..child_count, vec![new_entry.into()]);
    }

    /// Remove an entry by key name. Returns true if found and removed.
    #[must_use]
    pub fn remove_entry(&self, key: &str) -> bool {
        if let Some(entry) = self.find_entry(key) {
            entry.syntax().detach();
            true
        } else {
            false
        }
    }

    /// Rename a key (preserving its value and formatting).
    #[must_use]
    pub fn rename_key(&self, old_key: &str, new_key: &str) -> bool {
        if let Some(entry) = self.find_entry(old_key) {
            if let Some(key_node) = entry.key_node() {
                let key_syntax = key_node.syntax().clone();
                let old_count = key_syntax.children_with_tokens().count();
                let fresh =
                    SyntaxNode::new_root(green_builders::key_node(new_key)).clone_for_update();
                let new_children: Vec<crate::SyntaxElement> =
                    fresh.children_with_tokens().collect();
                key_syntax.splice_children(0..old_count, new_children);
                return true;
            }
        }
        false
    }

    /// Remove this entire section (header + all entries).
    pub fn remove(self) {
        self.node.detach();
    }

    /// Insert raw text lines at the end of this section.
    ///
    /// Each line is parsed and inserted as a proper tree node (entry or
    /// comment). Lines that don't parse as either are inserted as comments
    /// to preserve them losslessly.
    pub fn insert_raw_lines(&self, lines: &[&str]) {
        for line in lines {
            let text = if line.ends_with('\n') {
                (*line).to_string()
            } else {
                format!("{line}\n")
            };

            let child_count = self.node.children_with_tokens().count();
            let trimmed = text.trim_start();

            if trimmed.starts_with(';') || trimmed.starts_with('#') {
                // Comment line: parse as tokens.
                let comment_text = text.trim_end_matches(['\n', '\r']);
                let comment_green = {
                    let mut b = rowan::GreenNodeBuilder::new();
                    b.start_node(SyntaxKind::ENTRY.into()); // wrapper node
                    b.token(SyntaxKind::COMMENT.into(), comment_text);
                    b.token(SyntaxKind::NEWLINE.into(), "\n");
                    b.finish_node();
                    b.finish()
                };
                let node = SyntaxNode::new_root(comment_green).clone_for_update();
                // Splice the children (COMMENT + NEWLINE) directly, not the wrapper.
                let children: Vec<crate::SyntaxElement> = node.children_with_tokens().collect();
                self.node
                    .splice_children(child_count..child_count, children);
            } else if let Some(eq_pos) = trimmed.find('=') {
                // Entry line.
                let k = trimmed[..eq_pos].trim();
                let v = trimmed[eq_pos + 1..].trim_end_matches(['\n', '\r']).trim();
                let entry_green = green_builders::entry_node(k, v);
                let entry_node = SyntaxNode::new_root(entry_green).clone_for_update();
                self.node
                    .splice_children(child_count..child_count, vec![entry_node.into()]);
            } else {
                // Unknown line — preserve as comment.
                let raw = text.trim_end_matches(['\n', '\r']);
                let green = {
                    let mut b = rowan::GreenNodeBuilder::new();
                    b.start_node(SyntaxKind::ENTRY.into());
                    b.token(SyntaxKind::COMMENT.into(), raw);
                    b.token(SyntaxKind::NEWLINE.into(), "\n");
                    b.finish_node();
                    b.finish()
                };
                let node = SyntaxNode::new_root(green).clone_for_update();
                let children: Vec<crate::SyntaxElement> = node.children_with_tokens().collect();
                self.node
                    .splice_children(child_count..child_count, children);
            }
        }
    }

    /// Remove a range of child elements (0-indexed within this section).
    ///
    /// Index 0 is the section header. Entries, comments, and whitespace
    /// tokens each count as one element.
    pub fn remove_lines(&self, range: std::ops::Range<usize>) {
        // Collect then detach — splice_children has issues with large ranges
        // in rowan's mutable tree (indices shift during removal).
        let to_remove: Vec<_> = self
            .node
            .children_with_tokens()
            .skip(range.start)
            .take(range.end - range.start)
            .collect();
        for child in to_remove {
            match child {
                rowan::NodeOrToken::Node(n) => n.detach(),
                rowan::NodeOrToken::Token(t) => t.detach(),
            }
        }
    }

    fn find_entry(&self, key: &str) -> Option<Entry> {
        let section = Section::cast(self.node.clone())?;
        section.entries().find(|e| e.key().as_deref() == Some(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_existing_value() {
        let ed = Editor::new("[server]\nhost = 0.0.0.0\nport = 8080\n");
        ed.section("server").set("port", "9090");
        let out = ed.finish();
        assert!(out.contains("port = 9090"), "got: {out}");
        assert!(out.contains("host = 0.0.0.0"));
    }

    #[test]
    fn append_new_entry() {
        let ed = Editor::new("[server]\nhost = 0.0.0.0\n");
        ed.section("server").append_entry("port", "8080");
        let out = ed.finish();
        assert!(out.contains("port = 8080"), "got: {out}");
        assert!(out.contains("host = 0.0.0.0"));
    }

    #[test]
    fn set_creates_if_missing() {
        let ed = Editor::new("[server]\nhost = 0.0.0.0\n");
        ed.section("server").set("port", "8080");
        let out = ed.finish();
        assert!(out.contains("port = 8080"), "got: {out}");
    }

    #[test]
    fn auto_create_section() {
        let ed = Editor::new("[existing]\nk = v\n");
        ed.section("new").append_entry("key", "value");
        let out = ed.finish();
        assert!(out.contains("[new]"), "got: {out}");
        assert!(out.contains("key = value"), "got: {out}");
        assert!(out.contains("[existing]"));
    }

    #[test]
    fn remove_entry() {
        let ed = Editor::new("[s]\na = 1\nb = 2\nc = 3\n");
        assert!(ed.section("s").remove_entry("b"));
        let out = ed.finish();
        assert!(!out.contains("b = 2"), "got: {out}");
        assert!(out.contains("a = 1"));
        assert!(out.contains("c = 3"));
    }

    #[test]
    fn rename_key() {
        let ed = Editor::new("[s]\nold_name = value\n");
        assert!(ed.section("s").rename_key("old_name", "new_name"));
        let out = ed.finish();
        assert!(out.contains("new_name = value"), "got: {out}");
        assert!(!out.contains("old_name"));
    }

    #[test]
    fn remove_section() {
        let ed = Editor::new("; comment\n[a]\nx = 1\n[b]\ny = 2\n");
        ed.section("a").remove();
        let out = ed.finish();
        assert!(!out.contains("[a]"), "got: {out}");
        assert!(!out.contains("x = 1"), "got: {out}");
        assert!(out.contains("; comment"));
        assert!(out.contains("[b]"));
    }

    #[test]
    fn preserves_formatting() {
        let ed = Editor::new(
            "; top comment\n\n[server]\nhost = 0.0.0.0\nport = 8080\n\n[other]\nk = v\n",
        );
        ed.section("server").set("port", "9090");
        let out = ed.finish();
        assert!(out.starts_with("; top comment"), "got: {out}");
        assert!(out.contains("[other]\nk = v"), "got: {out}");
        assert!(out.contains("host = 0.0.0.0"), "got: {out}");
    }

    #[test]
    fn insert_raw_lines_entries() {
        let ed = Editor::new("[s]\nk = v\n");
        ed.section("s")
            .insert_raw_lines(&["new_key=new_val", "another = thing"]);
        let out = ed.finish();
        assert!(out.contains("new_key = new_val"), "got: {out}");
        assert!(out.contains("another = thing"), "got: {out}");
        assert!(out.contains("k = v"));
    }

    #[test]
    fn insert_raw_lines_comments() {
        let ed = Editor::new("[s]\nk = v\n");
        ed.section("s")
            .insert_raw_lines(&["; marker start", "; marker end"]);
        let out = ed.finish();
        assert!(out.contains("; marker start"), "got: {out}");
        assert!(out.contains("; marker end"), "got: {out}");
    }

    #[test]
    fn remove_lines_by_range() {
        let ed = Editor::new("[s]\na = 1\nb = 2\nc = 3\n");
        // Children: SECTION_HEADER, NEWLINE, ENTRY(a), ENTRY(b), ENTRY(c)
        // Remove indices 3..4 should remove ENTRY(b)
        ed.section("s").remove_lines(3..4);
        let out = ed.finish();
        assert!(out.contains("a = 1"), "got: {out}");
        assert!(!out.contains("b = 2"), "got: {out}");
        assert!(out.contains("c = 3"), "got: {out}");
    }
}
