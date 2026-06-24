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
use crate::syntax_kind::{SyntaxKind, SyntaxNode};
use crate::{ParseOptions, parse_with};

/// A format-preserving editor for INI files.
#[derive(Debug)]
pub struct Editor {
    root: SyntaxNode,
}

impl Editor {
    /// Create an editor from source text.
    #[must_use]
    pub fn new(src: &str) -> Self {
        Self::with_parse_options(src, &ParseOptions::default())
    }

    /// Create an editor from source text using custom [`ParseOptions`].
    ///
    /// Use this to edit files whose features require opt-in parsing, e.g.
    /// [`inline_comments`](crate::ParseOptions::inline_comments) — passing the
    /// same options the file was authored with ensures inline comments are
    /// preserved across edits rather than absorbed into values.
    #[must_use]
    pub fn with_parse_options(src: &str, options: &ParseOptions) -> Self {
        let p = parse_with(src, options);
        let root = p.syntax().clone_for_update();
        Self { root }
    }

    /// Get a handle to a section. Creates the section at the end of the
    /// file if it doesn't exist.
    #[must_use]
    pub fn section(&self, name: &str) -> SectionEditor<'_> {
        if let Some(section) = self.find_section(name) {
            return SectionEditor {
                editor: self,
                node: section.syntax().clone(),
            };
        }

        // Create the section at the end of the file. It always starts on its
        // own line, separated from existing content by one blank line.
        let new_section_green = green_builders::empty_section_node(name);
        let new_section = SyntaxNode::new_root(new_section_green).clone_for_update();

        let mut elements: Vec<crate::SyntaxElement> = Vec::new();
        for _ in 0..self.separator_blank_lines() {
            let blank = SyntaxNode::new_root(green_builders::blank_line_node()).clone_for_update();
            elements.push(blank.into());
        }
        elements.push(new_section.clone().into());

        let child_count = self.root.children_with_tokens().count();
        self.root
            .splice_children(child_count..child_count, elements);

        #[expect(clippy::missing_panics_doc, reason = "we just spliced the section in")]
        let section = self.find_section(name).expect("just inserted");
        SectionEditor {
            editor: self,
            node: section.syntax().clone(),
        }
    }

    /// Number of blank-line nodes to insert before a newly-created section so
    /// that it begins on its own line and is separated from existing content by
    /// exactly one blank line.
    ///
    /// - `0` when the file is empty or already ends with a blank line.
    /// - `1` when the last line is terminated but not blank (adds the blank).
    /// - `2` when the last line has no terminating newline (terminates it, then
    ///   adds the blank) — this prevents gluing the new `[section]` onto the
    ///   previous line.
    fn separator_blank_lines(&self) -> usize {
        let Some(last_token) = self.root.last_token() else {
            return 0; // empty file
        };
        if last_token.kind() != SyntaxKind::NEWLINE {
            return 2; // unterminated last line
        }
        usize::from(!self.ends_with_blank_line())
    }

    /// Whether the file's last line is already blank.
    fn ends_with_blank_line(&self) -> bool {
        let last_line = match self.root.last_child() {
            Some(node) if node.kind() == SyntaxKind::SECTION => node.last_child(),
            other => other,
        };
        last_line.is_some_and(|n| n.kind() == SyntaxKind::BLANK_LINE)
    }

    /// Render the final output.
    #[must_use]
    pub fn finish(&self) -> String {
        self.root.text().to_string()
    }

    /// A read-only typed view over the editor's current tree.
    ///
    /// Inspect sections and entries before or between edits without parsing the
    /// source a second time. The returned [`File`] reflects the tree at the
    /// time of the call; fetch it again after mutating to observe changes.
    ///
    /// ```
    /// use ini_edit::editor::Editor;
    ///
    /// let ed = Editor::new("[server]\nhost = 0.0.0.0\nport = 8080\n");
    ///
    /// // Read using the same parse that backs the editor — no second parse.
    /// let names: Vec<_> = ed.file().sections().filter_map(|s| s.name()).collect();
    /// assert_eq!(names, ["server"]);
    ///
    /// // Then mutate the same tree.
    /// ed.section("server").set("port", "9090");
    /// assert!(ed.finish().contains("port = 9090"));
    /// ```
    #[must_use]
    pub fn file(&self) -> File {
        #[expect(
            clippy::missing_panics_doc,
            reason = "the editor root is always a ROOT node produced by the parser"
        )]
        let file = File::cast(self.root.clone()).expect("editor root is a ROOT node");
        file
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
    pub fn set(&self, key: &str, value: &str) {
        if let Some(entry) = self.find_entry(key) {
            #[expect(
                clippy::missing_panics_doc,
                reason = "parser always creates a VALUE node inside ENTRY"
            )]
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

    /// Append a new entry to this section (canonical `key = value` format).
    ///
    /// The entry is inserted after the section's last content line (entry or
    /// comment) and **before** any trailing blank lines that separate this
    /// section from the next, so it joins the section's body rather than
    /// drifting below the blank-line gap. If the preceding content line has no
    /// terminating newline (e.g. at end of file), a separating newline is
    /// inserted first.
    pub fn append_entry(&self, key: &str, value: &str) {
        let (index, needs_newline) = self.content_end();
        let entry = SyntaxNode::new_root(green_builders::entry_node(key, value)).clone_for_update();
        let mut elements: Vec<crate::SyntaxElement> = Vec::new();
        if needs_newline {
            elements.push(Self::newline_element());
        }
        elements.push(entry.into());
        self.node.splice_children(index..index, elements);
    }

    /// Insert a new entry after the `line`-th logical content line within this
    /// section (1-based). Logical content lines are entries and comments; the
    /// `[section]` header is not counted, so `line == 0` inserts before the
    /// first content line (right after the header). A `line` value at or beyond
    /// the number of content lines appends after the last one, exactly like
    /// [`append_entry`](Self::append_entry).
    ///
    /// In contrast to [`insert_raw_lines_at`](Self::insert_raw_lines_at) — which
    /// takes a raw child index (counting the header and blank lines) and inserts
    /// verbatim, opaque text — this counts only content lines and inserts a
    /// parsed entry, so the result is found by [`set`](Self::set),
    /// [`remove_entry`](Self::remove_entry), and [`rename_key`](Self::rename_key).
    pub fn insert_entry_at_line(&self, line: usize, key: &str, value: &str) {
        let (index, needs_newline) = self.after_content_line(line);
        let entry = SyntaxNode::new_root(green_builders::entry_node(key, value)).clone_for_update();
        let mut elements: Vec<crate::SyntaxElement> = Vec::new();
        if needs_newline {
            elements.push(Self::newline_element());
        }
        elements.push(entry.into());
        self.node.splice_children(index..index, elements);
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

    /// Append raw text lines to this section's body.
    ///
    /// Lines are inserted **verbatim** — no parsing, no reformatting. A newline
    /// is appended to each line that doesn't already end with one. Like
    /// [`append_entry`](Self::append_entry), lines land after the last content
    /// line and before any trailing blank lines.
    pub fn append_raw_lines(&self, lines: &[&str]) {
        let (index, needs_newline) = self.content_end();
        let mut elements: Vec<crate::SyntaxElement> = Vec::new();
        if needs_newline {
            elements.push(Self::newline_element());
        }
        elements.extend(Self::raw_line_elements(lines));
        self.node.splice_children(index..index, elements);
    }

    /// Insert raw text lines at a specific child index within this section.
    ///
    /// Index 0 is the section header. Lines are inserted **verbatim**.
    /// A newline is appended to each line that doesn't already end with one.
    /// If `index` exceeds the number of children, lines are appended at the end.
    pub fn insert_raw_lines_at(&self, index: usize, lines: &[&str]) {
        let child_count = self.node.children_with_tokens().count();
        let index = index.min(child_count);
        let elements = Self::raw_line_elements(lines);
        self.node.splice_children(index..index, elements);
    }

    /// Build the verbatim child elements for a set of raw lines. Each line
    /// becomes a `COMMENT_LINE` node: its content is an opaque `COMMENT` token
    /// followed by its newline, keeping the line-node invariant intact.
    fn raw_line_elements(lines: &[&str]) -> Vec<crate::SyntaxElement> {
        let mut elements: Vec<crate::SyntaxElement> = Vec::new();
        for line in lines {
            let text = if line.ends_with('\n') || line.ends_with('\r') {
                (*line).to_string()
            } else {
                format!("{line}\n")
            };
            // Emit the line content (without newline) as a COMMENT token
            // and the newline separately, wrapped in a COMMENT_LINE node.
            // COMMENT is used as a generic "opaque text" kind — it preserves
            // the content verbatim.
            let content = text.trim_end_matches(['\n', '\r']);
            let nl = if text.ends_with("\r\n") { "\r\n" } else { "\n" };

            let green = {
                let mut b = rowan::GreenNodeBuilder::new();
                b.start_node(SyntaxKind::COMMENT_LINE.into());
                b.token(SyntaxKind::COMMENT.into(), content);
                b.token(SyntaxKind::NEWLINE.into(), nl);
                b.finish_node();
                b.finish()
            };
            let node = SyntaxNode::new_root(green).clone_for_update();
            elements.push(node.into());
        }
        elements
    }

    /// A standalone `NEWLINE` token element, used to separate appended content
    /// from a preceding line that lacks its own terminator.
    fn newline_element() -> crate::SyntaxElement {
        let mut b = rowan::GreenNodeBuilder::new();
        b.start_node(SyntaxKind::ROOT.into());
        b.token(SyntaxKind::NEWLINE.into(), "\n");
        b.finish_node();
        let wrapper = SyntaxNode::new_root(b.finish()).clone_for_update();
        wrapper
            .children_with_tokens()
            .next()
            .expect("wrapper contains one newline token")
    }

    /// Insertion point at the end of the section's logical content: just after
    /// the last entry/comment/header line, but before any trailing blank lines.
    /// Returns `(child_index, needs_newline)`.
    fn content_end(&self) -> (usize, bool) {
        let children: Vec<crate::SyntaxElement> = self.node.children_with_tokens().collect();
        // A SECTION always has at least its header, so a content line always
        // exists; fall back to the first child defensively.
        let k = children
            .iter()
            .rposition(|el| {
                el.as_node().is_some_and(|n| {
                    matches!(
                        n.kind(),
                        SyntaxKind::ENTRY | SyntaxKind::COMMENT_LINE | SyntaxKind::SECTION_HEADER
                    )
                })
            })
            .unwrap_or(0);
        Self::after_element(&children, k)
    }

    /// Insertion point just after the `n`-th logical content line (1-based),
    /// where content lines are entries and comments (header excluded). `n == 0`
    /// (or an empty body) inserts right after the header; `n` is clamped to the
    /// number of content lines. Returns `(child_index, needs_newline)`.
    fn after_content_line(&self, n: usize) -> (usize, bool) {
        let children: Vec<crate::SyntaxElement> = self.node.children_with_tokens().collect();
        let header = children
            .iter()
            .position(|el| {
                el.as_node()
                    .is_some_and(|node| node.kind() == SyntaxKind::SECTION_HEADER)
            })
            .unwrap_or(0);
        let content: Vec<usize> = children
            .iter()
            .enumerate()
            .filter(|(_, el)| {
                el.as_node().is_some_and(|node| {
                    matches!(node.kind(), SyntaxKind::ENTRY | SyntaxKind::COMMENT_LINE)
                })
            })
            .map(|(i, _)| i)
            .collect();

        if n == 0 || content.is_empty() {
            return Self::after_element(&children, header);
        }
        let n = n.min(content.len());
        Self::after_element(&children, content[n - 1])
    }

    /// Compute the insertion point immediately after the line node at
    /// `children[k]`. In the line-node model every line node owns its
    /// terminating newline, so this is simply the next index — with
    /// `needs_newline` set when the line has no terminator (end of file).
    fn after_element(children: &[crate::SyntaxElement], k: usize) -> (usize, bool) {
        // Content lines are always nodes that own their terminating newline.
        let ends_with_newline = children[k]
            .as_node()
            .and_then(rowan::SyntaxNode::last_token)
            .is_some_and(|t| t.kind() == SyntaxKind::NEWLINE);
        (k + 1, !ends_with_newline)
    }

    /// Remove a range of child line nodes (0-indexed within this section).
    ///
    /// In the line-node model each physical line is one child, so index 0 is
    /// the section header and child index equals line index. Entries, comment
    /// lines, and blank lines each count as one element.
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
    fn append_raw_lines_entries() {
        let ed = Editor::new("[s]\nk = v\n");
        ed.section("s")
            .append_raw_lines(&["new_key=new_val", "another = thing"]);
        let out = ed.finish();
        assert!(out.contains("new_key=new_val"), "got: {out}");
        assert!(out.contains("another = thing"), "got: {out}");
        assert!(out.contains("k = v"));
    }

    #[test]
    fn append_raw_lines_comments() {
        let ed = Editor::new("[s]\nk = v\n");
        ed.section("s")
            .append_raw_lines(&["; marker start", "; marker end"]);
        let out = ed.finish();
        assert!(out.contains("; marker start"), "got: {out}");
        assert!(out.contains("; marker end"), "got: {out}");
    }

    #[test]
    fn remove_lines_by_range() {
        let ed = Editor::new("[s]\na = 1\nb = 2\nc = 3\n");
        // Line-node layout — child index == line index:
        //   0 SECTION_HEADER, 1 ENTRY(a), 2 ENTRY(b), 3 ENTRY(c)
        // Remove index 2..3 to drop ENTRY(b).
        ed.section("s").remove_lines(2..3);
        let out = ed.finish();
        assert!(out.contains("a = 1"), "got: {out}");
        assert!(!out.contains("b = 2"), "got: {out}");
        assert!(out.contains("c = 3"), "got: {out}");
    }

    #[test]
    fn set_empty_value() {
        let ed = Editor::new("[s]\nk = old\n");
        ed.section("s").set("k", "");
        let out = ed.finish();
        assert!(out.contains("k = \n") || out.contains("k = "), "got: {out}");
    }

    #[test]
    fn rename_nonexistent_key() {
        let ed = Editor::new("[s]\nk = v\n");
        assert!(!ed.section("s").rename_key("nonexistent", "new"));
    }

    #[test]
    fn append_raw_unknown_line() {
        let ed = Editor::new("[s]\nk = v\n");
        ed.section("s")
            .append_raw_lines(&["just some text without equals"]);
        let out = ed.finish();
        assert!(out.contains("just some text without equals"), "got: {out}");
    }

    #[test]
    fn insert_raw_lines_at_position() {
        let ed = Editor::new("[s]\na = 1\nb = 2\n");
        // Line-node layout: 0 SECTION_HEADER, 1 ENTRY(a), 2 ENTRY(b).
        // Insert at index 2 to land between a and b.
        ed.section("s").insert_raw_lines_at(2, &["; injected"]);
        let out = ed.finish();
        // The injected line should appear between a and b.
        let a_pos = out.find("a = 1").unwrap();
        let inj_pos = out.find("; injected").unwrap();
        let b_pos = out.find("b = 2").unwrap();
        assert!(a_pos < inj_pos, "got: {out}");
        assert!(inj_pos < b_pos, "got: {out}");
    }

    #[test]
    fn append_raw_lines_are_verbatim() {
        // Verify no reformatting happens — exact text preserved.
        let ed = Editor::new("[s]\n");
        ed.section("s")
            .append_raw_lines(&["key=value_no_spaces", "  indented=line"]);
        let out = ed.finish();
        assert!(out.contains("key=value_no_spaces\n"), "got: {out}");
        assert!(out.contains("  indented=line\n"), "got: {out}");
    }

    #[test]
    fn set_preserves_inline_comment() {
        let opts = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        let ed = Editor::with_parse_options("[s]\nretain = 1   ; keep this note\n", &opts);
        ed.section("s").set("retain", "0");
        assert_eq!(ed.finish(), "[s]\nretain = 0   ; keep this note\n");
    }

    #[test]
    fn rename_key_preserves_inline_comment() {
        let opts = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        let ed = Editor::with_parse_options("[s]\nold = v ; note\n", &opts);
        assert!(ed.section("s").rename_key("old", "new"));
        assert_eq!(ed.finish(), "[s]\nnew = v ; note\n");
    }

    // --- content-aware append ---

    #[test]
    fn append_entry_before_trailing_blank_line() {
        let ed =
            Editor::new("[server]\nhost = 0.0.0.0\nport = 8080\n\n[database]\nurl = localhost\n");
        ed.section("server").append_entry("tls", "on");
        assert_eq!(
            ed.finish(),
            "[server]\nhost = 0.0.0.0\nport = 8080\ntls = on\n\n[database]\nurl = localhost\n"
        );
    }

    #[test]
    fn append_entry_no_trailing_blank() {
        let ed = Editor::new("[s]\na = 1\n");
        ed.section("s").append_entry("b", "2");
        assert_eq!(ed.finish(), "[s]\na = 1\nb = 2\n");
    }

    #[test]
    fn append_entry_eof_without_newline() {
        // Last line has no terminator — a separating newline must be added.
        let ed = Editor::new("[s]\nhost = 0.0.0.0");
        ed.section("s").append_entry("port", "8080");
        assert_eq!(ed.finish(), "[s]\nhost = 0.0.0.0\nport = 8080\n");
    }

    #[test]
    fn append_entry_into_empty_section() {
        let ed = Editor::new("[s]\n");
        ed.section("s").append_entry("k", "v");
        assert_eq!(ed.finish(), "[s]\nk = v\n");
    }

    #[test]
    fn append_entry_after_trailing_comment() {
        // A trailing comment counts as content; the new entry goes after it,
        // before the blank line.
        let ed = Editor::new("[s]\na = 1\n; trailing note\n\n[next]\n");
        ed.section("s").append_entry("b", "2");
        assert_eq!(
            ed.finish(),
            "[s]\na = 1\n; trailing note\nb = 2\n\n[next]\n"
        );
    }

    #[test]
    fn append_entry_set_path_respects_trailing_blank() {
        // `set` on a missing key uses the same content-aware append.
        let ed = Editor::new("[s]\na = 1\n\n[next]\n");
        ed.section("s").set("b", "2");
        assert_eq!(ed.finish(), "[s]\na = 1\nb = 2\n\n[next]\n");
    }

    #[test]
    fn append_raw_lines_before_trailing_blank() {
        let ed = Editor::new("[s]\na = 1\n\n[next]\n");
        ed.section("s").append_raw_lines(&["raw = line"]);
        assert_eq!(ed.finish(), "[s]\na = 1\nraw = line\n\n[next]\n");
    }

    // --- logical-line insert ---

    #[test]
    fn insert_entry_at_line_middle() {
        let ed = Editor::new("[s]\na = 1\nb = 2\nc = 3\n");
        // After the 2nd logical content line (b).
        ed.section("s").insert_entry_at_line(2, "x", "9");
        assert_eq!(ed.finish(), "[s]\na = 1\nb = 2\nx = 9\nc = 3\n");
    }

    #[test]
    fn insert_entry_at_line_zero_is_before_first() {
        let ed = Editor::new("[s]\na = 1\nb = 2\n");
        ed.section("s").insert_entry_at_line(0, "x", "9");
        assert_eq!(ed.finish(), "[s]\nx = 9\na = 1\nb = 2\n");
    }

    #[test]
    fn insert_entry_at_line_beyond_end_appends() {
        let ed = Editor::new("[s]\na = 1\nb = 2\n\n[next]\n");
        ed.section("s").insert_entry_at_line(99, "x", "9");
        // Clamped to append, still before the trailing blank line.
        assert_eq!(ed.finish(), "[s]\na = 1\nb = 2\nx = 9\n\n[next]\n");
    }

    #[test]
    fn insert_entry_at_line_counts_comments() {
        let ed = Editor::new("[s]\na = 1\n; note\nb = 2\n");
        // Content lines: 1=a, 2=; note, 3=b. Insert after line 2 (the comment).
        ed.section("s").insert_entry_at_line(2, "x", "9");
        assert_eq!(ed.finish(), "[s]\na = 1\n; note\nx = 9\nb = 2\n");
    }

    #[test]
    fn insert_entry_at_line_eof_without_newline() {
        // Inserting after a line that lacks a terminator adds a separating one.
        let ed = Editor::new("[s]\na = 1");
        ed.section("s").insert_entry_at_line(1, "b", "2");
        assert_eq!(ed.finish(), "[s]\na = 1\nb = 2\n");
    }

    #[test]
    fn insert_entry_at_line_into_empty_section() {
        let ed = Editor::new("[s]\n");
        ed.section("s").insert_entry_at_line(1, "k", "v");
        assert_eq!(ed.finish(), "[s]\nk = v\n");
    }

    // --- coverage for raw-line and edge paths ---

    #[test]
    fn remove_entry_missing_returns_false() {
        let ed = Editor::new("[s]\nk = v\n");
        assert!(!ed.section("s").remove_entry("absent"));
        assert!(ed.finish().contains("k = v"));
    }

    #[test]
    fn append_raw_lines_preserves_existing_newline() {
        // A line that already ends in a newline is inserted verbatim.
        let ed = Editor::new("[s]\nk = v\n");
        ed.section("s").append_raw_lines(&["already = newlined\n"]);
        let out = ed.finish();
        assert!(out.contains("already = newlined\n"), "got: {out}");
        assert!(!out.contains("newlined\n\n"), "got: {out}");
    }

    #[test]
    fn append_raw_lines_eof_without_newline() {
        // Appending raw lines after an unterminated last line adds a separator.
        let ed = Editor::new("[s]\nk = v");
        ed.section("s").append_raw_lines(&["raw = line"]);
        assert_eq!(ed.finish(), "[s]\nk = v\nraw = line\n");
    }

    #[test]
    fn remove_lines_detaches_loose_tokens() {
        // A line starting with `=` lexes to a loose LEX_ERROR token (plus its
        // newline) directly under the section, so remove_lines must detach
        // tokens as well as line nodes.
        let ed = Editor::new("[s]\n=bad\nk = v\n");
        // Children: 0 SECTION_HEADER, 1 LEX_ERROR token, 2 NEWLINE token,
        //   3 ENTRY(k). Remove the two loose error tokens.
        ed.section("s").remove_lines(1..3);
        assert_eq!(ed.finish(), "[s]\nk = v\n");
    }

    #[test]
    fn file_read_view_reflects_mutations() {
        let ed = Editor::new("[s]\na = 1\n");
        // Read structure from the editor's own tree (no second parse).
        let file = ed.file();
        let section = file.sections().next().unwrap();
        assert_eq!(section.name().as_deref(), Some("s"));
        assert_eq!(section.entries().count(), 1);

        // Mutate, then re-fetch to observe the change.
        ed.section("s").append_entry("b", "2");
        assert_eq!(ed.file().sections().next().unwrap().entries().count(), 2);
    }

    // --- section creation: separation and no gluing ---

    #[test]
    fn create_section_inserts_blank_line_separator() {
        let ed = Editor::new("[server]\nhost = 0.0.0.0\n");
        ed.section("logging").append_entry("level", "info");
        assert_eq!(
            ed.finish(),
            "[server]\nhost = 0.0.0.0\n\n[logging]\nlevel = info\n"
        );
    }

    #[test]
    fn create_section_does_not_glue_unterminated_line() {
        // Previous content has no trailing newline — the new section must still
        // start on its own line.
        let ed = Editor::new("[server]\nhost = 0.0.0.0");
        ed.section("logging").append_entry("level", "info");
        assert_eq!(
            ed.finish(),
            "[server]\nhost = 0.0.0.0\n\n[logging]\nlevel = info\n"
        );
    }

    #[test]
    fn create_section_no_double_blank_line() {
        // File already ends with a blank line — don't add a second one.
        let ed = Editor::new("[server]\nk = v\n\n");
        ed.section("logging").append_entry("level", "info");
        assert_eq!(ed.finish(), "[server]\nk = v\n\n[logging]\nlevel = info\n");
    }

    #[test]
    fn create_section_in_empty_file_has_no_leading_blank() {
        let ed = Editor::new("");
        ed.section("a").append_entry("k", "v");
        assert_eq!(ed.finish(), "[a]\nk = v\n");
    }

    #[test]
    fn create_section_after_preamble() {
        let ed = Editor::new("g = 1\n");
        ed.section("s").append_entry("k", "v");
        assert_eq!(ed.finish(), "g = 1\n\n[s]\nk = v\n");
    }
}
