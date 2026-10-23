//! Equivalence oracle: the typed AST reads a node's children from its green node, and every
//! accessor must return exactly what a walk of rowan's cursors returns. The cursor readings here
//! are the accessors as they were before they read green nodes, kept as the reference: every child
//! is a cursor, visited in order.

use ini_edit::ast::{AstNode, CommentLine, Entry, File, Key, Section, SectionHeader, Value};
use ini_edit::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// Check every accessor of every node under `root`, `root` included, against its cursor reading.
pub(crate) fn check(root: &SyntaxNode) {
    for node in root.descendants() {
        let children = Children::of(&node);
        if let Some(file) = File::cast(node.clone()) {
            check_file(&file, &children);
        } else if let Some(section) = Section::cast(node.clone()) {
            check_section(&section, &children);
        } else if let Some(header) = SectionHeader::cast(node.clone()) {
            same_token(header.name_token(), children.token(SyntaxKind::IDENT));
            assert_eq!(header.name(), children.text(SyntaxKind::IDENT));
        } else if let Some(entry) = Entry::cast(node.clone()) {
            check_entry(&entry, &children);
        } else if let Some(key) = Key::cast(node.clone()) {
            same_token(key.token(), children.token(SyntaxKind::IDENT));
            assert_eq!(key.text(), children.text(SyntaxKind::IDENT));
        } else if let Some(value) = Value::cast(node.clone()) {
            same_token(value.token(), children.token(SyntaxKind::VALUE_TEXT));
            assert_eq!(value.text(), children.text(SyntaxKind::VALUE_TEXT));
        } else if let Some(comment) = CommentLine::cast(node.clone()) {
            same_token(comment.token(), children.token(SyntaxKind::COMMENT));
            assert_eq!(comment.text(), children.text(SyntaxKind::COMMENT));
        }
    }
}

fn check_file(file: &File, children: &Children) {
    let entries = file.preamble_entries().map(|entry| entry.syntax().clone());
    same_nodes(entries, &children.nodes(SyntaxKind::ENTRY));
    let sections = file.sections().map(|section| section.syntax().clone());
    same_nodes(sections, &children.nodes(SyntaxKind::SECTION));
}

fn check_section(section: &Section, children: &Children) {
    let header = children.first_node(SyntaxKind::SECTION_HEADER);
    let actual = section.header().map(|header| header.syntax().clone());
    same_nodes(actual.into_iter(), header.as_slice());
    let name = header.map(|header| Children::of(&header).text(SyntaxKind::IDENT));
    assert_eq!(section.name(), name.flatten(), "{:?}", section.syntax());
    let entries = section.entries().map(|entry| entry.syntax().clone());
    same_nodes(entries, &children.nodes(SyntaxKind::ENTRY));
    let comments = section.comment_lines().map(|line| line.syntax().clone());
    same_nodes(comments, &children.nodes(SyntaxKind::COMMENT_LINE));
}

fn check_entry(entry: &Entry, children: &Children) {
    let node = entry.syntax();
    let key = children.first_node(SyntaxKind::KEY);
    let actual = entry.key_node().map(|key| key.syntax().clone());
    same_nodes(actual.into_iter(), key.as_slice());
    let value = children.first_node(SyntaxKind::VALUE);
    let actual = entry.value_node().map(|value| value.syntax().clone());
    same_nodes(actual.into_iter(), value.as_slice());
    let key = key.and_then(|key| Children::of(&key).text(SyntaxKind::IDENT));
    assert_eq!(entry.key(), key, "{node:?}");
    // A bare key has no separator token, unlike an empty value.
    let separated = children.has_token(&[SyntaxKind::EQ, SyntaxKind::COLON]);
    let value = value.filter(|_| separated).map(|value| {
        let text = Children::of(&value).text(SyntaxKind::VALUE_TEXT);
        text.unwrap_or_default()
    });
    assert_eq!(entry.value(), value, "{node:?}");
    let colon = children.has_token(&[SyntaxKind::COLON]);
    assert_eq!(entry.uses_colon(), colon, "{node:?}");
    let comment = children.text(SyntaxKind::COMMENT);
    assert_eq!(entry.inline_comment(), comment, "{node:?}");
}

/// The children of a node, walked once with cursors, in order: each reading below visits them as
/// the accessor it stands for once walked them.
struct Children(Vec<SyntaxElement>);

impl Children {
    fn of(node: &SyntaxNode) -> Self {
        Self(node.children_with_tokens().collect())
    }

    /// The child nodes of kind `kind`.
    fn nodes(&self, kind: SyntaxKind) -> Vec<SyntaxNode> {
        let nodes = self.0.iter().filter_map(SyntaxElement::as_node);
        nodes.filter(|node| node.kind() == kind).cloned().collect()
    }

    /// The first child node of kind `kind`.
    fn first_node(&self, kind: SyntaxKind) -> Option<SyntaxNode> {
        self.nodes(kind).into_iter().next()
    }

    /// The first child token of kind `kind`.
    fn token(&self, kind: SyntaxKind) -> Option<SyntaxToken> {
        let mut tokens = self.0.iter().filter_map(SyntaxElement::as_token);
        tokens.find(|token| token.kind() == kind).cloned()
    }

    /// The text of the first child token of kind `kind`.
    fn text(&self, kind: SyntaxKind) -> Option<String> {
        self.token(kind).map(|token| token.text().to_owned())
    }

    /// Whether a child token is of one of `kinds`.
    fn has_token(&self, kinds: &[SyntaxKind]) -> bool {
        let mut tokens = self.0.iter().filter_map(SyntaxElement::as_token);
        tokens.any(|token| kinds.contains(&token.kind()))
    }
}

/// The nodes that an accessor returned are exactly the expected ones: equal cursors with the same
/// parent, index, kind and range, in the same order. Equal empty nodes at one offset are equal
/// cursors, so the index tells them apart.
fn same_nodes(actual: impl Iterator<Item = SyntaxNode>, expected: &[SyntaxNode]) {
    let actual: Vec<SyntaxNode> = actual.collect();
    assert_eq!(actual, expected);
    let place = |node: &SyntaxNode| (node.parent(), node.index(), node.kind(), node.text_range());
    let actual: Vec<_> = actual.iter().map(place).collect();
    let expected: Vec<_> = expected.iter().map(place).collect();
    assert_eq!(actual, expected);
}

/// The token that an accessor returned is exactly the expected one, as [`same_nodes`] compares
/// nodes.
fn same_token(actual: Option<SyntaxToken>, expected: Option<SyntaxToken>) {
    assert_eq!(actual, expected);
    let place = |token: SyntaxToken| {
        let parent = token.parent();
        (parent, token.index(), token.kind(), token.text_range())
    };
    assert_eq!(actual.map(place), expected.map(place));
}
