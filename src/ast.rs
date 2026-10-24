//! Typed AST wrappers over the lossless syntax tree.
//!
//! Each wrapper checks the node's `SyntaxKind` at runtime via `cast`.
//! Accessors return `Option` because the tree may be malformed.
//!
//! The accessors read a node's children from its green node, the immutable data under the cursor.
//! rowan's child iterators build a cursor for every child they step over. Here the string and
//! `bool` accessors build none, and the node and token accessors build one for each child they
//! return. The exception is a tree with several empty siblings at one offset, which no parser
//! makes: reaching one of them before the last walks a cursor over every child before it. The line
//! iterators, [`File::preamble_lines`] and [`Section::body_lines`], return almost every child node
//! and use rowan's iterator.
//!
//! Every wrapper holds a [`SyntaxNode`], a rowan cursor that counts its references without
//! atomics, so none of them is `Send` or `Sync`: build them in the thread that reads them, from a
//! shared [`Parse`](crate::Parse) or [`Editor`](crate::editor::Editor) ([Threads](crate#threads)).

use std::iter::Enumerate;

use rowan::{Children, GreenNodeData, GreenTokenData, Language, NodeOrToken, TextRange, TextSize};

use crate::syntax_kind::{IniLang, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// Trait for typed wrappers over `SyntaxNode`.
pub trait AstNode: Sized {
    /// Try to cast a `SyntaxNode` into this type.
    fn cast(syntax: SyntaxNode) -> Option<Self>;
    /// Access the underlying `SyntaxNode`.
    fn syntax(&self) -> &SyntaxNode;
}

macro_rules! ast_node {
    ($(#[doc = $doc:expr])* $name:ident, $kind:ident) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone)]
        pub struct $name(SyntaxNode);
        impl AstNode for $name {
            fn cast(syntax: SyntaxNode) -> Option<Self> {
                (syntax.kind() == SyntaxKind::$kind).then_some(Self(syntax))
            }
            fn syntax(&self) -> &SyntaxNode { &self.0 }
        }
    };
}

ast_node!(/// The root of an INI file.
    File, ROOT);
ast_node!(/// A `[name]` section with its entries.
    Section, SECTION);
ast_node!(/// The `[name]` header of a section.
    SectionHeader, SECTION_HEADER);
ast_node!(/// A key-value entry.
    Entry, ENTRY);
ast_node!(/// The key portion of an entry.
    Key, KEY);
ast_node!(/// The value portion of an entry.
    Value, VALUE);
ast_node!(/// A full-line comment with its terminating newline.
    CommentLine, COMMENT_LINE);
ast_node!(/// A blank line (whitespace and/or a newline).
    BlankLine, BLANK_LINE);

impl File {
    /// Entries before any `[section]` header.
    pub fn preamble_entries(&self) -> impl Iterator<Item = Entry> + '_ {
        child_nodes(&self.0, SyntaxKind::ENTRY).map(Entry)
    }

    /// All sections in document order.
    pub fn sections(&self) -> impl Iterator<Item = Section> + '_ {
        child_nodes(&self.0, SyntaxKind::SECTION).map(Section)
    }

    /// The line nodes before the first section, after a byte order mark: the preamble's body
    /// lines, in the order of the editor's body-line positions.
    pub fn preamble_lines(&self) -> impl Iterator<Item = SyntaxNode> + '_ {
        self.0
            .children()
            .take_while(|node| node.kind() != SyntaxKind::SECTION)
    }
}

impl Section {
    /// The section header node.
    #[must_use]
    pub fn header(&self) -> Option<SectionHeader> {
        child_nodes(&self.0, SyntaxKind::SECTION_HEADER)
            .next()
            .map(SectionHeader)
    }

    /// The section name.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        section_name(self.0.green()).map(str::to_owned)
    }

    /// Entries in this section.
    pub fn entries(&self) -> impl Iterator<Item = Entry> + '_ {
        child_nodes(&self.0, SyntaxKind::ENTRY).map(Entry)
    }

    /// Full-line comments in this section (in document order).
    pub fn comment_lines(&self) -> impl Iterator<Item = CommentLine> + '_ {
        child_nodes(&self.0, SyntaxKind::COMMENT_LINE).map(CommentLine)
    }

    /// The line nodes after the header: the section's body lines, in the order of the editor's
    /// body-line positions.
    pub fn body_lines(&self) -> impl Iterator<Item = SyntaxNode> + '_ {
        self.0
            .children()
            .filter(|node| node.kind() != SyntaxKind::SECTION_HEADER)
    }
}

impl SectionHeader {
    /// The IDENT token between the brackets.
    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        child_token(&self.0, SyntaxKind::IDENT)
    }

    /// The section name as a string.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        token_text(self.0.green(), SyntaxKind::IDENT).map(str::to_owned)
    }
}

impl Entry {
    /// The KEY node.
    #[must_use]
    pub fn key_node(&self) -> Option<Key> {
        child_nodes(&self.0, SyntaxKind::KEY).next().map(Key)
    }

    /// The key text.
    #[must_use]
    pub fn key(&self) -> Option<String> {
        entry_key(self.0.green()).map(str::to_owned)
    }

    /// The VALUE node.
    #[must_use]
    pub fn value_node(&self) -> Option<Value> {
        child_nodes(&self.0, SyntaxKind::VALUE).next().map(Value)
    }

    /// The value text. Returns `Some("")` for empty values (`key =`),
    /// `None` for bare keys without a separator (`key` alone).
    #[must_use]
    pub fn value(&self) -> Option<String> {
        let entry = self.0.green();
        // A bare key has no EQ or COLON token, unlike an empty value (`key =`).
        if !has_token(entry, &[SyntaxKind::EQ, SyntaxKind::COLON]) {
            return None;
        }
        let value = child_node(entry, SyntaxKind::VALUE)?;
        let text = token_text(value, SyntaxKind::VALUE_TEXT).unwrap_or_default();
        Some(text.to_owned())
    }

    /// True if the separator is `:` rather than `=`.
    #[must_use]
    pub fn uses_colon(&self) -> bool {
        has_token(self.0.green(), &[SyntaxKind::COLON])
    }

    /// The trailing inline comment (e.g. `; note`), including its leading
    /// marker, if one is present.
    ///
    /// Only populated when the source was parsed with
    /// [`ParseOptions::inline_comments`](crate::ParseOptions::inline_comments)
    /// enabled; otherwise the marker is part of [`value()`](Self::value) and
    /// this returns `None`.
    #[must_use]
    pub fn inline_comment(&self) -> Option<String> {
        token_text(self.0.green(), SyntaxKind::COMMENT).map(str::to_owned)
    }
}

impl Key {
    /// The underlying IDENT token.
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        child_token(&self.0, SyntaxKind::IDENT)
    }

    /// The key as a string.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        token_text(self.0.green(), SyntaxKind::IDENT).map(str::to_owned)
    }
}

impl Value {
    /// The underlying `VALUE_TEXT` token.
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        child_token(&self.0, SyntaxKind::VALUE_TEXT)
    }

    /// The value text, or `None` for empty values (`key =`).
    #[must_use]
    pub fn text(&self) -> Option<String> {
        token_text(self.0.green(), SyntaxKind::VALUE_TEXT).map(str::to_owned)
    }
}

impl CommentLine {
    /// The `COMMENT` token (including its leading `;`/`#` marker).
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        child_token(&self.0, SyntaxKind::COMMENT)
    }

    /// The comment text, including its leading `;`/`#` marker.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        token_text(self.0.green(), SyntaxKind::COMMENT).map(str::to_owned)
    }
}

/// The name of a `SECTION`, read from its green node as [`Section::name`] reads it: the first
/// `IDENT` token of its first `SECTION_HEADER` child.
pub(crate) fn section_name(section: &GreenNodeData) -> Option<&str> {
    assert_eq!(node_kind(section), SyntaxKind::SECTION);
    let header = child_node(section, SyntaxKind::SECTION_HEADER)?;
    token_text(header, SyntaxKind::IDENT)
}

/// The key of an `ENTRY`, read from its green node as [`Entry::key`] reads it: the first `IDENT`
/// token of its first `KEY` child.
pub(crate) fn entry_key(entry: &GreenNodeData) -> Option<&str> {
    assert_eq!(node_kind(entry), SyntaxKind::ENTRY);
    let key = child_node(entry, SyntaxKind::KEY)?;
    token_text(key, SyntaxKind::IDENT)
}

/// The kind of a green node. A raw kind that is not a [`SyntaxKind`] panics, as it does when a
/// cursor reads it.
fn node_kind(node: &GreenNodeData) -> SyntaxKind {
    IniLang::kind_from_raw(node.kind())
}

/// The kind of a green token, read as [`node_kind`] reads a node's.
fn token_kind(token: &GreenTokenData) -> SyntaxKind {
    IniLang::kind_from_raw(token.kind())
}

/// The first child node of `node` of kind `kind`.
fn child_node(node: &GreenNodeData, kind: SyntaxKind) -> Option<&GreenNodeData> {
    node.children()
        .filter_map(NodeOrToken::into_node)
        .find(|child| node_kind(child) == kind)
}

/// The text of the first child token of `node` of kind `kind`.
fn token_text(node: &GreenNodeData, kind: SyntaxKind) -> Option<&str> {
    node.children()
        .filter_map(NodeOrToken::into_token)
        .find(|token| token_kind(token) == kind)
        .map(GreenTokenData::text)
}

/// Whether `node` has a child token of one of `kinds`.
fn has_token(node: &GreenNodeData, kinds: &[SyntaxKind]) -> bool {
    node.children()
        .filter_map(NodeOrToken::into_token)
        .any(|token| kinds.contains(&token_kind(token)))
}

/// The child nodes of `parent` of kind `kind`, in order. Only they get cursors.
fn child_nodes(parent: &SyntaxNode, kind: SyntaxKind) -> ChildNodes<'_> {
    ChildNodes {
        parent,
        kind,
        children: parent.green().children().enumerate(),
        start: parent.text_range().start(),
        previous: None,
    }
}

/// The child nodes of one kind of a node, found through its green node.
struct ChildNodes<'a> {
    parent: &'a SyntaxNode,
    kind: SyntaxKind,
    /// The green children not walked yet, with their indices.
    children: Enumerate<Children<'a>>,
    /// Where the next child starts.
    start: TextSize,
    /// The child node returned last, unless a child node of another kind followed it.
    previous: Option<SyntaxNode>,
}

impl Iterator for ChildNodes<'_> {
    type Item = SyntaxNode;

    fn next(&mut self) -> Option<SyntaxNode> {
        for (index, child) in self.children.by_ref() {
            let range = TextRange::at(self.start, child.text_len());
            self.start = range.end();
            // Tokens lie between nodes without separating them: a node's next sibling is a node.
            let Some(node) = child.into_node() else {
                continue;
            };
            if node_kind(node) != self.kind {
                self.previous = None;
                continue;
            }
            let cursor = match self.previous.take() {
                // The child node directly after the one returned last: its next sibling, in O(1).
                Some(previous) => previous.next_sibling(),
                None => child_at(self.parent, index, range).into_node(),
            }
            .expect("the child is a node");
            assert_eq!(cursor.index(), index);
            assert_eq!(cursor.text_range(), range);
            assert_eq!(cursor.kind(), self.kind);
            self.previous = Some(cursor.clone());
            return Some(cursor);
        }
        None
    }
}

/// The first child token of `parent` of kind `kind`. Only it gets a cursor.
fn child_token(parent: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    let mut start = parent.text_range().start();
    for (index, child) in parent.green().children().enumerate() {
        let range = TextRange::at(start, child.text_len());
        start = range.end();
        if child
            .into_token()
            .is_some_and(|token| token_kind(token) == kind)
        {
            let token = child_at(parent, index, range).into_token();
            let token = token.expect("the child is a token");
            assert_eq!(token.kind(), kind);
            return Some(token);
        }
    }
    None
}

/// The cursor of child `index` of `parent`, which covers `range`.
fn child_at(parent: &SyntaxNode, index: usize, range: TextRange) -> SyntaxElement {
    // Siblings are contiguous, so a child with text is the only one that covers its whole range,
    // and the binary search finds it. For an empty range it finds the last of the empty children
    // there, such as the empty VALUE of `key =` or of a bare key.
    let found = parent
        .child_or_token_at_range(range)
        .expect("parent has the child");
    let child = if found.index() == index {
        found
    } else {
        // Only an empty child shares its range with a sibling: the walk counts its way there.
        assert!(range.is_empty());
        let child = parent.children_with_tokens().nth(index);
        child.expect("parent has the child")
    };
    assert_eq!(child.index(), index);
    assert_eq!(child.text_range(), range);
    child
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::parse;

    fn ast(input: &str) -> File {
        File::cast(parse(input).syntax()).unwrap()
    }

    #[test]
    fn sections_and_entries() {
        let file = ast("[a]\nx=1\ny=2\n[b]\nz=3\n");
        let sections: Vec<_> = file.sections().collect();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name().as_deref(), Some("a"));
        assert_eq!(sections[1].name().as_deref(), Some("b"));
        let entries: Vec<_> = sections[0].entries().collect();
        assert_eq!(entries[0].key().as_deref(), Some("x"));
        assert_eq!(entries[0].value().as_deref(), Some("1"));
    }

    #[test]
    fn empty_value() {
        let file = ast("[s]\nk=\n");
        let entry = file.sections().next().unwrap().entries().next().unwrap();
        assert_eq!(entry.value().as_deref(), Some(""));
    }

    #[test]
    fn colon_separator() {
        let file = ast("[s]\nk: v\n");
        let entry = file.sections().next().unwrap().entries().next().unwrap();
        assert!(entry.uses_colon());
        assert_eq!(entry.value().as_deref(), Some("v"));

        let file = ast("[s]\nk = v\nbare\n");
        let entries: Vec<_> = file.sections().next().unwrap().entries().collect();
        assert!(!entries[0].uses_colon());
        assert!(!entries[1].uses_colon());
    }

    #[test]
    fn preamble() {
        let file = ast("g=1\n[s]\nk=2\n");
        assert_eq!(file.preamble_entries().count(), 1);
        assert_eq!(file.sections().next().unwrap().entries().count(), 1);
    }

    #[test]
    fn inline_comment_accessor() {
        use crate::{ParseOptions, parse_with};
        let options = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        let file = File::cast(parse_with("[s]\nk = 1   ; note\n", &options).syntax()).unwrap();
        let entry = file.sections().next().unwrap().entries().next().unwrap();
        assert_eq!(entry.key().as_deref(), Some("k"));
        assert_eq!(entry.value().as_deref(), Some("1"));
        assert_eq!(entry.inline_comment().as_deref(), Some("; note"));
    }

    #[test]
    fn no_inline_comment_when_disabled() {
        // Default parse leaves the marker in the value and reports no comment.
        let file = ast("[s]\nk = 1   ; note\n");
        let entry = file.sections().next().unwrap().entries().next().unwrap();
        assert_eq!(entry.value().as_deref(), Some("1   ; note"));
        assert_eq!(entry.inline_comment(), None);
    }

    #[test]
    fn comment_lines_and_blank_lines() {
        let file = ast("[s]\n; first\nk = v\n# second\n\n");
        let section = file.sections().next().unwrap();

        let first = section.comment_lines().next().unwrap();
        assert_eq!(first.token().unwrap().kind(), SyntaxKind::COMMENT);
        assert_eq!(first.text().as_deref(), Some("; first"));
        assert_eq!(first.syntax().text().to_string(), "; first\n");

        let texts: Vec<_> = section
            .comment_lines()
            .filter_map(|comment| comment.text())
            .collect();
        assert_eq!(texts, vec!["; first".to_string(), "# second".to_string()]);

        let blank = section
            .syntax()
            .children()
            .find_map(BlankLine::cast)
            .unwrap();
        assert_eq!(blank.syntax().text().to_string(), "\n");
    }

    #[test]
    fn all_typed_nodes_are_cloneable_and_debuggable() {
        fn assert_traits<T: AstNode + Clone + std::fmt::Debug>(node: &T) {
            let cloned = T::clone(node);
            assert_ne!(format!("{cloned:?}"), "");
            assert_eq!(node.syntax().kind(), cloned.syntax().kind());
        }

        let file = ast("[s]\nk = v\n; note\n\n");
        let section = file.sections().next().unwrap();
        let header = section.header().unwrap();
        let entry = section.entries().next().unwrap();
        let key = entry.key_node().unwrap();
        let value = entry.value_node().unwrap();
        let comment = section.comment_lines().next().unwrap();
        let blank = section
            .syntax()
            .children()
            .find_map(BlankLine::cast)
            .unwrap();

        assert_traits(&file);
        assert_traits(&section);
        assert_traits(&header);
        assert_traits(&entry);
        assert_traits(&key);
        assert_traits(&value);
        assert_traits(&comment);
        assert_traits(&blank);
    }

    #[test]
    fn malformed_nodes_return_none_from_optional_accessors() {
        fn empty_node(kind: SyntaxKind) -> SyntaxNode {
            let mut builder = rowan::GreenNodeBuilder::new();
            builder.start_node(kind.into());
            builder.finish_node();
            SyntaxNode::new_root(builder.finish())
        }

        let section = Section::cast(empty_node(SyntaxKind::SECTION)).unwrap();
        assert!(section.header().is_none());
        assert!(section.name().is_none());
        assert!(section.entries().next().is_none());
        assert!(section.comment_lines().next().is_none());

        let header = SectionHeader::cast(empty_node(SyntaxKind::SECTION_HEADER)).unwrap();
        assert!(header.name_token().is_none());
        assert!(header.name().is_none());
        assert_eq!(header.syntax().kind(), SyntaxKind::SECTION_HEADER);

        let entry = Entry::cast(empty_node(SyntaxKind::ENTRY)).unwrap();
        assert!(entry.key_node().is_none());
        assert!(entry.key().is_none());
        assert!(entry.value_node().is_none());
        assert!(entry.value().is_none());
        assert!(!entry.uses_colon());
        assert!(entry.inline_comment().is_none());

        let key = Key::cast(empty_node(SyntaxKind::KEY)).unwrap();
        assert!(key.token().is_none());
        assert!(key.text().is_none());

        let value = Value::cast(empty_node(SyntaxKind::VALUE)).unwrap();
        assert!(value.token().is_none());
        assert!(value.text().is_none());

        let comment = CommentLine::cast(empty_node(SyntaxKind::COMMENT_LINE)).unwrap();
        assert!(comment.token().is_none());
        assert!(comment.text().is_none());

        let blank = BlankLine::cast(empty_node(SyntaxKind::BLANK_LINE)).unwrap();
        assert_eq!(blank.syntax().kind(), SyntaxKind::BLANK_LINE);

        let root = empty_node(SyntaxKind::ROOT);
        let file = File::cast(root.clone()).unwrap();
        assert_eq!(file.syntax().kind(), SyntaxKind::ROOT);
        assert!(Section::cast(root.clone()).is_none());
        assert!(SectionHeader::cast(root.clone()).is_none());
        assert!(Entry::cast(root.clone()).is_none());
        assert!(Key::cast(root.clone()).is_none());
        assert!(Value::cast(root.clone()).is_none());
        assert!(CommentLine::cast(root.clone()).is_none());
        assert!(BlankLine::cast(root).is_none());
    }

    /// A tree built from `events`: `Some(kind)` with `None` for its text starts a node, `None`
    /// finishes the last node started, and `Some(kind)` with `Some(text)` is a token.
    fn tree(events: &[(Option<SyntaxKind>, Option<&str>)]) -> SyntaxNode {
        let mut builder = rowan::GreenNodeBuilder::new();
        for &(kind, text) in events {
            match (kind, text) {
                (Some(kind), None) => builder.start_node(kind.into()),
                (Some(kind), Some(text)) => builder.token(kind.into(), text),
                (None, _) => builder.finish_node(),
            }
        }
        SyntaxNode::new_root(builder.finish())
    }

    fn start(kind: SyntaxKind) -> (Option<SyntaxKind>, Option<&'static str>) {
        (Some(kind), None)
    }

    fn token(kind: SyntaxKind, text: &str) -> (Option<SyntaxKind>, Option<&str>) {
        (Some(kind), Some(text))
    }

    const FINISH: (Option<SyntaxKind>, Option<&str>) = (None, None);

    fn indices<T: AstNode>(nodes: impl Iterator<Item = T>) -> Vec<usize> {
        nodes.map(|node| node.syntax().index()).collect()
    }

    #[test]
    fn empty_children_are_found_by_their_index() {
        use SyntaxKind::{COMMENT_LINE, ROOT, SECTION};
        // Empty siblings share one range, so only their index tells them apart.
        let root = tree(&[
            start(ROOT),
            start(SECTION),
            FINISH,
            start(SECTION),
            FINISH,
            start(COMMENT_LINE),
            FINISH,
            start(SECTION),
            FINISH,
            FINISH,
        ]);
        let file = File::cast(root).unwrap();
        assert_eq!(indices(file.sections()), [0, 1, 3]);
        for section in file.sections() {
            assert!(section.syntax().text_range().is_empty());
        }

        // An empty token, too.
        let key = tree(&[start(SyntaxKind::KEY), token(SyntaxKind::IDENT, ""), FINISH]);
        let key = Key::cast(key).unwrap();
        let ident = key.token().unwrap();
        assert_eq!(
            (ident.index(), ident.text_range()),
            (0, key.syntax().text_range())
        );
        assert_eq!(key.text().as_deref(), Some(""));

        // The parser leaves the VALUE of `key =` and of a bare key empty.
        let file = ast("k =\nb\n");
        let entries: Vec<_> = file.preamble_entries().collect();
        let values: Vec<_> = entries.iter().filter_map(Entry::value_node).collect();
        assert_eq!(indices(values.iter().cloned()), [3, 1]);
        for value in &values {
            assert!(value.syntax().text_range().is_empty());
        }
    }

    #[test]
    fn a_separator_without_a_value_node_reads_no_value() {
        use SyntaxKind::{ENTRY, EQ, IDENT, KEY};
        let entry = tree(&[
            start(ENTRY),
            start(KEY),
            token(IDENT, "k"),
            FINISH,
            token(EQ, "="),
            FINISH,
        ]);
        let entry = Entry::cast(entry).unwrap();
        assert_eq!(entry.key().as_deref(), Some("k"));
        assert!(entry.value_node().is_none());
        assert_eq!(entry.value(), None);
        assert!(!entry.uses_colon());
    }

    #[test]
    fn children_of_a_kind_are_found_across_tokens_and_other_nodes() {
        use SyntaxKind::{COMMENT, COMMENT_LINE, IDENT, L_BRACK, SECTION_HEADER, WHITESPACE};
        let section = tree(&[
            start(SyntaxKind::SECTION),
            start(COMMENT_LINE),
            token(COMMENT, "; c"),
            FINISH,
            token(WHITESPACE, " "),
            start(SECTION_HEADER),
            token(L_BRACK, "["),
            token(IDENT, "a"),
            FINISH,
            token(WHITESPACE, " "),
            start(COMMENT_LINE),
            token(COMMENT, "; d"),
            FINISH,
            start(COMMENT_LINE),
            token(COMMENT, "; e"),
            FINISH,
            FINISH,
        ]);
        let section = Section::cast(section).unwrap();
        // The header need not be the first child.
        let header = section.header().unwrap();
        assert_eq!(header.syntax().index(), 2);
        assert_eq!(header.syntax().text(), "[a");
        assert_eq!(section.name().as_deref(), Some("a"));
        let name = header.name_token().unwrap();
        assert_eq!((name.index(), name.text()), (1, "a"));
        // A token between two comment lines, or a header, leaves each found where it is.
        let comments: Vec<_> = section.comment_lines().collect();
        assert_eq!(indices(comments.iter().cloned()), [0, 4, 5]);
        let texts: Vec<_> = comments.iter().filter_map(CommentLine::text).collect();
        assert_eq!(texts, ["; c", "; d", "; e"]);
        let ranges: Vec<_> = comments
            .iter()
            .map(|comment| comment.syntax().text_range())
            .collect();
        let range = |start: u32, end: u32| TextRange::new(start.into(), end.into());
        assert_eq!(ranges, [range(0, 3), range(7, 10), range(10, 13)]);
    }

    #[test]
    fn preamble_and_body_lines_are_the_editor_positions() {
        fn texts(lines: impl Iterator<Item = SyntaxNode>) -> Vec<String> {
            lines.map(|line| line.text().to_string()).collect()
        }

        // A byte order mark is not a line, and the preamble ends at the first header.
        let file = ast("\u{FEFF}g=1\n; c\n[a]\nx=1\n\n[b]\n");
        assert_eq!(texts(file.preamble_lines()), ["g=1\n", "; c\n"]);
        let sections: Vec<_> = file.sections().collect();
        // A header is not a body line; blank lines are.
        assert_eq!(texts(sections[0].body_lines()), ["x=1\n", "\n"]);
        assert_eq!(sections[1].body_lines().count(), 0);

        // Without a section, every line is in the preamble.
        let file = ast("k=v\n\n=bad");
        assert_eq!(texts(file.preamble_lines()), ["k=v\n", "\n", "=bad"]);
        assert_eq!(ast("[s]\n").preamble_lines().count(), 0);
    }
}
