//! Helpers for constructing green tree nodes programmatically.
//!
//! These are the building blocks for the mutation API — they create
//! well-formed INI syntax fragments that can be spliced into an existing
//! tree via rowan's `splice_children` / `replace_with`.

use rowan::{GreenNode, GreenNodeBuilder};

use crate::syntax_kind::SyntaxKind;

/// Build a KEY node wrapping an IDENT token.
#[must_use]
pub fn key_node(name: &str) -> GreenNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::KEY.into());
    builder.token(SyntaxKind::IDENT.into(), name);
    builder.finish_node();
    builder.finish()
}

/// Build a VALUE node wrapping a `VALUE_TEXT` token (or empty).
#[must_use]
pub fn value_node(text: &str) -> GreenNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::VALUE.into());
    if !text.is_empty() {
        builder.token(SyntaxKind::VALUE_TEXT.into(), text);
    }
    builder.finish_node();
    builder.finish()
}

/// Build a complete ENTRY node: `key = value\n`
#[must_use]
pub fn entry_node(key: &str, value: &str) -> GreenNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::ENTRY.into());
    // KEY
    builder.start_node(SyntaxKind::KEY.into());
    builder.token(SyntaxKind::IDENT.into(), key);
    builder.finish_node();
    // ws = ws
    builder.token(SyntaxKind::WHITESPACE.into(), " ");
    builder.token(SyntaxKind::EQ.into(), "=");
    builder.token(SyntaxKind::WHITESPACE.into(), " ");
    // VALUE
    builder.start_node(SyntaxKind::VALUE.into());
    if !value.is_empty() {
        builder.token(SyntaxKind::VALUE_TEXT.into(), value);
    }
    builder.finish_node();
    // newline
    builder.token(SyntaxKind::NEWLINE.into(), "\n");
    builder.finish_node();
    builder.finish()
}

/// Build a complete SECTION node: `[name]\n` (empty, no entries).
///
/// In the line-node model the `SECTION_HEADER` owns its terminating newline.
#[must_use]
pub fn empty_section_node(name: &str) -> GreenNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::SECTION.into());
    // header (owns its trailing newline)
    builder.start_node(SyntaxKind::SECTION_HEADER.into());
    builder.token(SyntaxKind::L_BRACK.into(), "[");
    builder.token(SyntaxKind::IDENT.into(), name);
    builder.token(SyntaxKind::R_BRACK.into(), "]");
    builder.token(SyntaxKind::NEWLINE.into(), "\n");
    builder.finish_node();
    builder.finish_node();
    builder.finish()
}

/// Build a `BLANK_LINE` node: a single newline.
#[must_use]
pub fn blank_line_node() -> GreenNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::BLANK_LINE.into());
    builder.token(SyntaxKind::NEWLINE.into(), "\n");
    builder.finish_node();
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax_kind::IniLang;
    use rowan::SyntaxNode;

    fn text_of(green: &GreenNode) -> String {
        SyntaxNode::<IniLang>::new_root(green.clone())
            .text()
            .to_string()
    }

    #[test]
    fn entry_node_renders_correctly() {
        let node = entry_node("host", "0.0.0.0");
        assert_eq!(text_of(&node), "host = 0.0.0.0\n");
    }

    #[test]
    fn entry_node_empty_value() {
        let node = entry_node("key", "");
        assert_eq!(text_of(&node), "key = \n");
    }

    #[test]
    fn empty_section_renders() {
        let node = empty_section_node("server");
        assert_eq!(text_of(&node), "[server]\n");
    }

    #[test]
    fn key_and_value_nodes() {
        assert_eq!(text_of(&key_node("foo")), "foo");
        assert_eq!(text_of(&value_node("bar")), "bar");
        assert_eq!(text_of(&value_node("")), "");
    }
}
