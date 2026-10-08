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

/// Formatting options applied when an [`Editor`] creates an entry or assigns
/// its value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditOptions {
    /// Whitespace policy around the `=` or `:` separator.
    pub separator_spacing: SeparatorSpacing,
}

/// Whitespace policy around separators in entries created or value-updated by
/// the editor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SeparatorSpacing {
    /// Preserve an existing entry's spacing and use `key = value` for new
    /// entries or bare keys that need a separator.
    /// Clearing a value keeps its surrounding whitespace in the separator
    /// gap, so later edits use the same spacing after reopening the file.
    #[default]
    Preserve,
    /// Remove whitespace around the separator, producing `key=value`.
    Compact,
    /// Use exact whitespace before and after the separator.
    ///
    /// The supplied strings should contain only spaces and tabs.
    Exact {
        /// Whitespace inserted between the key and separator.
        before: String,
        /// Whitespace inserted between the separator and value.
        after: String,
    },
}

impl SeparatorSpacing {
    /// Create a policy with exact whitespace before and after the separator.
    #[must_use]
    pub fn exact(before: impl Into<String>, after: impl Into<String>) -> Self {
        Self::Exact {
            before: before.into(),
            after: after.into(),
        }
    }
}

/// A format-preserving editor for INI files.
///
/// Lines the editor adds (entries, sections, blank separators, raw lines
/// without a terminator, and the terminator of an unterminated final line) use
/// the document's line ending: the first line break in its current text, or
/// `\n` if it has none. Existing line endings are preserved, except that an
/// `\n` an edit places directly after a bare `\r` becomes `\r\n`, so the two
/// line breaks stay separate.
///
/// ```
/// use ini_edit::editor::Editor;
///
/// let ed = Editor::new("[server]\r\nhost = 0.0.0.0\r\n");
/// ed.section("server").append_entry("port", "8080");
/// assert_eq!(ed.finish(), "[server]\r\nhost = 0.0.0.0\r\nport = 8080\r\n");
/// ```
#[derive(Debug)]
pub struct Editor {
    root: SyntaxNode,
    options: EditOptions,
}

impl Editor {
    /// Create an editor from source text.
    #[must_use]
    pub fn new(src: &str) -> Self {
        Self::with_options(src, &ParseOptions::default(), &EditOptions::default())
    }

    /// Create an editor from source text using custom [`ParseOptions`].
    ///
    /// Use this to edit files whose features require opt-in parsing, e.g.
    /// [`inline_comments`](crate::ParseOptions::inline_comments) — passing the
    /// same options the file was authored with ensures inline comments are
    /// preserved across edits rather than absorbed into values.
    #[must_use]
    pub fn with_parse_options(src: &str, options: &ParseOptions) -> Self {
        Self::with_options(src, options, &EditOptions::default())
    }

    /// Create an editor from source text using custom [`EditOptions`].
    ///
    /// The default parsing behavior is unchanged. Existing entries are
    /// reformatted only when their value is assigned; untouched entries retain
    /// their original bytes.
    #[must_use]
    pub fn with_edit_options(src: &str, options: &EditOptions) -> Self {
        Self::with_options(src, &ParseOptions::default(), options)
    }

    /// Create an editor using custom parsing and editing options.
    ///
    /// This is equivalent to combining [`with_parse_options`](Self::with_parse_options)
    /// and [`with_edit_options`](Self::with_edit_options).
    #[must_use]
    pub fn with_options(
        src: &str,
        parse_options: &ParseOptions,
        edit_options: &EditOptions,
    ) -> Self {
        let p = parse_with(src, parse_options);
        let root = p.syntax().clone_for_update();
        Self {
            root,
            options: edit_options.clone(),
        }
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
        let ending = document_line_ending(&self.root);
        let new_section_green = green_builders::empty_section_node(name, ending);
        let new_section = SyntaxNode::new_root(new_section_green).clone_for_update();

        // Separators belong to the preceding section, just as they do when
        // parsing the same text again. A missing terminator belongs to its line.
        let separator_parent = self
            .root
            .last_child()
            .filter(|node| node.kind() == SyntaxKind::SECTION)
            .unwrap_or_else(|| self.root.clone());
        let needs_newline =
            last_token(&self.root).is_some_and(|token| token.kind() != SyntaxKind::NEWLINE);
        let index = separator_parent.children_with_tokens().count();
        let mut blank_lines = self.separator_blank_lines();
        if needs_newline && blank_lines != 0 {
            terminate_line(&separator_parent, index, ending);
            blank_lines -= 1;
        }
        for _ in 0..blank_lines {
            let blank =
                SyntaxNode::new_root(green_builders::blank_line_node(ending)).clone_for_update();
            separate_line_breaks(last_token(&self.root), blank.first_token());
            separator_parent.splice_children(index..index, vec![blank.into()]);
        }

        let child_count = self.root.children_with_tokens().count();
        self.root
            .splice_children(child_count..child_count, vec![new_section.into()]);

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
    /// - `1` when a content line is terminated (adds the blank), or a blank
    ///   line is unterminated (terminates the existing blank).
    /// - `2` when a content line has no terminating newline (terminates it, then
    ///   adds the blank), preventing the new `[section]` from joining that line.
    fn separator_blank_lines(&self) -> usize {
        let Some(last_token) = last_token(&self.root) else {
            return 0; // empty file
        };
        if last_token.kind() == SyntaxKind::WHITESPACE && last_token.text() == "\u{FEFF}" {
            return 0; // a document marker is not a content line
        }
        if last_token.kind() != SyntaxKind::NEWLINE {
            return 1 + usize::from(!self.ends_with_blank_line());
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
        let root = SyntaxNode::new_root(self.root.green().into_owned());
        #[expect(
            clippy::missing_panics_doc,
            reason = "the editor root is always a ROOT node produced by the parser"
        )]
        let file = File::cast(root).expect("editor root is a ROOT node");
        file
    }

    fn find_section(&self, name: &str) -> Option<Section> {
        let file = File::cast(self.root.clone()).expect("an Editor always stores a ROOT node");
        file.sections().find(|s| s.name().as_deref() == Some(name))
    }
}

/// Rowan's `last_token` stops at an empty VALUE node. Walk backwards past empty
/// nodes so an unterminated `key=` is still recognized as a content line.
fn last_token(node: &SyntaxNode) -> Option<crate::SyntaxToken> {
    let mut child = node.last_child_or_token();
    while let Some(element) = child {
        let token = match &element {
            rowan::NodeOrToken::Node(node) => last_token(node),
            rowan::NodeOrToken::Token(token) => Some(token.clone()),
        };
        if token.is_some() {
            return token;
        }
        child = element.prev_sibling_or_token();
    }
    None
}

/// The first line terminator in the text of `node`'s tree, read as the lexer
/// reads it (CR followed by LF is one CRLF), or LF if there is none.
///
/// Lines the editor creates use this ending, so CRLF and CR documents keep a
/// single style. It depends only on the current text: reopening the output
/// never changes how later lines are terminated.
fn document_line_ending(node: &SyntaxNode) -> &'static str {
    let root = node.ancestors().last().expect("a node is its own ancestor");
    let mut after_cr = false;
    for token in root
        .descendants_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
    {
        let text = token.text();
        // A token ending in CR is followed by the next nonempty token.
        if after_cr && !text.is_empty() {
            return if text.starts_with('\n') { "\r\n" } else { "\r" };
        }
        let Some(start) = text.find(['\r', '\n']) else {
            continue;
        };
        match &text[start..] {
            "\r" => after_cr = true,
            rest if rest.starts_with("\r\n") => return "\r\n",
            rest if rest.starts_with('\r') => return "\r",
            _ => return "\n",
        }
    }
    if after_cr { "\r" } else { "\n" }
}

/// Handle for editing a specific section.
pub struct SectionEditor<'a> {
    editor: &'a Editor,
    node: SyntaxNode,
}

impl SectionEditor<'_> {
    /// Set a key's value. Updates in-place if it exists, appends if not, and
    /// applies the configured [`SeparatorSpacing`] to the touched entry.
    /// Clearing a value moves its inline comment to a preceding comment line,
    /// so reopening the file keeps the value empty.
    /// The same applies to a continuation ending on a blank line.
    pub fn set(&self, key: &str, value: &str) {
        if let Some(entry) = self.find_entry(key) {
            replace_value(
                entry.syntax(),
                value,
                &self.editor.options.separator_spacing,
            );
        } else {
            self.append_entry(key, value);
        }
    }

    /// Append a new entry to this section using the configured separator
    /// spacing.
    ///
    /// The entry is inserted after the section's last content line (entry or
    /// comment) and **before** any trailing blank lines that separate this
    /// section from the next, so it joins the section's body rather than
    /// drifting below the blank-line gap. If the preceding content line has no
    /// terminating newline (e.g. at end of file), a separating newline is
    /// inserted first. Both use the document's line ending (see [`Editor`]).
    pub fn append_entry(&self, key: &str, value: &str) {
        let (index, needs_newline) = self.content_end();
        self.insert_elements(index, needs_newline, |ending| {
            vec![self.entry_element(key, value, ending)]
        });
    }

    /// Insert a new entry after the `line`-th logical content line within this
    /// section (1-based). Logical content lines are entries and comments; the
    /// `[section]` header, blank lines, and malformed lines are not counted.
    ///
    /// - `line == 0` inserts right after the header, before the first content
    ///   line. This holds even when the section has no content lines.
    /// - Any other `line` at or beyond the number of content lines appends
    ///   exactly like [`append_entry`](Self::append_entry): after every line
    ///   that is not blank, including malformed lines that follow the last
    ///   entry or comment, and before any trailing blank lines.
    ///
    /// In contrast to [`insert_raw_lines_at`](Self::insert_raw_lines_at) — which
    /// takes a raw child index (counting the header and blank lines) and inserts
    /// verbatim, opaque text — this counts only content lines and inserts a
    /// parsed entry, so the result is found by [`set`](Self::set),
    /// [`remove_entry`](Self::remove_entry), and [`rename_key`](Self::rename_key).
    pub fn insert_entry_at_line(&self, line: usize, key: &str, value: &str) {
        let (index, needs_newline) = self.after_content_line(line);
        self.insert_elements(index, needs_newline, |ending| {
            vec![self.entry_element(key, value, ending)]
        });
    }

    /// A new entry using the configured separator spacing.
    fn entry_element(&self, key: &str, value: &str, ending: &'static str) -> SyntaxNode {
        let (before, after) = spacing_for_new_entry(&self.editor.options.separator_spacing);
        let entry = green_builders::entry_node(key, value, before, after, ending);
        SyntaxNode::new_root(entry).clone_for_update()
    }

    /// Remove an entry by key name. Returns true if found and removed.
    #[must_use]
    pub fn remove_entry(&self, key: &str) -> bool {
        if let Some(entry) = self.find_entry(key) {
            detach_preserving_line_breaks(entry.syntax());
            true
        } else {
            false
        }
    }

    /// Rename a key, preserving its value and formatting.
    ///
    /// Returns `false` and makes **no change** if `old_key` is not found, or if
    /// `new_key` already exists on a different entry — renaming is refused
    /// rather than silently creating a duplicate key. Renaming a key to its
    /// current name is a successful no-op.
    ///
    /// For unconditional, position-targeted renaming that bypasses this guard
    /// (e.g. when intermediate states transiently collide), use
    /// [`entries_mut`](Self::entries_mut).
    #[must_use]
    pub fn rename_key(&self, old_key: &str, new_key: &str) -> bool {
        let Some(entry) = self.find_entry(old_key) else {
            return false;
        };
        // Refuse to create a duplicate key.
        if old_key != new_key && self.find_entry(new_key).is_some() {
            return false;
        }
        replace_key(entry.syntax(), new_key);
        true
    }

    /// Handles to each entry in this section, in document order, for
    /// position-targeted mutation.
    ///
    /// Because handles address entries by position rather than by key name,
    /// they stay unambiguous even when duplicate keys are present — useful for
    /// batch renames whose intermediate states would transiently collide.
    ///
    /// The returned handles are a snapshot taken when this is called. Mutating
    /// one (e.g. [`EntryEditor::set_key`]) does not invalidate the others, so
    /// it's safe to iterate the vector and mutate as you go.
    #[must_use]
    pub fn entries_mut(&self) -> Vec<EntryEditor> {
        self.node
            .children()
            .filter_map(Entry::cast)
            .enumerate()
            .map(|(index, entry)| EntryEditor {
                node: entry.syntax().clone(),
                index,
                separator_spacing: self.editor.options.separator_spacing.clone(),
            })
            .collect()
    }

    /// Remove this entire section (header + all entries).
    pub fn remove(self) {
        detach_preserving_line_breaks(&self.node);
    }

    /// Append raw text lines to this section's body.
    ///
    /// Lines are inserted **verbatim** — no parsing, no reformatting. The
    /// document's line ending (see [`Editor`]) is appended to each line that
    /// doesn't already end with `\n` or `\r`. Like
    /// [`append_entry`](Self::append_entry), lines land after the last content
    /// line and before any trailing blank lines. An empty slice is a no-op.
    pub fn append_raw_lines(&self, lines: &[&str]) {
        if lines.is_empty() {
            return;
        }
        let (index, needs_newline) = self.content_end();
        self.insert_elements(index, needs_newline, |ending| {
            Self::raw_line_elements(lines, ending)
        });
    }

    /// Insert raw text lines at a specific child index within this section.
    ///
    /// Index 0 is the section header. Lines are inserted **verbatim**.
    /// The document's line ending (see [`Editor`]) is appended to each line
    /// that doesn't already end with `\n` or `\r`.
    /// If `index` exceeds the number of children, lines are appended at the end.
    /// An unterminated preceding line is separated with a newline first.
    /// An empty slice is a no-op.
    pub fn insert_raw_lines_at(&self, index: usize, lines: &[&str]) {
        if lines.is_empty() {
            return;
        }
        let children: Vec<_> = self.node.children().collect();
        let index = index.min(children.len());
        let (index, needs_newline) = if index == 0 {
            (0, false)
        } else {
            Self::after_line(&children, index - 1)
        };
        self.insert_elements(index, needs_newline, |ending| {
            Self::raw_line_elements(lines, ending)
        });
    }

    /// Preserve physical-line indices when completing an unterminated line.
    /// `lines` builds the new lines with the document's line ending, which is
    /// read before any terminator is added.
    fn insert_elements(
        &self,
        index: usize,
        needs_newline: bool,
        lines: impl FnOnce(&'static str) -> Vec<SyntaxNode>,
    ) {
        let ending = document_line_ending(&self.node);
        let lines = lines(ending);
        if needs_newline {
            terminate_line(&self.node, index, ending);
        }
        // Repair internal boundaries while the new lines are still detached,
        // so each newline edit only rebuilds that line, not the document.
        for pair in lines.windows(2) {
            separate_line_breaks(last_token(&pair[0]), first_token(&pair[1]));
        }
        let end = index + lines.len();
        self.node
            .splice_children(index..index, lines.iter().cloned().map(Into::into));
        // Repair the two boundaries that depend on the surrounding document.
        repair_line_boundary(&self.node, index);
        repair_line_boundary(&self.node, end);
    }

    /// Build the verbatim child elements for a set of raw lines. Each line
    /// becomes a `COMMENT_LINE` node: its content is an opaque `COMMENT` token
    /// followed by its newline, keeping the line-node invariant intact.
    /// Unterminated lines receive the document's line `ending`.
    fn raw_line_elements(lines: &[&str], ending: &str) -> Vec<SyntaxNode> {
        let mut elements = Vec::new();
        for line in lines {
            let text = if line.ends_with('\n') || line.ends_with('\r') {
                (*line).to_string()
            } else {
                format!("{line}{ending}")
            };
            // Emit the line content (without newline) as a COMMENT token
            // and the newline separately, wrapped in a COMMENT_LINE node.
            // COMMENT is used as a generic "opaque text" kind — it preserves
            // the content verbatim.
            let (content, nl) = if let Some(content) = text.strip_suffix("\r\n") {
                (content, "\r\n")
            } else if let Some(content) = text.strip_suffix('\n') {
                (content, "\n")
            } else {
                let content = text
                    .strip_suffix('\r')
                    .expect("raw lines always have a terminator");
                (content, "\r")
            };

            let green = {
                let mut b = rowan::GreenNodeBuilder::new();
                b.start_node(SyntaxKind::COMMENT_LINE.into());
                b.token(SyntaxKind::COMMENT.into(), content);
                b.token(SyntaxKind::NEWLINE.into(), nl);
                b.finish_node();
                b.finish()
            };
            elements.push(SyntaxNode::new_root(green).clone_for_update());
        }
        elements
    }

    /// A standalone `NEWLINE` token element with the given terminator text.
    fn newline_element(text: &str) -> crate::SyntaxElement {
        let mut b = rowan::GreenNodeBuilder::new();
        b.start_node(SyntaxKind::ROOT.into());
        b.token(SyntaxKind::NEWLINE.into(), text);
        b.finish_node();
        let wrapper = SyntaxNode::new_root(b.finish()).clone_for_update();
        wrapper
            .children_with_tokens()
            .next()
            .expect("wrapper contains one newline token")
    }

    /// Insertion point at the end of the section's content: just after the
    /// last line that is not blank, but before any trailing blank lines.
    ///
    /// "Not blank" includes the header, entries, comments, and malformed
    /// (error) lines. Trailing blank lines usually separate this section from
    /// the next one, so new content goes above them.
    ///
    /// Returns `(child_index, needs_newline)`.
    fn content_end(&self) -> (usize, bool) {
        let children: Vec<SyntaxNode> = self.node.children().collect();
        // remove_lines can remove even the header. Error lines still count as
        // content; only blank-line nodes belong after an append.
        children
            .iter()
            .rposition(|line| line.kind() != SyntaxKind::BLANK_LINE)
            .map_or((0, false), |k| Self::after_line(&children, k))
    }

    /// Insertion point just after the `n`-th logical content line (1-based),
    /// where content lines are entries and comments (the header, blank lines,
    /// and malformed lines are not counted).
    ///
    /// - `n == 0` inserts right after the header, even with no content lines.
    /// - Any other `n` at or beyond the number of content lines returns the
    ///   same point as [`content_end`](Self::content_end), so the entry lands
    ///   exactly where [`append_entry`](Self::append_entry) would put it.
    ///
    /// Returns `(child_index, needs_newline)`.
    fn after_content_line(&self, n: usize) -> (usize, bool) {
        let children: Vec<SyntaxNode> = self.node.children().collect();
        let header = children
            .iter()
            .position(|line| line.kind() == SyntaxKind::SECTION_HEADER);
        let content: Vec<usize> = children
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line.kind(), SyntaxKind::ENTRY | SyntaxKind::COMMENT_LINE))
            .map(|(i, _)| i)
            .collect();

        if n == 0 {
            return header.map_or((0, false), |k| Self::after_line(&children, k));
        }
        // Past the last content line, `insert_entry_at_line` promises to act
        // exactly like `append_entry`. Inserting right after the last entry
        // or comment would break that promise whenever the section continues
        // with lines that are not counted as content, such as malformed
        // `=value` lines. `append_entry` keeps those lines above new content
        // (only trailing blank lines stay below). For `[s]\na=1\n=bad\n`:
        //
        //     append_entry                  -> a=1, =bad, n = 1
        //     stop after the last entry     -> a=1, n = 1, =bad   (wrong)
        //
        // Reusing `content_end` keeps both methods in agreement by
        // construction, including for a section with no content lines.
        if n >= content.len() {
            return self.content_end();
        }
        Self::after_line(&children, content[n - 1])
    }

    /// Compute the insertion point immediately after the line node at
    /// `children[k]`. In the line-node model every child of a section is a
    /// line node that owns its terminating newline, so this is simply the next
    /// index — with `needs_newline` set when the line has no terminator (end
    /// of file).
    fn after_line(children: &[SyntaxNode], k: usize) -> (usize, bool) {
        let ends_with_newline = children[k]
            .last_token()
            .is_some_and(|token| token.kind() == SyntaxKind::NEWLINE);
        (k + 1, !ends_with_newline)
    }

    /// Remove a range of child line nodes (0-indexed within this section).
    ///
    /// In the line-node model each physical line is one child, so index 0 is
    /// the section header and child index equals line index. Entries, comment
    /// lines, and blank lines each count as one element. Empty and reversed
    /// ranges are no-ops.
    pub fn remove_lines(&self, range: std::ops::Range<usize>) {
        if range.start >= range.end {
            return;
        }
        // Collect then detach — splice_children has issues with large ranges
        // in rowan's mutable tree (indices shift during removal).
        let to_remove: Vec<_> = self
            .node
            .children()
            .skip(range.start)
            .take(range.end - range.start)
            .collect();
        for line in to_remove {
            line.detach();
        }
        repair_line_boundary(&self.node, range.start);
    }

    fn find_entry(&self, key: &str) -> Option<Entry> {
        let section =
            Section::cast(self.node.clone()).expect("a SectionEditor always stores a SECTION node");
        section.entries().find(|e| e.key().as_deref() == Some(key))
    }
}

/// A CR terminator followed by an LF blank line must stay two line breaks.
/// Otherwise serialization joins them into CRLF and shifts later line indices.
fn separate_line_breaks(before: Option<crate::SyntaxToken>, after: Option<crate::SyntaxToken>) {
    if !before.is_some_and(|token| token.text().ends_with('\r')) {
        return;
    }
    let Some(after) = after
        .filter(|token| token.kind() == SyntaxKind::NEWLINE)
        .filter(|token| token.text() == "\n")
    else {
        return;
    };
    let parent = after.parent().expect("a newline token has a parent");
    let index = after.index();
    parent.splice_children(index..index + 1, [SectionEditor::newline_element("\r\n")]);
}

/// Repair the boundary before `parent`'s child at `index`. The neighboring
/// text can belong to another section, e.g. at a section's first line.
fn repair_line_boundary(parent: &SyntaxNode, index: usize) {
    separate_line_breaks(token_before(parent, index), token_after(parent, index));
}

/// The last token serialized before `parent`'s child at `index`.
fn token_before(parent: &SyntaxNode, index: usize) -> Option<crate::SyntaxToken> {
    let preceding: Vec<_> = parent.children_with_tokens().take(index).collect();
    preceding
        .into_iter()
        .rev()
        .find_map(|element| match element {
            rowan::NodeOrToken::Node(node) => last_token(&node),
            rowan::NodeOrToken::Token(token) => Some(token),
        })
        .or_else(|| {
            let grandparent = parent.parent()?;
            token_before(&grandparent, parent.index())
        })
}

/// The first nonempty token serialized from `parent`'s child at `index`.
/// Only a document's leading BOM is a loose token, and it never follows a
/// line, so only line nodes are searched.
fn token_after(parent: &SyntaxNode, index: usize) -> Option<crate::SyntaxToken> {
    parent
        .children()
        .skip_while(|line| line.index() < index)
        .find_map(|line| first_token(&line))
        .or_else(|| {
            let grandparent = parent.parent()?;
            token_after(&grandparent, parent.index() + 1)
        })
}

/// The first nonempty token of `node`. Inserted blank lines can have empty
/// content tokens before their newline.
fn first_token(node: &SyntaxNode) -> Option<crate::SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .find(|token| !token.text().is_empty())
}

fn detach_preserving_line_breaks(node: &SyntaxNode) {
    let parent = node.parent();
    let index = node.index();
    node.detach();
    if let Some(parent) = parent {
        repair_line_boundary(&parent, index);
    }
}

/// Complete the line before an insertion point with the document's line
/// `ending`. The line keeps ownership of its terminator so `remove_lines`
/// still addresses the same physical lines.
fn terminate_line(parent: &SyntaxNode, index: usize, ending: &'static str) {
    let line = parent
        .children_with_tokens()
        .nth(index - 1)
        .and_then(rowan::NodeOrToken::into_node)
        .expect("a missing terminator always belongs to a preceding line node");
    let previous_token = last_token(&line).expect("a line needing a terminator has content");
    let newline = SectionEditor::newline_element(green_builders::newline_after(
        previous_token.text(),
        ending,
    ));
    let end = line.children_with_tokens().count();
    line.splice_children(end..end, vec![newline]);
}

/// A handle to a specific entry within a section, for position-targeted
/// mutation that doesn't depend on key names.
///
/// Obtained from [`SectionEditor::entries_mut`]. Because it targets an entry by
/// identity rather than by key lookup, it operates unambiguously even when
/// several entries share a key, and its mutators (`set_key`/`set_value`) make
/// no duplicate-key checks — the caller is in control.
pub struct EntryEditor {
    node: SyntaxNode,
    index: usize,
    separator_spacing: SeparatorSpacing,
}

impl EntryEditor {
    /// This entry's 0-based position among its section's entries.
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    /// The current key text.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "EntryEditor instances can only be constructed from ENTRY nodes"
    )]
    pub fn key(&self) -> Option<String> {
        Entry::cast(self.node.clone())
            .expect("an EntryEditor always stores an ENTRY node")
            .key()
    }

    /// The current value text. Returns `None` for a bare key with no separator.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "EntryEditor instances can only be constructed from ENTRY nodes"
    )]
    pub fn value(&self) -> Option<String> {
        Entry::cast(self.node.clone())
            .expect("an EntryEditor always stores an ENTRY node")
            .value()
    }

    /// Replace this entry's key, preserving its value and formatting.
    ///
    /// Unlike [`SectionEditor::rename_key`], this targets exactly this entry
    /// and does **not** guard against creating duplicate keys.
    pub fn set_key(&self, new_key: &str) {
        replace_key(&self.node, new_key);
    }

    /// Replace this entry's value, preserving its key and applying the
    /// editor's configured separator spacing. An empty string clears the
    /// value (`key =` with the default options).
    /// Inline comments move to a preceding comment line when the replacement
    /// ends on a blank physical line.
    pub fn set_value(&self, value: &str) {
        replace_value(&self.node, value, &self.separator_spacing);
    }

    /// Remove this entry from its section.
    pub fn remove(self) {
        detach_preserving_line_breaks(&self.node);
    }
}

/// Replace the key identifier of an `ENTRY` node in place.
fn replace_key(entry: &SyntaxNode, new_key: &str) {
    let key_node = Entry::cast(entry.clone())
        .and_then(|e| e.key_node())
        .expect("an ENTRY always has a KEY node");
    let key_syntax = key_node.syntax().clone();
    let old_count = key_syntax.children_with_tokens().count();
    let fresh = SyntaxNode::new_root(green_builders::key_node(new_key)).clone_for_update();
    let new_children: Vec<crate::SyntaxElement> = fresh.children_with_tokens().collect();
    key_syntax.splice_children(0..old_count, new_children);
}

/// Replace the value of an `ENTRY` node in place. An empty string clears it.
fn replace_value(entry: &SyntaxNode, value: &str, spacing: &SeparatorSpacing) {
    let entry = Entry::cast(entry.clone()).expect("an ENTRY node can be cast to Entry");
    let separator_kind = entry
        .syntax()
        .children_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .find_map(|token| {
            matches!(token.kind(), SyntaxKind::EQ | SyntaxKind::COLON).then_some(token.kind())
        });

    if separator_kind.is_none() || !matches!(spacing, SeparatorSpacing::Preserve) {
        let children: Vec<_> = entry.syntax().children_with_tokens().collect();
        let key_index = children
            .iter()
            .position(|element| {
                element
                    .as_node()
                    .is_some_and(|node| node.kind() == SyntaxKind::KEY)
            })
            .expect("an ENTRY always has a KEY node");
        let value_index = children
            .iter()
            .position(|element| {
                element
                    .as_node()
                    .is_some_and(|node| node.kind() == SyntaxKind::VALUE)
            })
            .expect("an ENTRY always has a VALUE node");
        let (before, after) = spacing_for_new_entry(spacing);
        let separator_index = key_index + 1;
        for element in &children[separator_index..value_index] {
            element
                .as_token()
                .expect("only separator tokens occur between KEY and VALUE nodes")
                .detach();
        }
        entry.syntax().splice_children(
            separator_index..separator_index,
            separator_elements(separator_kind.unwrap_or(SyntaxKind::EQ), before, after),
        );
    }

    let value_node = entry
        .value_node()
        .expect("an ENTRY always has a VALUE node");
    let value_syntax = value_node.syntax().clone();
    let old_count = value_syntax.children_with_tokens().count();
    let new_children: Vec<crate::SyntaxElement> = if value.is_empty() {
        vec![]
    } else {
        SyntaxNode::new_root(green_builders::value_node(value))
            .clone_for_update()
            .children_with_tokens()
            .collect()
    };
    value_syntax.splice_children(0..old_count, new_children);
    if value
        .rsplit(['\n', '\r'])
        .next()
        .is_some_and(|line| line.trim_matches([' ', '\t']).is_empty())
    {
        move_inline_comment_before(entry.syntax());
    }
    if value.is_empty() && entry.inline_comment().is_none() {
        normalize_empty_value_whitespace(entry.syntax(), &value_syntax, spacing);
    }
    preserve_value_carriage_return(entry.syntax(), &value_syntax, value);
}

/// A raw continued value can end in CR. An immediately following LF would
/// become part of that CRLF continuation, changing the value and potentially
/// consuming the next entry. Use a separate CRLF terminator in that case.
fn preserve_value_carriage_return(entry: &SyntaxNode, value_node: &SyntaxNode, value: &str) {
    if !value.ends_with('\r') {
        return;
    }
    let Some(newline) = value_node
        .next_sibling_or_token()
        .and_then(rowan::NodeOrToken::into_token)
        .filter(|token| token.kind() == SyntaxKind::NEWLINE)
        .filter(|token| token.text() == "\n")
    else {
        return;
    };
    let index = newline.index();
    entry.splice_children(index..index + 1, [SectionEditor::newline_element("\r\n")]);
}

/// With no value text, all whitespace after the separator belongs to its gap.
/// Keep one token there in Preserve mode, matching the parser. Compact/Exact
/// already set their gap above and must discard the old value's trailing space.
fn normalize_empty_value_whitespace(
    entry: &SyntaxNode,
    value: &SyntaxNode,
    spacing: &SeparatorSpacing,
) {
    let Some(trailing) = value
        .next_sibling_or_token()
        .and_then(rowan::NodeOrToken::into_token)
        .filter(|token| token.kind() == SyntaxKind::WHITESPACE)
    else {
        return;
    };
    let trailing_text = trailing.text().to_owned();
    trailing.detach();
    if !matches!(spacing, SeparatorSpacing::Preserve) {
        return;
    }

    let mut gap = String::new();
    if let Some(leading) = value
        .prev_sibling_or_token()
        .and_then(rowan::NodeOrToken::into_token)
        .filter(|token| token.kind() == SyntaxKind::WHITESPACE)
    {
        gap.push_str(leading.text());
        leading.detach();
    }
    gap.push_str(&trailing_text);
    let mut builder = rowan::GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::ROOT.into());
    builder.token(SyntaxKind::WHITESPACE.into(), &gap);
    builder.finish_node();
    let wrapper = SyntaxNode::new_root(builder.finish()).clone_for_update();
    let index = value.index();
    entry.splice_children(index..index, wrapper.children_with_tokens());
}

/// A marker at the start of a value or its final continued line is literal.
/// Preserve the note on its own line when that final line is blank, instead of
/// serializing a line whose comment would become value text on the next parse.
fn move_inline_comment_before(entry: &SyntaxNode) {
    let Some(parent) = entry.parent() else {
        return; // an entry handle may outlive removal from the document
    };
    let children: Vec<_> = entry.children_with_tokens().collect();
    let Some(comment_index) = children
        .iter()
        .position(|element| element.kind() == SyntaxKind::COMMENT)
    else {
        return;
    };
    // Inline comments always follow the VALUE and a whitespace token.
    let first = comment_index - 1;
    let mut builder = rowan::GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::COMMENT_LINE.into());
    for element in &children[first..=comment_index] {
        let token = element
            .as_token()
            .expect("inline comment trivia is tokenized");
        builder.token(token.kind().into(), token.text());
    }
    let newline = entry.last_token().expect("an inline comment is nonempty");
    if newline.kind() == SyntaxKind::NEWLINE {
        builder.token(SyntaxKind::NEWLINE.into(), newline.text());
    } else {
        builder.token(SyntaxKind::NEWLINE.into(), document_line_ending(&parent));
    }
    builder.finish_node();
    let comment = SyntaxNode::new_root(builder.finish()).clone_for_update();
    for element in &children[first..=comment_index] {
        element
            .as_token()
            .expect("inline comment trivia is tokenized")
            .detach();
    }
    let index = entry.index();
    parent.splice_children(index..index, vec![comment.into()]);
}

fn spacing_for_new_entry(spacing: &SeparatorSpacing) -> (&str, &str) {
    match spacing {
        SeparatorSpacing::Preserve => (" ", " "),
        SeparatorSpacing::Compact => ("", ""),
        SeparatorSpacing::Exact { before, after } => (before, after),
    }
}

fn separator_elements(
    separator: SyntaxKind,
    before: &str,
    after: &str,
) -> Vec<crate::SyntaxElement> {
    let mut builder = rowan::GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::ROOT.into());
    if !before.is_empty() {
        builder.token(SyntaxKind::WHITESPACE.into(), before);
    }
    let separator_text = if separator == SyntaxKind::COLON {
        ":"
    } else {
        "="
    };
    builder.token(separator.into(), separator_text);
    if !after.is_empty() {
        builder.token(SyntaxKind::WHITESPACE.into(), after);
    }
    builder.finish_node();
    SyntaxNode::new_root(builder.finish())
        .clone_for_update()
        .children_with_tokens()
        .collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn deleting_lines_keeps_carriage_returns_separate_from_blank_lines() {
        let source = "[s]\ra=old\rb=remove\n\nc=keep\n";
        for operation in 0..3 {
            let direct = Editor::new(source);
            let snapshot = direct.file();
            let retained = direct.section("s").entries_mut().remove(2);
            match operation {
                0 => assert!(direct.section("s").remove_entry("b")),
                1 => direct.section("s").entries_mut().remove(1).remove(),
                _ => direct.section("s").remove_lines(2..3),
            }
            assert_eq!(direct.finish(), "[s]\ra=old\r\r\nc=keep\n");
            let reloaded = Editor::new(&direct.finish());
            assert_eq!(direct.root.green(), reloaded.root.green());
            assert_eq!(snapshot.syntax().text().to_string(), source);
            assert_eq!(retained.value().as_deref(), Some("keep"));

            // The remaining blank line must have the same index whether or
            // not the caller reopened the file after deleting the entry.
            direct.section("s").remove_lines(2..3);
            reloaded.section("s").remove_lines(2..3);
            assert_eq!(direct.finish(), reloaded.finish());
            assert_eq!(direct.finish(), "[s]\ra=old\rc=keep\n");
            assert_eq!(retained.value().as_deref(), Some("keep"));
        }
    }

    #[test]
    fn creating_a_section_after_cr_retains_a_blank_separator_line() {
        for (source, expected) in [
            ("[s]\rk=v\r", "[s]\rk=v\r\r[next]\r"),
            ("[s]\nk=v\n", "[s]\nk=v\n\n[next]\n"),
            ("[s]\r\nk=v\r\n", "[s]\r\nk=v\r\n\r\n[next]\r\n"),
            // An LF blank after a final CR line must not join it as CRLF.
            ("[s]\nk=v\r", "[s]\nk=v\r\r\n[next]\n"),
        ] {
            let editor = Editor::new(source);
            let _ = editor.section("next");
            assert_eq!(editor.finish(), expected);
            let reloaded = Editor::new(&editor.finish());
            assert_eq!(editor.root.green(), reloaded.root.green());
        }
    }

    #[test]
    fn deletion_preserves_mixed_newline_styles_and_indented_blanks() {
        for before in ["\r", "\n", "\r\n"] {
            for blank in ["\r", "\n", "\r\n"] {
                for indent in ["", "\t", " "] {
                    let source = format!("[s]\na=old{before}x=remove\n{indent}{blank}c=keep\n");
                    let editor = Editor::new(&source);
                    assert!(editor.section("s").remove_entry("x"));
                    let blank = if before == "\r" && indent.is_empty() && blank == "\n" {
                        "\r\n"
                    } else {
                        blank
                    };
                    assert_eq!(
                        editor.finish(),
                        format!("[s]\na=old{before}{indent}{blank}c=keep\n")
                    );
                    assert_eq!(
                        editor.root.green(),
                        Editor::new(&editor.finish()).root.green()
                    );
                }
            }
        }
    }

    #[test]
    fn deleting_before_raw_blank_lines_preserves_physical_lines() {
        for before in ["\r", "\n", "\r\n"] {
            for raw in ["", "\n", "\r", "\r\n"] {
                for operation in 0..3 {
                    let editor = Editor::new(&format!("[s]{before}x=remove\n"));
                    editor.section("s").append_raw_lines(&[raw]);
                    editor.section("s").append_entry("keep", "2");
                    match operation {
                        0 => assert!(editor.section("s").remove_entry("x")),
                        1 => editor.section("s").entries_mut().remove(0).remove(),
                        _ => editor.section("s").remove_lines(1..2),
                    }
                    // New lines use the document's first line ending.
                    let ending = if raw.is_empty() { before } else { raw };
                    let ending = if before == "\r" && ending == "\n" {
                        "\r\n"
                    } else {
                        ending
                    };
                    assert_eq!(
                        editor.finish(),
                        format!("[s]{before}{ending}keep = 2{before}")
                    );
                    let reopened = Editor::new(&editor.finish());
                    editor.section("s").remove_lines(1..2);
                    reopened.section("s").remove_lines(1..2);
                    assert_eq!(editor.finish(), reopened.finish());
                    assert_eq!(editor.finish(), format!("[s]{before}keep = 2{before}"));
                }
            }
        }
    }

    #[test]
    fn deletion_boundaries_cover_error_lines_and_opaque_raw_lines() {
        let editor = Editor::new("[s]\n=bad\rx=remove\n\nnext=keep\n");
        assert!(editor.section("s").remove_entry("x"));
        assert_eq!(editor.finish(), "[s]\n=bad\r\r\nnext=keep\n");
        assert_eq!(
            editor.root.green(),
            Editor::new(&editor.finish()).root.green()
        );

        let editor = Editor::new("[s]\r=bad\n\nnext=keep\n");
        editor.section("s").remove_lines(1..2); // remove the whole error line
        assert_eq!(editor.finish(), "[s]\r\r\nnext=keep\n");

        // Raw content is deliberately opaque and must not be reclassified as
        // a NEWLINE token, even when its bytes happen to be a single LF.
        let editor = Editor::new("[s]\rx=remove\n");
        editor.section("s").append_raw_lines(&["\n\n"]);
        assert!(editor.section("s").remove_entry("x"));
        assert_eq!(editor.finish(), "[s]\r\n\n");
    }

    #[test]
    fn deletion_boundaries_allow_detached_handles_and_empty_sections() {
        let editor = Editor::new("[s]\rx=remove\n\n");
        let first = editor.section("s").entries_mut().remove(0);
        let second = editor.section("s").entries_mut().remove(0);
        first.remove();
        second.remove();
        assert_eq!(editor.finish(), "[s]\r\r\n");
        editor.section("s").remove_lines(999..1000);
        assert_eq!(editor.finish(), "[s]\r\r\n");

        let editor = Editor::new("[a]\r[b]\nx=1\n[c]\n");
        editor.section("c").remove_lines(0..usize::MAX);
        editor.section("b").remove(); // the following section is empty
        assert_eq!(editor.finish(), "[a]\r");
        let _ = editor.section("next");
        assert_eq!(editor.finish(), "[a]\r\r[next]\r");

        let editor = Editor::new("[a]\n[b]\nx=1\n");
        editor.section("a").remove_lines(0..usize::MAX);
        editor.section("b").remove(); // the preceding section is empty
        assert_eq!(editor.finish(), "");

        let editor = Editor::new("[a]\r[b]\nx=remove\n[c]\n\nkeep=2\n");
        let tail = editor.section("c");
        tail.remove_lines(0..1);
        editor.section("b").remove(); // the following section starts with a blank
        assert_eq!(editor.finish(), "[a]\r\r\nkeep=2\n");
        tail.remove_lines(0..1);
        assert_eq!(editor.finish(), "[a]\rkeep=2\n");
    }

    fn assert_reopens_identically(editor: &Editor) {
        assert_eq!(
            editor.root.green(),
            Editor::new(&editor.finish()).root.green()
        );
    }

    #[test]
    fn assigning_a_key_followed_by_junk_replaces_the_junk() {
        let spacings = [
            (SeparatorSpacing::Preserve, " = "),
            (SeparatorSpacing::Compact, "="),
            (SeparatorSpacing::exact("\t", "  "), "\t=  "),
        ];
        for (key, junk) in [
            ("key", " junk"),
            ("key", "\t\tjunk"),
            ("my", " key = value"),
        ] {
            for ending in ["\n", "\r\n", "\r", ""] {
                for (spacing, separator) in &spacings {
                    for (operation, value) in (0..4).zip(["v", "v", "", ""]) {
                        let options = EditOptions {
                            separator_spacing: spacing.clone(),
                        };
                        let source = format!("[s]\n{key}{junk}{ending}");
                        let editor = Editor::with_edit_options(&source, &options);
                        let snapshot = editor.file();
                        if operation % 2 == 0 {
                            editor.section("s").set(key, value);
                        } else {
                            editor.section("s").entries_mut()[0].set_value(value);
                        }
                        assert_eq!(
                            editor.finish(),
                            format!("[s]\n{key}{separator}{value}{ending}"),
                            "{source:?}"
                        );
                        assert_reopens_identically(&editor);
                        assert_eq!(snapshot.syntax().text().to_string(), source);
                    }
                }
            }
        }
    }

    #[test]
    fn removing_a_key_followed_by_junk_removes_its_whole_line() {
        for line in ["key junk", "key\t\tjunk", "key = ok"] {
            for operation in 0..2 {
                let editor = Editor::new(&format!("[s]\n{line}\nnext=1\n"));
                if operation == 0 {
                    assert!(editor.section("s").remove_entry("key"));
                } else {
                    editor.section("s").entries_mut().remove(0).remove();
                }
                assert_eq!(editor.finish(), "[s]\nnext=1\n");
                let keys: Vec<_> = editor
                    .file()
                    .sections()
                    .flat_map(|section| {
                        section
                            .entries()
                            .filter_map(|entry| entry.key())
                            .collect::<Vec<_>>()
                    })
                    .collect();
                assert_eq!(keys, ["next"]);
                assert_reopens_identically(&editor);
            }
        }
    }

    #[test]
    fn line_indices_count_each_malformed_line_once() {
        for line in ["=bad", "  =bad", ":", "key junk", "my key = value"] {
            for ending in ["\n", "\r\n", "\r"] {
                let source = format!("[s]{ending}{line}{ending}b=2{ending}c=3{ending}");

                let editor = Editor::new(&source);
                editor.section("s").remove_lines(2..3);
                assert_eq!(
                    editor.finish(),
                    format!("[s]{ending}{line}{ending}c=3{ending}")
                );
                assert_reopens_identically(&editor);

                let editor = Editor::new(&source);
                editor.section("s").remove_lines(1..2);
                assert_eq!(
                    editor.finish(),
                    format!("[s]{ending}b=2{ending}c=3{ending}")
                );
                assert_reopens_identically(&editor);

                for (index, expected) in [
                    (
                        2,
                        format!("[s]{ending}{line}{ending}; note{ending}b=2{ending}c=3{ending}"),
                    ),
                    (
                        3,
                        format!("[s]{ending}{line}{ending}b=2{ending}; note{ending}c=3{ending}"),
                    ),
                ] {
                    let editor = Editor::new(&source);
                    editor.section("s").insert_raw_lines_at(index, &["; note"]);
                    assert_eq!(editor.finish(), expected);
                    assert_reopens_identically(&editor);
                }

                let editor = Editor::new(&format!("[s]{ending}{line}"));
                editor.section("s").append_entry("n", "1");
                assert_eq!(
                    editor.finish(),
                    format!("[s]{ending}{line}{ending}n = 1{ending}")
                );
                editor.section("s").append_raw_lines(&["; note"]);
                editor.section("s").remove_lines(2..3);
                assert_eq!(
                    editor.finish(),
                    format!("[s]{ending}{line}{ending}; note{ending}")
                );
                assert_reopens_identically(&editor);
            }
        }
    }

    #[test]
    fn a_new_section_is_separated_from_a_final_malformed_line() {
        for line in ["=bad", "  =bad", "key junk"] {
            for ending in ["", "\n"] {
                for prefix in ["", "[s]\nk=v\n\n"] {
                    let editor = Editor::new(&format!("{prefix}{line}{ending}"));
                    let _ = editor.section("next");
                    assert_eq!(editor.finish(), format!("{prefix}{line}\n\n[next]\n"));
                    assert_reopens_identically(&editor);
                }
            }
        }
    }

    /// A raw line with the terminator the raw-line APIs append in a document
    /// whose first line ends with `ending`.
    fn terminated(raw: &str, ending: &str) -> String {
        if raw.ends_with(['\n', '\r']) {
            raw.to_owned()
        } else {
            format!("{raw}{ending}")
        }
    }

    #[test]
    fn inserted_lines_keep_a_preceding_carriage_return_separate() {
        for before in ["\r", "\n", "\r\n"] {
            for raw in ["", "\n", "\r", "\r\n", "x\r", "x"] {
                for operation in 0..3 {
                    let source = format!("[s]{before}k=v{before}");
                    let editor = Editor::new(&source);
                    let snapshot = editor.file();
                    let section = editor.section("s");
                    match operation {
                        0 => section.append_raw_lines(&[raw]),
                        1 => section.insert_raw_lines_at(2, &[raw]),
                        _ => section.insert_raw_lines_at(usize::MAX, &[raw]),
                    }
                    section.append_entry("n", "1");
                    let mut line = terminated(raw, before);
                    if before == "\r" && line.starts_with('\n') {
                        line.insert(0, '\r');
                    }
                    assert_eq!(editor.finish(), format!("{source}{line}n = 1{before}"));
                    assert_eq!(snapshot.syntax().text().to_string(), source);

                    // The inserted line must have the same index whether or
                    // not the caller reopened the file after inserting it.
                    let reopened = Editor::new(&editor.finish());
                    editor.section("s").remove_lines(2..3);
                    reopened.section("s").remove_lines(2..3);
                    assert_eq!(editor.finish(), reopened.finish());
                    assert_eq!(editor.finish(), format!("{source}n = 1{before}"));
                }
            }
        }
    }

    #[test]
    fn inserted_carriage_returns_keep_following_blank_lines_separate() {
        for raw in ["", "\n", "\r", "\r\n", "x\r", "x"] {
            for blank in ["\n", "\r", "\r\n", " \n"] {
                for operation in 0..2 {
                    let source = format!("[s]\nk=v\n{blank}[t]\nnext=1\n");
                    let editor = Editor::new(&source);
                    let snapshot = editor.file();
                    let retained = editor.section("t").entries_mut().remove(0);
                    let section = editor.section("s");
                    match operation {
                        // Both insert after k=v, before the trailing blank.
                        0 => section.append_raw_lines(&[raw]),
                        _ => section.insert_raw_lines_at(2, &[raw]),
                    }
                    let line = terminated(raw, "\n");
                    let blank = if line.ends_with('\r') && blank.starts_with('\n') {
                        format!("\r{blank}")
                    } else {
                        blank.to_owned()
                    };
                    assert_eq!(
                        editor.finish(),
                        format!("[s]\nk=v\n{line}{blank}[t]\nnext=1\n")
                    );
                    assert_eq!(snapshot.syntax().text().to_string(), source);

                    let reopened = Editor::new(&editor.finish());
                    editor.section("s").remove_lines(3..4);
                    reopened.section("s").remove_lines(3..4);
                    assert_eq!(editor.finish(), reopened.finish());
                    assert_eq!(editor.finish(), format!("[s]\nk=v\n{line}[t]\nnext=1\n"));
                    retained.set_value("2");
                    assert!(editor.finish().ends_with("[t]\nnext=2\n"));
                }
            }
        }
    }

    #[test]
    fn lines_inserted_together_keep_their_shared_boundaries() {
        // Each pair joins a bare CR to a following LF unless repaired.
        let pairs = [
            (vec!["; raw\r", ""], "; raw\r\r\n"),
            (vec!["\r", "\n"], "\r\r\n"),
            (vec!["x\r", "", ""], "x\r\r\n\n"),
            (vec!["x\r", "\r", ""], "x\r\r\r\n"),
            (vec!["", "x\r", "\n", "y"], "\nx\r\r\ny\n"),
        ];
        for (raw, inserted) in pairs {
            let count = raw.len();
            for operation in 0..3 {
                let source = "[s]\nk=v\n\nlast=1\n";
                let live = Editor::new(source);
                let snapshot = live.file();
                let section = live.section("s");
                match operation {
                    0 => section.append_raw_lines(&raw),
                    1 => section.insert_raw_lines_at(2, &raw),
                    _ => section.insert_raw_lines_at(1, &raw),
                }
                let (expected, first) = match operation {
                    0 => (format!("{source}{inserted}"), 4),
                    1 => (format!("[s]\nk=v\n{inserted}\nlast=1\n"), 2),
                    _ => (format!("[s]\n{inserted}k=v\n\nlast=1\n"), 1),
                };
                // Raw lines stay opaque, so compare text and line indices
                // rather than trees.
                assert_eq!(live.finish(), expected, "{raw:?}");
                assert_eq!(snapshot.syntax().text().to_string(), source);

                // Every inserted line keeps its index after reopening.
                for line in first..first + count {
                    let live = Editor::new(source);
                    match operation {
                        0 => live.section("s").append_raw_lines(&raw),
                        1 => live.section("s").insert_raw_lines_at(2, &raw),
                        _ => live.section("s").insert_raw_lines_at(1, &raw),
                    }
                    let reopened = Editor::new(&live.finish());
                    live.section("s").remove_lines(line..line + 1);
                    reopened.section("s").remove_lines(line..line + 1);
                    assert_eq!(live.finish(), reopened.finish(), "{raw:?} line {line}");
                }
            }
        }

        // The reported reproduction, followed by a parsed entry.
        let live = Editor::new("[s]\nk=v\n");
        live.section("s").append_raw_lines(&["; raw\r", ""]);
        live.section("s").append_entry("n", "1");
        let reopened = Editor::new(&live.finish());
        live.section("s").remove_lines(3..4);
        reopened.section("s").remove_lines(3..4);
        assert_eq!(live.finish(), reopened.finish());
        assert_eq!(live.finish(), "[s]\nk=v\n; raw\rn = 1\n");
    }

    #[test]
    fn lines_inserted_together_in_a_cr_document_stay_separate() {
        // Unterminated raw lines receive CR here, so a following raw LF must
        // not join it, whichever line in the run it follows.
        for (raw, inserted) in [
            (vec!["x", "\n"], "x\r\r\n"),
            (vec!["", "\n", "y"], "\r\r\ny\r"),
            (vec!["x", "", "\n"], "x\r\r\r\n"),
        ] {
            for line in 2..2 + raw.len() {
                let live = Editor::new("[s]\rk=v\r");
                live.section("s").append_raw_lines(&raw);
                live.section("s").append_entry("n", "1");
                assert_eq!(live.finish(), format!("[s]\rk=v\r{inserted}n = 1\r"));
                let reopened = Editor::new(&live.finish());
                live.section("s").remove_lines(line..line + 1);
                reopened.section("s").remove_lines(line..line + 1);
                assert_eq!(live.finish(), reopened.finish(), "{raw:?} line {line}");
            }
        }
    }

    #[test]
    fn line_boundaries_are_repaired_across_sections() {
        // Lines inserted before a header follow the previous section's line.
        let editor = Editor::new("[a]\nk=v\r[b]\nx=1\n");
        editor.section("b").insert_raw_lines_at(0, &[""]);
        assert_eq!(editor.finish(), "[a]\nk=v\r\r\n[b]\nx=1\n");
        let reopened = Editor::new(&editor.finish());
        editor.section("b").remove_lines(0..1);
        reopened.section("a").remove_lines(2..3);
        assert_eq!(editor.finish(), reopened.finish());
        assert_eq!(editor.finish(), "[a]\nk=v\r[b]\nx=1\n");

        // Removing a header exposes the section's first line to the previous
        // section's terminator.
        let editor = Editor::new("[a]\r[b]\n\nx=1\n");
        let headless = editor.section("b");
        headless.remove_lines(0..1);
        assert_eq!(editor.finish(), "[a]\r\r\nx=1\n");
        let reopened = Editor::new(&editor.finish());
        headless.remove_lines(0..1);
        reopened.section("a").remove_lines(1..2);
        assert_eq!(editor.finish(), reopened.finish());
        assert_eq!(editor.finish(), "[a]\rx=1\n");

        // An appended CR can precede the first line of a headless section.
        let editor = Editor::new("[a]\nk=v\n[b]\n\nx=1\n");
        let headless = editor.section("b");
        headless.remove_lines(0..1);
        editor.section("a").append_raw_lines(&["r\r"]);
        assert_eq!(editor.finish(), "[a]\nk=v\nr\r\r\nx=1\n");
        let reopened = Editor::new(&editor.finish());
        headless.remove_lines(0..1);
        reopened.section("a").remove_lines(3..4);
        assert_eq!(editor.finish(), reopened.finish());
        assert_eq!(editor.finish(), "[a]\nk=v\nr\rx=1\n");

        // Detached sections and the document edges have no neighbors.
        let editor = Editor::new("[a]\rx=1\n");
        let detached = editor.section("a");
        editor.section("a").remove();
        detached.insert_raw_lines_at(0, &[""]);
        detached.append_raw_lines(&["r\r"]);
        assert_eq!(editor.finish(), "");
        assert_eq!(detached.node.text().to_string(), "\r[a]\rx=1\nr\r");
    }

    fn assert_reopens_identically_with(editor: &Editor, options: &ParseOptions) {
        let reopened = Editor::with_parse_options(&editor.finish(), options);
        assert_eq!(editor.root.green(), reopened.root.green());
    }

    #[test]
    fn inserted_lines_use_the_documents_first_line_ending() {
        let options = ParseOptions::default();
        // (first line ending, later line endings, expected ending)
        let endings = [
            ("\n", "\n", "\n"),
            ("\r\n", "\r\n", "\r\n"),
            ("\r", "\r", "\r"),
            // Mixed documents follow their first line.
            ("\r\n", "\n", "\r\n"),
            ("\r", "\r\n", "\r"),
            ("\n", "\r\n", "\n"),
        ];
        for (first, rest, e) in endings {
            for terminated in [true, false] {
                let last = if terminated { rest } else { "" };
                let source = format!("[s]{first}a=1{rest}k=v{last}");
                let body = format!("[s]{first}a=1{rest}");
                // An unterminated final line is completed with the same ending.
                let tail = format!("{body}k=v{}", if terminated { rest } else { e });
                for operation in 0..8 {
                    let editor = Editor::new(&source);
                    let snapshot = editor.file();
                    let expected = match operation {
                        0 => {
                            editor.section("s").set("n", "1");
                            format!("{tail}n = 1{e}")
                        }
                        1 => {
                            editor.section("s").append_entry("n", "1");
                            format!("{tail}n = 1{e}")
                        }
                        2 => {
                            editor.section("s").insert_entry_at_line(0, "n", "1");
                            format!("[s]{first}n = 1{e}a=1{rest}k=v{last}")
                        }
                        3 => {
                            editor.section("s").append_raw_lines(&["; raw", "# x"]);
                            format!("{tail}; raw{e}# x{e}")
                        }
                        4 => {
                            editor.section("s").insert_raw_lines_at(2, &["; raw"]);
                            format!("{body}; raw{e}k=v{last}")
                        }
                        5 => {
                            let _ = editor.section("t");
                            format!("{tail}{e}[t]{e}")
                        }
                        6 => {
                            editor.section("t").set("n", "1");
                            format!("{tail}{e}[t]{e}n = 1{e}")
                        }
                        _ => {
                            editor.section("s").remove_lines(1..2);
                            editor.section("s").insert_entry_at_line(1, "n", "1");
                            let tail = if terminated { rest } else { e };
                            format!("[s]{first}k=v{tail}n = 1{e}")
                        }
                    };
                    assert_eq!(editor.finish(), expected, "{source:?} #{operation}");
                    assert_reopens_identically_with(&editor, &options);
                    assert_eq!(snapshot.syntax().text().to_string(), source);
                }
            }
        }
    }

    #[test]
    fn documents_without_a_line_ending_receive_lf_lines() {
        let options = ParseOptions::default();
        for (source, expected) in [
            ("", "[t]\nk = v\n"),
            ("\u{FEFF}", "\u{FEFF}[t]\nk = v\n"),
            ("[s]", "[s]\n\n[t]\nk = v\n"),
            ("k=v", "k=v\n\n[t]\nk = v\n"),
        ] {
            let editor = Editor::new(source);
            editor.section("t").set("k", "v");
            assert_eq!(editor.finish(), expected);
            assert_reopens_identically_with(&editor, &options);
        }
    }

    #[test]
    fn a_continued_values_line_break_is_the_documents_first_line_ending() {
        let options = ParseOptions::default();
        for (source, expected) in [
            // The first physical line break is inside the continued value.
            ("[s]k=a \\\rb\nz=1\n", "\r"),
            ("[s]k=a \\\r\nb\nz=1\n", "\r\n"),
            ("[s]k=a \\\nb\r\nz=1\r\n", "\n"),
        ] {
            let source = source.replacen("[s]", "", 1);
            let editor = Editor::new(&source);
            let _ = editor.section("t");
            assert_eq!(editor.finish(), format!("{source}{expected}[t]{expected}"));
            assert_reopens_identically_with(&editor, &options);
        }
    }

    #[test]
    fn the_line_ending_is_read_as_the_lexer_reads_it() {
        // Opaque raw text can start with LF right after a CR line: the text
        // then begins with CRLF, so a reopened copy terminates lines alike.
        let editor = Editor::new("[s]\r");
        editor.section("s").append_raw_lines(&["\n; raw\n"]);
        let reopened = Editor::new(&editor.finish());
        for editor in [&editor, &reopened] {
            editor.section("s").append_entry("n", "1");
        }
        assert_eq!(editor.finish(), "[s]\r\n; raw\nn = 1\r\n");
        assert_eq!(editor.finish(), reopened.finish());

        // Empty raw content between a CR and the next line break is skipped.
        // (Raw lines stay opaque, so this one is content rather than blank.)
        let editor = Editor::new("[s]\r");
        editor.section("s").append_raw_lines(&[""]);
        editor.section("s").append_entry("n", "1");
        assert_eq!(editor.finish(), "[s]\r\rn = 1\r");
    }

    #[test]
    fn the_line_ending_follows_the_current_first_line() {
        let options = ParseOptions::default();
        // Removing the CRLF lines leaves an LF document. The live editor and a
        // reopened copy must keep terminating new lines identically.
        let editor = Editor::new("[a]\r\nx=1\r\n[b]\nk=v\n");
        editor.section("a").remove();
        let reopened = Editor::new(&editor.finish());
        for editor in [&editor, &reopened] {
            editor.section("b").append_entry("n", "1");
            let _ = editor.section("c");
        }
        assert_eq!(editor.finish(), "[b]\nk=v\nn = 1\n\n[c]\n");
        assert_eq!(editor.finish(), reopened.finish());
        assert_reopens_identically_with(&editor, &options);

        // A detached section is terminated like its own text.
        let editor = Editor::new("[a]\nx=1\n[b]\r\ny=2");
        let detached = editor.section("b");
        editor.section("b").remove();
        detached.append_entry("n", "1");
        assert_eq!(detached.node.text().to_string(), "[b]\r\ny=2\r\nn = 1\r\n");
        assert_eq!(editor.finish(), "[a]\nx=1\n");
    }

    #[test]
    fn a_moved_inline_comment_uses_the_documents_line_ending() {
        let options = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        for ending in ["\n", "\r\n", "\r"] {
            for operation in 0..2 {
                let source = format!("[s]{ending}k = v ; note");
                let editor = Editor::with_parse_options(&source, &options);
                if operation == 0 {
                    editor.section("s").set("k", "");
                } else {
                    editor.section("s").entries_mut()[0].set_value("");
                }
                assert_eq!(
                    editor.finish(),
                    format!("[s]{ending} ; note{ending}k = "),
                    "{source:?}"
                );
                assert_reopens_identically_with(&editor, &options);
            }
        }
    }

    #[test]
    fn editing_a_bare_key_keeps_its_inline_comment() {
        let parse_options = ParseOptions {
            allow_no_value: true,
            inline_comments: true,
        };
        let spacings = [
            (SeparatorSpacing::Preserve, " = "),
            (SeparatorSpacing::Compact, "="),
            (SeparatorSpacing::exact("\t", "  "), "\t=  "),
        ];
        for (gap, comment) in [(" ", "; note"), ("\t", "#"), (" \t ", "; λ")] {
            for ending in ["\n", "\r\n", "\r", ""] {
                for (spacing, separator) in &spacings {
                    for operation in 0..4 {
                        let edit_options = EditOptions {
                            separator_spacing: spacing.clone(),
                        };
                        let source = format!("[s]\nflag{gap}{comment}{ending}");
                        let editor = Editor::with_options(&source, &parse_options, &edit_options);
                        let snapshot = editor.file();
                        let expected = match operation {
                            0 => {
                                editor.section("s").set("flag", "1");
                                format!("[s]\nflag{separator}1{gap}{comment}{ending}")
                            }
                            1 => {
                                editor.section("s").entries_mut()[0].set_value("1");
                                format!("[s]\nflag{separator}1{gap}{comment}{ending}")
                            }
                            2 => {
                                // An empty value cannot precede a marker, so
                                // the comment moves to its own line.
                                editor.section("s").set("flag", "");
                                let terminator = if ending.is_empty() { "\n" } else { ending };
                                format!("[s]\n{gap}{comment}{terminator}flag{separator}{ending}")
                            }
                            _ => {
                                assert!(editor.section("s").rename_key("flag", "other"));
                                format!("[s]\nother{gap}{comment}{ending}")
                            }
                        };
                        assert_eq!(editor.finish(), expected, "{source:?}");
                        let reopened = Editor::with_parse_options(&editor.finish(), &parse_options);
                        assert_eq!(editor.root.green(), reopened.root.green());
                        assert_eq!(snapshot.syntax().text().to_string(), source);
                    }
                }
            }
        }

        let editor = Editor::with_parse_options("[s]\nflag ; note\nnext=1\n", &parse_options);
        assert!(editor.section("s").remove_entry("flag"));
        assert_eq!(editor.finish(), "[s]\nnext=1\n");
    }

    #[test]
    fn file_views_are_immutable_snapshots() {
        let source = "[s]\nk=old\nk=duplicate\n[t]\nx=1\n";
        let editor = Editor::new(source);
        let snapshot = editor.file();
        let section = snapshot.sections().next().unwrap();
        let entry = section.entries().next().unwrap();
        assert!(!snapshot.syntax().is_mutable());

        editor.section("s").set("k", "new");
        editor.section("t").remove();
        editor.section("added").set("key", "value");
        assert_eq!(snapshot.syntax().text().to_string(), source);
        assert_eq!(entry.value().as_deref(), Some("old"));
        assert_eq!(section.entries().count(), 2);
        assert_eq!(
            editor
                .file()
                .sections()
                .next()
                .unwrap()
                .entries()
                .next()
                .unwrap()
                .value()
                .as_deref(),
            Some("new")
        );

        // Callers can explicitly create an editable copy of a snapshot.
        let copy = snapshot.syntax().clone_for_update();
        copy.first_child().unwrap().detach();
        assert_eq!(copy.text().to_string(), "[t]\nx=1\n");
        assert_eq!(snapshot.syntax().text().to_string(), source);
        assert!(editor.finish().contains("k=new\n"));
    }

    #[test]
    fn deleting_an_appended_line_matches_reloaded_editing() {
        for source in ["[s]", "[s]\na=1", "[s]\n; note"] {
            for operation in 0..4 {
                let direct = Editor::new(source);
                match operation {
                    0 => direct.section("s").append_entry("b", "2"),
                    1 => direct.section("s").insert_entry_at_line(99, "b", "2"),
                    2 => direct.section("s").append_raw_lines(&["; tail"]),
                    _ => direct.section("s").insert_raw_lines_at(99, &["; tail"]),
                }
                let reloaded = Editor::new(&direct.finish());
                let last_line = direct.section("s").node.children_with_tokens().count() - 1;
                direct.section("s").remove_lines(last_line..last_line + 1);
                reloaded.section("s").remove_lines(last_line..last_line + 1);
                assert_eq!(direct.finish(), reloaded.finish(), "{source:?}");
                assert_eq!(direct.finish(), format!("{source}\n"));
            }
        }
    }

    #[test]
    fn removing_sections_matches_reloaded_editing() {
        for source in ["[a]", "[a]\nk=1", "[a]\nk=", "[a]\nk=1\n", "[a]\n\n"] {
            let direct = Editor::new(source);
            direct.section("b").set("x", "2");
            let reloaded = Editor::new(&direct.finish());
            direct.section("a").remove();
            reloaded.section("a").remove();
            assert_eq!(direct.finish(), reloaded.finish(), "{source:?}");
            assert_eq!(direct.finish(), "[b]\nx = 2\n");
        }
    }

    #[test]
    fn creating_a_section_after_clearing_the_previous_one() {
        let editor = Editor::new("[a]\nx=1\n[b]\ny=2\n");
        editor.section("b").remove_lines(0..usize::MAX);
        editor.section("c").set("z", "3");
        assert_eq!(editor.finish(), "[a]\nx=1\n\n[c]\nz = 3\n");
    }

    #[test]
    fn new_section_follows_an_unterminated_empty_value_on_its_own_line() {
        let editor = Editor::new("[a]\nk=");
        editor.section("b").set("x", "2");
        assert_eq!(editor.finish(), "[a]\nk=\n\n[b]\nx = 2\n");
    }

    #[test]
    fn new_section_terminates_an_existing_blank_without_adding_another() {
        for source in [" \t", "\u{FEFF} \t", "k=1\n \t", "[a]\nk=1\n \t"] {
            let editor = Editor::new(source);
            let _ = editor.section("b");
            assert_eq!(editor.finish(), format!("{source}\n[b]\n"));
        }
    }

    #[test]
    fn creating_section_after_a_document_marker_needs_no_separator() {
        let editor = Editor::new("\u{FEFF}");
        editor.section("s").set("k", "v");
        assert_eq!(editor.finish(), "\u{FEFF}[s]\nk = v\n");

        // A BOM character used as a value is ordinary content, not metadata.
        let editor = Editor::new("k=\u{FEFF}");
        let _ = editor.section("s");
        assert_eq!(editor.finish(), "k=\u{FEFF}\n\n[s]\n");
    }

    #[test]
    fn raw_insertion_counts_an_error_line_as_one_line() {
        let editor = Editor::new("[s]\n=bad\nk=1\n");
        editor.section("s").insert_raw_lines_at(2, &["; note"]);
        assert_eq!(editor.finish(), "[s]\n=bad\n; note\nk=1\n");

        let editor = Editor::new("[s]\n=bad\nk=1\n");
        editor.section("s").insert_raw_lines_at(3, &["; note"]);
        assert_eq!(editor.finish(), "[s]\n=bad\nk=1\n; note\n");
    }

    #[test]
    fn retained_handle_can_insert_after_all_children_are_removed() {
        for operation in 0..5 {
            let editor = Editor::new("[s]\na=1\n");
            let section = editor.section("s");
            section.remove_lines(0..usize::MAX);
            assert_eq!(editor.finish(), "");
            match operation {
                0 => section.append_entry("b", "2"),
                1 => section.insert_entry_at_line(0, "b", "2"),
                2 => section.insert_entry_at_line(usize::MAX, "b", "2"),
                3 => section.append_raw_lines(&["b = 2"]),
                _ => section.insert_raw_lines_at(usize::MAX, &["b = 2"]),
            }
            assert_eq!(editor.finish(), "b = 2\n");
        }
    }

    #[test]
    fn insertion_into_a_body_of_only_blank_lines_precedes_the_blanks() {
        let editor = Editor::new("[s]\n\n\n");
        let section = editor.section("s");
        section.remove_lines(0..1);
        section.append_entry("b", "2");
        assert_eq!(editor.finish(), "b = 2\n\n\n");
    }

    #[test]
    fn appending_no_raw_lines_does_not_modify_unterminated_input() {
        for source in ["[s]", "[s]\nk=value", "[s]\n; comment", "[s]\nk=value\n"] {
            let editor = Editor::new(source);
            editor.section("s").append_raw_lines(&[]);
            editor.section("s").insert_raw_lines_at(0, &[]);
            editor.section("s").insert_raw_lines_at(usize::MAX, &[]);
            assert_eq!(editor.finish(), source);
        }
    }

    #[test]
    fn raw_insertion_at_zero_remains_before_the_header() {
        let editor = Editor::new("[s]\na=1\n");
        editor.section("s").insert_raw_lines_at(0, &["; preamble"]);
        assert_eq!(editor.finish(), "; preamble\n[s]\na=1\n");
    }

    #[test]
    fn raw_insertion_terminates_the_preceding_line() {
        for source in ["[s]", "[s]\nk=value", "[s]\n; comment"] {
            let editor = Editor::new(source);
            editor
                .section("s")
                .insert_raw_lines_at(usize::MAX, &["next=2"]);
            assert_eq!(editor.finish(), format!("{source}\nnext=2\n"));
        }
    }

    #[test]
    fn repeated_insertion_reuses_a_previously_added_terminator() {
        let editor = Editor::new("[s]");
        let section = editor.section("s");
        section.insert_entry_at_line(0, "a", "1");
        section.insert_entry_at_line(0, "b", "2");
        assert_eq!(editor.finish(), "[s]\nb = 2\na = 1\n");
    }

    #[test]
    fn append_keeps_malformed_lines_before_new_content() {
        for source in ["[s]\n=bad", "[s]\n=bad\n", "[s]\n=bad\n\n"] {
            let editor = Editor::new(source);
            editor.section("s").append_entry("b", "2");
            let gap = if source.ends_with("\n\n") { "\n" } else { "" };
            assert_eq!(editor.finish(), format!("[s]\n=bad\nb = 2\n{gap}"));
        }
    }

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
    fn clearing_a_value_keeps_its_comment_out_of_the_saved_value() {
        let parse_options = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        for ending in ["\n", "\r\n", "\r", ""] {
            for marker in [";", "#"] {
                for separator in ["=", ":"] {
                    for spacing in [
                        SeparatorSpacing::Preserve,
                        SeparatorSpacing::Compact,
                        SeparatorSpacing::exact("\t", "\t"),
                    ] {
                        for use_handle in [false, true] {
                            let source = format!("[s]\nk {separator} old \t{marker} keep{ending}");
                            let edit_options = EditOptions {
                                separator_spacing: spacing.clone(),
                            };
                            let editor =
                                Editor::with_options(&source, &parse_options, &edit_options);
                            let section = editor.section("s");
                            let handle = section.entries_mut().pop().unwrap();
                            if use_handle {
                                handle.set_value("");
                            } else {
                                section.set("k", "");
                            }
                            let output = editor.finish();
                            let reopened = Editor::with_parse_options(&output, &parse_options);
                            let saved = reopened.section("s").entries_mut().pop().unwrap();
                            assert_eq!(saved.value().as_deref(), Some(""), "{output:?}");
                            assert_eq!(handle.value().as_deref(), Some(""));
                            let (before, after) = spacing_for_new_entry(&spacing);
                            let comment_ending = if ending.is_empty() { "\n" } else { ending };
                            assert_eq!(
                                output,
                                format!(
                                    "[s]\n \t{marker} keep{comment_ending}k{before}{separator}{after}{ending}"
                                )
                            );
                            // Moving the comment must preserve the entry's identity.
                            handle.set_value("again");
                            assert!(editor.finish().contains("again"));
                            assert_eq!(editor.finish().matches("keep").count(), 1);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn clearing_a_detached_entry_handle_is_safe() {
        let options = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        let editor = Editor::with_parse_options("[s]\nk=old ; note\n", &options);
        let handle = editor.section("s").entries_mut().pop().unwrap();
        assert!(editor.section("s").remove_entry("k"));
        handle.set_value("");
        assert_eq!(handle.value().as_deref(), Some(""));
        assert_eq!(editor.finish(), "[s]\n");
    }

    #[test]
    fn blank_continued_lines_do_not_absorb_inline_comments() {
        let parse_options = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        for ending in ["\n", "\r\n", "\r"] {
            for spacing in [
                SeparatorSpacing::Preserve,
                SeparatorSpacing::Compact,
                SeparatorSpacing::exact("\t", "  "),
            ] {
                for marker in [";", "#"] {
                    for use_handle in [false, true] {
                        for source_ending in ["\n", "\r\n", "\r", ""] {
                            let source = format!("[s]\nk: old \t{marker} keep{source_ending}");
                            let options = EditOptions {
                                separator_spacing: spacing.clone(),
                            };
                            let editor = Editor::with_options(&source, &parse_options, &options);
                            let handle = editor.section("s").entries_mut().pop().unwrap();
                            let replacement = format!("one \\{ending}");
                            if use_handle {
                                handle.set_value(&replacement);
                            } else {
                                editor.section("s").set("k", &replacement);
                            }
                            let output = editor.finish();
                            let saved = Editor::with_parse_options(&output, &parse_options);
                            assert_eq!(handle.value().as_deref(), Some(replacement.as_str()));
                            assert_eq!(
                                saved.section("s").entries_mut()[0].value().as_deref(),
                                Some(replacement.as_str()),
                                "{output:?}"
                            );
                            assert_eq!(output.matches("keep").count(), 1);
                            assert!(output.contains(&format!("\t{marker} keep")));
                            handle.set_value("next");
                            assert_eq!(handle.value().as_deref(), Some("next"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn whitespace_only_replacements_keep_comments_separate() {
        let options = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        for replacement in [" \t", "one \\\n \t"] {
            let editor = Editor::with_parse_options("[s]\nk=old ; note\n", &options);
            editor.section("s").set("k", replacement);
            let output = editor.finish();
            let saved = Editor::with_parse_options(&output, &options);
            assert_eq!(
                saved.section("s").entries_mut()[0].value().as_deref(),
                Some(replacement.trim_end_matches([' ', '\t'])),
                "{output:?}"
            );
            assert_eq!(output.matches("; note").count(), 1);
        }
    }

    #[test]
    fn empty_value_spacing_is_stable_across_reloads() {
        for spacing in [
            SeparatorSpacing::Preserve,
            SeparatorSpacing::Compact,
            SeparatorSpacing::exact("\t", "  "),
        ] {
            for separator in ["=", ":"] {
                for (before, after, trailing) in [
                    ("", "", " \t"),
                    ("\t", "  ", "\t "),
                    (" ", "\t", ""),
                    ("", "", ""),
                ] {
                    for ending in ["\n", "\r\n", "\r", ""] {
                        for use_handle in [false, true] {
                            let source = format!(
                                "[s]\nuntouched = value  \nκ{before}{separator}{after}old{trailing}{ending}"
                            );
                            let options = EditOptions {
                                separator_spacing: spacing.clone(),
                            };
                            let editor = Editor::with_edit_options(&source, &options);
                            let handle = editor.section("s").entries_mut().pop().unwrap();
                            if use_handle {
                                handle.set_value("");
                            } else {
                                editor.section("s").set("κ", "");
                            }
                            let (expected_before, expected_after) = match &spacing {
                                SeparatorSpacing::Preserve => {
                                    (before, format!("{after}{trailing}"))
                                }
                                SeparatorSpacing::Compact => ("", String::new()),
                                SeparatorSpacing::Exact { before, after } => {
                                    (before.as_str(), after.clone())
                                }
                            };
                            let output = editor.finish();
                            assert_eq!(
                                output,
                                format!(
                                    "[s]\nuntouched = value  \nκ{expected_before}{separator}{expected_after}{ending}"
                                )
                            );
                            let reopened = Editor::with_edit_options(&output, &options);
                            assert_eq!(
                                editor.file().syntax().green(),
                                reopened.file().syntax().green()
                            );
                            handle.set_value("next");
                            reopened.section("s").set("κ", "next");
                            assert_eq!(editor.finish(), reopened.finish());
                            assert_eq!(handle.value().as_deref(), Some("next"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn carriage_return_value_keeps_its_own_line_ending() {
        let value = "one \\\r";
        for ending in ["\n", "\r\n", "\r", " \t\n", ""] {
            let source = format!("[s]\nk=old{ending}");
            let editor = Editor::new(&source);
            editor.section("s").set("k", value);
            let expected_ending = if ending == "\n" { "\r\n" } else { ending };
            let output = editor.finish();
            assert_eq!(output, format!("[s]\nk={value}{expected_ending}"));
            let reopened = Editor::new(&output);
            assert_eq!(
                reopened.section("s").entries_mut()[0].value().as_deref(),
                Some(value)
            );
        }
    }

    #[test]
    fn carriage_return_value_does_not_consume_following_entries() {
        let value = "one \\\r";
        for operation in 0..4 {
            let editor = Editor::new("[s]\nk=old\nnext=stay\n");
            let expected = match operation {
                0 => {
                    editor.section("s").set("k", value);
                    format!("[s]\nk={value}\r\nnext=stay\n")
                }
                1 => {
                    editor.section("s").entries_mut()[0].set_value(value);
                    format!("[s]\nk={value}\r\nnext=stay\n")
                }
                2 => {
                    editor.section("s").append_entry("added", value);
                    format!("[s]\nk=old\nnext=stay\nadded = {value}\r\n")
                }
                _ => {
                    editor.section("s").insert_entry_at_line(0, "added", value);
                    format!("[s]\nadded = {value}\r\nk=old\nnext=stay\n")
                }
            };
            let output = editor.finish();
            assert_eq!(output, expected);
            let reopened = Editor::new(&output);
            assert_eq!(
                reopened.section("s").entries_mut().len(),
                if operation < 2 { 2 } else { 3 }
            );
            assert_eq!(
                reopened
                    .section("s")
                    .find_entry("next")
                    .unwrap()
                    .value()
                    .as_deref(),
                Some("stay")
            );
            let key = if operation < 2 { "k" } else { "added" };
            assert_eq!(
                reopened
                    .section("s")
                    .find_entry(key)
                    .unwrap()
                    .value()
                    .as_deref(),
                Some(value)
            );
        }
    }

    #[test]
    fn completing_a_cr_continuation_keeps_the_following_line_separate() {
        let value = "one \\\r";
        for operation in 0..4 {
            let editor = Editor::new(&format!("[s]\nk={value}"));
            match operation {
                0 => editor.section("s").append_entry("next", "stay"),
                1 => editor.section("s").insert_entry_at_line(99, "next", "stay"),
                2 => editor.section("s").append_raw_lines(&["; keep"]),
                _ => editor.section("other").set("next", "stay"),
            }
            let output = editor.finish();
            assert!(
                output.starts_with(&format!("[s]\nk={value}\r\n")),
                "{output:?}"
            );
            let reopened = Editor::new(&output);
            assert_eq!(
                reopened.section("s").entries_mut()[0].value().as_deref(),
                Some(value)
            );
            assert_eq!(
                editor.file().syntax().green(),
                reopened.file().syntax().green()
            );
        }
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

    #[test]
    fn insert_entry_at_line_past_the_end_matches_append_entry() {
        // `insert_entry_at_line` documents that any `line` at or beyond the
        // number of content lines behaves exactly like `append_entry`. Only
        // entries and comments count as content lines, so every body below
        // ends with lines that do not count: malformed lines, sometimes after
        // a blank line. Each body is also tried with a following blank line
        // and section, which both methods must keep below the new entry.
        let bodies = [
            "a=1\n=bad\n",
            "a=1\n  =bad\n",
            "a=1\n:\n",
            "; note\n=bad\n",
            "a=1\n\n=bad\n",
            "a=1\n=bad\n=worse\n",
            "=bad\n",
            "\n=bad\n",
        ];
        for body in bodies {
            for tail in ["", "\n[next]\nz=9\n"] {
                for ending in ["\n", "\r\n", "\r"] {
                    let source = format!("[s]\n{body}{tail}").replace('\n', ending);
                    let appended = Editor::new(&source);
                    appended.section("s").append_entry("n", "1");
                    // Count content lines independently of the editor: every
                    // line except blank lines and malformed lines, which are
                    // the ones starting with `=` or `:` here.
                    let content_lines = body
                        .lines()
                        .filter(|line| {
                            !line.trim_start().starts_with(['=', ':']) && !line.is_empty()
                        })
                        .count();
                    // `0` is documented as "right after the header", even in a
                    // section without content lines, so start at `1`.
                    let first = content_lines.max(1);
                    for line in [first, first + 1, usize::MAX] {
                        let inserted = Editor::new(&source);
                        inserted.section("s").insert_entry_at_line(line, "n", "1");
                        assert_eq!(
                            inserted.finish(),
                            appended.finish(),
                            "{source:?}, line {line}"
                        );
                        assert_eq!(
                            inserted.root.green(),
                            Editor::new(&inserted.finish()).root.green()
                        );
                    }
                }
            }
        }

        // The same holds when the last malformed line has no terminator.
        let appended = Editor::new("[s]\na=1\n=bad");
        appended.section("s").append_entry("n", "1");
        let inserted = Editor::new("[s]\na=1\n=bad");
        inserted
            .section("s")
            .insert_entry_at_line(usize::MAX, "n", "1");
        assert_eq!(inserted.finish(), "[s]\na=1\n=bad\nn = 1\n");
        assert_eq!(inserted.finish(), appended.finish());

        // Positions before the end still count only entries and comments:
        // `0` stays right after the header and `1` right after `a=1`, even
        // though malformed lines follow. In a section whose only lines are
        // malformed, `0` also stays right after the header.
        let editor = Editor::new("[s]\n=bad\n");
        editor.section("s").insert_entry_at_line(0, "x", "0");
        assert_eq!(editor.finish(), "[s]\nx = 0\n=bad\n");

        let editor = Editor::new("[s]\na=1\n=bad\nb=2\n=worse\n");
        editor.section("s").insert_entry_at_line(0, "x", "0");
        editor.section("s").insert_entry_at_line(2, "y", "1");
        assert_eq!(
            editor.finish(),
            "[s]\nx = 0\na=1\ny = 1\n=bad\nb=2\n=worse\n"
        );
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
    fn append_raw_lines_preserves_crlf() {
        let ed = Editor::new("[s]\r\n");
        ed.section("s").append_raw_lines(&["raw = line\r\n"]);
        assert_eq!(ed.finish(), "[s]\r\nraw = line\r\n");
    }

    #[test]
    fn append_raw_lines_preserves_bare_cr() {
        let ed = Editor::new("[s]\r");
        ed.section("s").append_raw_lines(&["raw = line\r"]);
        assert_eq!(ed.finish(), "[s]\rraw = line\r");
    }

    #[test]
    fn append_raw_lines_eof_without_newline() {
        // Appending raw lines after an unterminated last line adds a separator.
        let ed = Editor::new("[s]\nk = v");
        ed.section("s").append_raw_lines(&["raw = line"]);
        assert_eq!(ed.finish(), "[s]\nk = v\nraw = line\n");
    }

    #[test]
    fn remove_lines_counts_an_error_line_as_one_line() {
        // A line starting with `=` is an ERROR_LINE node that owns its newline.
        // Children: 0 SECTION_HEADER, 1 ERROR_LINE, 2 ENTRY(k).
        let ed = Editor::new("[s]\n=bad\nk = v\n");
        ed.section("s").remove_lines(1..2);
        assert_eq!(ed.finish(), "[s]\nk = v\n");
    }

    #[test]
    fn remove_lines_reversed_range_is_noop() {
        let ed = Editor::new("[s]\na = 1\nb = 2\n");
        let start = 3;
        let end = 1;
        ed.section("s").remove_lines(start..end);
        assert_eq!(ed.finish(), "[s]\na = 1\nb = 2\n");
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

    #[test]
    fn create_section_after_blank_preamble_does_not_add_another_blank() {
        let ed = Editor::new("g = 1\n\n");
        ed.section("s").append_entry("k", "v");
        assert_eq!(ed.finish(), "g = 1\n\n[s]\nk = v\n");
    }

    // --- duplicate-key safety + entry handles ---

    #[test]
    fn rename_key_refuses_duplicate() {
        let ed = Editor::new("[paths]\na = 1\nb = 2\n");
        // Renaming b -> a would collide with the existing a; refuse, no change.
        assert!(!ed.section("paths").rename_key("b", "a"));
        assert_eq!(ed.finish(), "[paths]\na = 1\nb = 2\n");
    }

    #[test]
    fn rename_key_to_same_name_is_noop_success() {
        let ed = Editor::new("[s]\nk = v\n");
        assert!(ed.section("s").rename_key("k", "k"));
        assert_eq!(ed.finish(), "[s]\nk = v\n");
    }

    #[test]
    fn rename_key_still_works_without_collision() {
        let ed = Editor::new("[s]\nold = v\n");
        assert!(ed.section("s").rename_key("old", "new"));
        assert_eq!(ed.finish(), "[s]\nnew = v\n");
        assert!(!ed.section("s").rename_key("absent", "x"));
    }

    #[test]
    fn entries_mut_positional_rename() {
        let ed = Editor::new("[paths]\na = 1\nb = 2\nc = 3\n");
        for entry in ed.section("paths").entries_mut() {
            let i = entry.index();
            entry.set_key(&format!("k{i}"));
        }
        assert_eq!(ed.finish(), "[paths]\nk0 = 1\nk1 = 2\nk2 = 3\n");
    }

    #[test]
    fn entries_mut_disambiguates_duplicate_keys() {
        let ed = Editor::new("[s]\na = 1\na = 2\n");
        let entries = ed.section("s").entries_mut();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].value().as_deref(), Some("1"));
        assert_eq!(entries[1].value().as_deref(), Some("2"));
        // Target the second `a` by position — name lookup couldn't.
        entries[1].set_key("b");
        assert_eq!(ed.finish(), "[s]\na = 1\nb = 2\n");
    }

    #[test]
    fn entry_editor_accessors() {
        let ed = Editor::new("[s]\nhost = localhost\nport = 8080\n");
        let entries = ed.section("s").entries_mut();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].index(), 0);
        assert_eq!(entries[0].key().as_deref(), Some("host"));
        assert_eq!(entries[0].value().as_deref(), Some("localhost"));
        assert_eq!(entries[1].index(), 1);
        assert_eq!(entries[1].key().as_deref(), Some("port"));
    }

    #[test]
    fn entry_editor_value_none_for_bare_key() {
        let opts = ParseOptions {
            allow_no_value: true,
            ..Default::default()
        };
        let ed = Editor::with_parse_options("[s]\nflag\n", &opts);
        let entries = ed.section("s").entries_mut();
        assert_eq!(entries[0].key().as_deref(), Some("flag"));
        assert_eq!(entries[0].value(), None);
    }

    #[test]
    fn entry_editor_set_value() {
        let ed = Editor::new("[s]\na = 1\nb = 2\n");
        let entries = ed.section("s").entries_mut();
        entries[0].set_value("10");
        entries[1].set_value(""); // clear the value
        assert_eq!(ed.finish(), "[s]\na = 10\nb = \n");
    }

    #[test]
    fn set_value_preserves_colon_separator() {
        let ed = Editor::new("[s]\na: 1\n");
        ed.section("s").set("a", "2");
        assert_eq!(ed.finish(), "[s]\na: 2\n");
    }

    #[test]
    fn set_value_adds_separator_to_bare_key() {
        let opts = ParseOptions {
            allow_no_value: true,
            ..Default::default()
        };

        let ed = Editor::with_parse_options("[s]\nflag\n", &opts);
        ed.section("s").set("flag", "on");
        assert_eq!(ed.finish(), "[s]\nflag = on\n");

        let ed = Editor::with_parse_options("[s]\nflag\n", &opts);
        ed.section("s").entries_mut()[0].set_value("");
        assert_eq!(ed.finish(), "[s]\nflag = \n");
    }

    #[test]
    fn compact_separator_spacing_applies_to_touched_entries() {
        let edit_options = EditOptions {
            separator_spacing: SeparatorSpacing::Compact,
        };
        let ed = Editor::with_edit_options(
            "[s]\n  changed   =   old\nuntouched : keep\ncolon: old\n",
            &edit_options,
        );

        ed.section("s").set("changed", "new");
        ed.section("s").set("missing", "added");
        ed.section("s").insert_entry_at_line(1, "inserted", "here");
        ed.section("s").entries_mut()[3].set_value("new");

        assert_eq!(
            ed.finish(),
            "[s]\n  changed=new\ninserted=here\nuntouched : keep\ncolon:new\nmissing=added\n"
        );
    }

    #[test]
    fn exact_separator_spacing_supports_custom_whitespace() {
        let edit_options = EditOptions {
            separator_spacing: SeparatorSpacing::exact("\t", "  "),
        };
        let ed = Editor::with_edit_options("[s]\na = old\n", &edit_options);

        ed.section("s").set("a", "new");
        ed.section("s").append_entry("b", "added");

        assert_eq!(ed.finish(), "[s]\na\t=  new\nb\t=  added\n");
    }

    #[test]
    fn parse_and_edit_options_can_be_combined() {
        let parse_options = ParseOptions {
            allow_no_value: true,
            inline_comments: true,
        };
        let edit_options = EditOptions {
            separator_spacing: SeparatorSpacing::Compact,
        };
        let ed = Editor::with_options(
            "[s]\nflag   \nvalue = old   ; keep\nuntouched = yes\n",
            &parse_options,
            &edit_options,
        );

        ed.section("s").set("flag", "on");
        ed.section("s").set("value", "new");

        assert_eq!(
            ed.finish(),
            "[s]\nflag=on\nvalue=new   ; keep\nuntouched = yes\n"
        );
    }

    #[test]
    fn entry_editor_remove() {
        let ed = Editor::new("[s]\na = 1\nb = 2\nc = 3\n");
        // Remove the middle entry by position.
        ed.section("s")
            .entries_mut()
            .into_iter()
            .nth(1)
            .unwrap()
            .remove();
        assert_eq!(ed.finish(), "[s]\na = 1\nc = 3\n");
    }

    #[test]
    fn entries_mut_empty_section_is_empty() {
        let ed = Editor::new("[s]\n");
        assert!(ed.section("s").entries_mut().is_empty());
    }
}
