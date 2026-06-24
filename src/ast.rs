//! Typed AST wrappers over the lossless syntax tree.
//!
//! Each wrapper checks the node's `SyntaxKind` at runtime via `cast`.
//! Accessors return `Option` because the tree may be malformed.

use crate::syntax_kind::{SyntaxKind, SyntaxNode, SyntaxToken};

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
        self.0.children().filter_map(Entry::cast)
    }

    /// All sections in document order.
    pub fn sections(&self) -> impl Iterator<Item = Section> + '_ {
        self.0.children().filter_map(Section::cast)
    }
}

impl Section {
    /// The section header node.
    #[must_use]
    pub fn header(&self) -> Option<SectionHeader> {
        self.0.children().find_map(SectionHeader::cast)
    }

    /// The section name.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.header()?.name()
    }

    /// Entries in this section.
    pub fn entries(&self) -> impl Iterator<Item = Entry> + '_ {
        self.0.children().filter_map(Entry::cast)
    }

    /// Full-line comments in this section (in document order).
    pub fn comment_lines(&self) -> impl Iterator<Item = CommentLine> + '_ {
        self.0.children().filter_map(CommentLine::cast)
    }
}

impl SectionHeader {
    /// The IDENT token between the brackets.
    #[must_use]
    pub fn name_token(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .find(|t| t.kind() == SyntaxKind::IDENT)
    }

    /// The section name as a string.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.name_token().map(|t| t.text().to_string())
    }
}

impl Entry {
    /// The KEY node.
    #[must_use]
    pub fn key_node(&self) -> Option<Key> {
        self.0.children().find_map(Key::cast)
    }

    /// The key text.
    #[must_use]
    pub fn key(&self) -> Option<String> {
        self.key_node()?.text()
    }

    /// The VALUE node.
    #[must_use]
    pub fn value_node(&self) -> Option<Value> {
        self.0.children().find_map(Value::cast)
    }

    /// The value text. Returns `Some("")` for empty values (`key =`),
    /// `None` for bare keys without a separator (`key` alone).
    #[must_use]
    pub fn value(&self) -> Option<String> {
        // A bare key has no EQ or COLON token — distinguish from empty value
        let has_separator = self
            .0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .any(|t| t.kind() == SyntaxKind::EQ || t.kind() == SyntaxKind::COLON);
        if !has_separator {
            return None;
        }
        self.value_node().map(|v| v.text().unwrap_or_default())
    }

    /// True if the separator is `:` rather than `=`.
    #[must_use]
    pub fn uses_colon(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .any(|t| t.kind() == SyntaxKind::COLON)
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
        self.0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .find(|t| t.kind() == SyntaxKind::COMMENT)
            .map(|t| t.text().to_string())
    }
}

impl Key {
    /// The underlying IDENT token.
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .find(|t| t.kind() == SyntaxKind::IDENT)
    }

    /// The key as a string.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.token().map(|t| t.text().to_string())
    }
}

impl Value {
    /// The underlying `VALUE_TEXT` token.
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .find(|t| t.kind() == SyntaxKind::VALUE_TEXT)
    }

    /// The value text, or `None` for empty values (`key =`).
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.token().map(|t| t.text().to_string())
    }
}

impl CommentLine {
    /// The `COMMENT` token (including its leading `;`/`#` marker).
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .find(|t| t.kind() == SyntaxKind::COMMENT)
    }

    /// The comment text, including its leading `;`/`#` marker.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.token().map(|t| t.text().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn ast(input: &str) -> File {
        File::cast(parse(input).syntax()).unwrap()
    }

    #[test]
    fn sections_and_entries() {
        let f = ast("[a]\nx=1\ny=2\n[b]\nz=3\n");
        let secs: Vec<_> = f.sections().collect();
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].name().as_deref(), Some("a"));
        assert_eq!(secs[1].name().as_deref(), Some("b"));
        let entries: Vec<_> = secs[0].entries().collect();
        assert_eq!(entries[0].key().as_deref(), Some("x"));
        assert_eq!(entries[0].value().as_deref(), Some("1"));
    }

    #[test]
    fn empty_value() {
        let f = ast("[s]\nk=\n");
        let e = f.sections().next().unwrap().entries().next().unwrap();
        assert_eq!(e.value().as_deref(), Some(""));
    }

    #[test]
    fn colon_separator() {
        let f = ast("[s]\nk: v\n");
        let e = f.sections().next().unwrap().entries().next().unwrap();
        assert!(e.uses_colon());
        assert_eq!(e.value().as_deref(), Some("v"));
    }

    #[test]
    fn preamble() {
        let f = ast("g=1\n[s]\nk=2\n");
        assert_eq!(f.preamble_entries().count(), 1);
        assert_eq!(f.sections().next().unwrap().entries().count(), 1);
    }

    #[test]
    fn inline_comment_accessor() {
        use crate::{ParseOptions, parse_with};
        let opts = ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        let f = File::cast(parse_with("[s]\nk = 1   ; note\n", &opts).syntax()).unwrap();
        let e = f.sections().next().unwrap().entries().next().unwrap();
        assert_eq!(e.key().as_deref(), Some("k"));
        assert_eq!(e.value().as_deref(), Some("1"));
        assert_eq!(e.inline_comment().as_deref(), Some("; note"));
    }

    #[test]
    fn no_inline_comment_when_disabled() {
        // Default parse leaves the marker in the value and reports no comment.
        let f = ast("[s]\nk = 1   ; note\n");
        let e = f.sections().next().unwrap().entries().next().unwrap();
        assert_eq!(e.value().as_deref(), Some("1   ; note"));
        assert_eq!(e.inline_comment(), None);
    }
}
