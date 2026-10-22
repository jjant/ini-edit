//! Builders for every syntax fragment the editor adds to a tree.
//!
//! The [`Editor`](crate::editor::Editor) creates entries, sections, blank and
//! raw lines, and loose tokens only through these functions, so the fragments
//! it splices into a tree have the shapes the parser produces. Nodes are built
//! as green trees. Loose tokens come out as mutable elements outside any
//! document, ready for rowan's `splice_children`.

use rowan::{GreenNode, GreenToken, NodeOrToken};

use crate::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// A child of a green node.
type GreenElement = NodeOrToken<GreenNode, GreenToken>;

/// The whitespace written before and after an `=` or `:` separator. An empty
/// side writes no whitespace token.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SeparatorGap<'a> {
    /// Whitespace between the key and the separator.
    pub(crate) before: &'a str,
    /// Whitespace between the separator and the value.
    pub(crate) after: &'a str,
}

fn green_token(kind: SyntaxKind, text: &str) -> GreenElement {
    NodeOrToken::Token(GreenToken::new(kind.into(), text))
}

fn green_node(kind: SyntaxKind, children: Vec<GreenElement>) -> GreenNode {
    GreenNode::new(kind.into(), children)
}

/// Build a KEY node wrapping an IDENT token.
#[must_use]
pub(crate) fn key_node(name: &str) -> GreenNode {
    green_node(SyntaxKind::KEY, vec![green_token(SyntaxKind::IDENT, name)])
}

/// Build a VALUE node wrapping a `VALUE_TEXT` token, or an empty VALUE node
/// when `text` is empty.
#[must_use]
pub(crate) fn value_node(text: &str) -> GreenNode {
    let children = if text.is_empty() {
        Vec::new()
    } else {
        vec![green_token(SyntaxKind::VALUE_TEXT, text)]
    };
    green_node(SyntaxKind::VALUE, children)
}

/// The terminator for a line whose content ends with `text`, in a document
/// whose lines end with `ending`. A trailing CR in the content must stay
/// separate from an LF terminator, so LF documents use CRLF there. CR and CRLF
/// terminators already follow a CR as a separate line break.
#[must_use]
pub(crate) fn newline_after(text: &str, ending: &'static str) -> &'static str {
    if ending == "\n" && text.ends_with('\r') {
        "\r\n"
    } else {
        ending
    }
}

/// The tokens of a separator: `separator` with the nonempty sides of `gap`
/// around it.
fn separator_children(separator: SyntaxKind, gap: SeparatorGap<'_>) -> Vec<GreenElement> {
    let text = if separator == SyntaxKind::COLON {
        ":"
    } else {
        "="
    };
    let mut children = Vec::with_capacity(3);
    if !gap.before.is_empty() {
        children.push(green_token(SyntaxKind::WHITESPACE, gap.before));
    }
    children.push(green_token(separator, text));
    if !gap.after.is_empty() {
        children.push(green_token(SyntaxKind::WHITESPACE, gap.after));
    }
    children
}

/// Build a complete ENTRY node: `key`, an `=` with `gap` around it, and
/// `value`, terminated with the document's line `ending`.
#[must_use]
pub(crate) fn entry_node(
    key: &str,
    value: &str,
    gap: SeparatorGap<'_>,
    ending: &'static str,
) -> GreenNode {
    let mut children = vec![NodeOrToken::Node(key_node(key))];
    children.extend(separator_children(SyntaxKind::EQ, gap));
    children.push(NodeOrToken::Node(value_node(value)));
    children.push(green_token(
        SyntaxKind::NEWLINE,
        newline_after(value, ending),
    ));
    green_node(SyntaxKind::ENTRY, children)
}

/// Build a complete SECTION node: `[name]` and the line `ending`, with no
/// entries. In the line-node model the `SECTION_HEADER` owns its terminator.
#[must_use]
pub(crate) fn empty_section_node(name: &str, ending: &'static str) -> GreenNode {
    let header = green_node(
        SyntaxKind::SECTION_HEADER,
        vec![
            green_token(SyntaxKind::L_BRACK, "["),
            green_token(SyntaxKind::IDENT, name),
            green_token(SyntaxKind::R_BRACK, "]"),
            green_token(SyntaxKind::NEWLINE, ending),
        ],
    );
    green_node(SyntaxKind::SECTION, vec![NodeOrToken::Node(header)])
}

/// Build a `BLANK_LINE` node: a single line `ending`.
#[must_use]
pub(crate) fn blank_line_node(ending: &'static str) -> GreenNode {
    green_node(
        SyntaxKind::BLANK_LINE,
        vec![green_token(SyntaxKind::NEWLINE, ending)],
    )
}

/// Build a `COMMENT_LINE` node that holds `line` verbatim. The text before its
/// line break is one opaque `COMMENT` token, which preserves any content, and
/// the line break is its `NEWLINE`. A line without one receives `ending`.
#[must_use]
pub(crate) fn raw_line_node(line: &str, ending: &'static str) -> GreenNode {
    let (content, newline) = split_line_break(line).unwrap_or((line, ending));
    green_node(
        SyntaxKind::COMMENT_LINE,
        vec![
            green_token(SyntaxKind::COMMENT, content),
            green_token(SyntaxKind::NEWLINE, newline),
        ],
    )
}

/// `line` split before its final line break (`\r\n`, `\n`, or `\r`), if it
/// ends with one.
fn split_line_break(line: &str) -> Option<(&str, &str)> {
    if let Some(content) = line.strip_suffix("\r\n") {
        Some((content, "\r\n"))
    } else if let Some(content) = line.strip_suffix('\n') {
        Some((content, "\n"))
    } else {
        line.strip_suffix('\r').map(|content| (content, "\r"))
    }
}

/// Build a `COMMENT_LINE` node from the tokens of an inline comment (the
/// whitespace before it and the `COMMENT` itself) and a `newline`.
#[must_use]
pub(crate) fn comment_line_node(comment_tokens: &[SyntaxToken], newline: &str) -> GreenNode {
    let mut children: Vec<GreenElement> = comment_tokens
        .iter()
        .map(|token| NodeOrToken::Token(token.green().to_owned()))
        .collect();
    children.push(green_token(SyntaxKind::NEWLINE, newline));
    green_node(SyntaxKind::COMMENT_LINE, children)
}

/// Mutable copies of `node`'s children, outside any document, ready to splice
/// into an editor's tree. Splicing moves each one out of its fresh parent.
#[must_use]
pub(crate) fn detached_children(node: GreenNode) -> Vec<SyntaxElement> {
    SyntaxNode::new_root_mut(node)
        .children_with_tokens()
        .collect()
}

/// Mutable loose tokens, outside any document. Rowan creates a token handle
/// only as the child of a node, so the tokens are built inside a throwaway
/// `ROOT` node and taken out of it. That container never joins a document:
/// splicing moves the tokens out of it.
fn detached_tokens(tokens: Vec<GreenElement>) -> Vec<SyntaxElement> {
    detached_children(green_node(SyntaxKind::ROOT, tokens))
}

fn detached_token(kind: SyntaxKind, text: &str) -> SyntaxElement {
    let mut tokens = detached_tokens(vec![green_token(kind, text)]);
    tokens.pop().expect("the container holds the one token")
}

/// A loose `NEWLINE` token with the terminator `text`.
#[must_use]
pub(crate) fn newline_token(text: &str) -> SyntaxElement {
    detached_token(SyntaxKind::NEWLINE, text)
}

/// A loose `WHITESPACE` token.
#[must_use]
pub(crate) fn whitespace_token(text: &str) -> SyntaxElement {
    detached_token(SyntaxKind::WHITESPACE, text)
}

/// Loose separator tokens: `separator` with the nonempty sides of `gap` around
/// it.
#[must_use]
pub(crate) fn separator_tokens(separator: SyntaxKind, gap: SeparatorGap<'_>) -> Vec<SyntaxElement> {
    detached_tokens(separator_children(separator, gap))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    const SPACED: SeparatorGap<'static> = SeparatorGap {
        before: " ",
        after: " ",
    };

    fn text_of(green: &GreenNode) -> String {
        SyntaxNode::new_root(green.clone()).text().to_string()
    }

    fn texts_of(elements: &[SyntaxElement]) -> Vec<(SyntaxKind, String)> {
        elements
            .iter()
            .map(|element| {
                let token = element.as_token().expect("a loose token");
                (token.kind(), token.text().to_owned())
            })
            .collect()
    }

    #[test]
    fn entry_node_renders_correctly() {
        let node = entry_node("host", "0.0.0.0", SPACED, "\n");
        assert_eq!(text_of(&node), "host = 0.0.0.0\n");
        let node = entry_node("host", "0.0.0.0", SPACED, "\r\n");
        assert_eq!(text_of(&node), "host = 0.0.0.0\r\n");
    }

    #[test]
    fn entry_node_empty_value() {
        let node = entry_node("key", "", SPACED, "\n");
        assert_eq!(text_of(&node), "key = \n");
    }

    #[test]
    fn entry_node_has_the_parsed_shape() {
        let gap = SeparatorGap {
            before: "",
            after: "\t",
        };
        let built = SyntaxNode::new_root(entry_node("k", "v", gap, "\n"));
        let parsed = crate::parse("k=\tv\n").syntax().first_child().unwrap();
        assert_eq!(built.green(), parsed.green());
    }

    #[test]
    fn empty_section_renders() {
        let node = empty_section_node("server", "\n");
        assert_eq!(text_of(&node), "[server]\n");
        let node = empty_section_node("server", "\r");
        assert_eq!(text_of(&node), "[server]\r");
        assert_eq!(text_of(&blank_line_node("\r\n")), "\r\n");
    }

    #[test]
    fn a_trailing_carriage_return_never_joins_an_lf_terminator() {
        assert_eq!(newline_after("value", "\n"), "\n");
        assert_eq!(newline_after("head \\\r", "\n"), "\r\n");
        assert_eq!(newline_after("value", "\r"), "\r");
        assert_eq!(newline_after("head \\\r", "\r"), "\r");
        assert_eq!(newline_after("value", "\r\n"), "\r\n");
        assert_eq!(newline_after("head \\\r", "\r\n"), "\r\n");
    }

    #[test]
    fn key_and_value_nodes() {
        assert_eq!(text_of(&key_node("foo")), "foo");
        assert_eq!(text_of(&value_node("bar")), "bar");
        assert_eq!(text_of(&value_node("")), "");
        assert_eq!(detached_children(value_node("")).len(), 0);
    }

    #[test]
    fn raw_lines_keep_their_own_line_break() {
        for (line, content, newline) in [
            ("x", "x", "\r"),
            ("x\n", "x", "\n"),
            ("x\r\n", "x", "\r\n"),
            ("x\r", "x", "\r"),
            ("x\n\r", "x\n", "\r"),
            ("x\r\r\n", "x\r", "\r\n"),
            ("", "", "\r"),
        ] {
            let node = SyntaxNode::new_root(raw_line_node(line, "\r"));
            assert_eq!(node.kind(), SyntaxKind::COMMENT_LINE);
            let tokens: Vec<_> = node
                .children_with_tokens()
                .map(|element| {
                    let token = element.into_token().unwrap();
                    (token.kind(), token.text().to_owned())
                })
                .collect();
            assert_eq!(
                tokens,
                [
                    (SyntaxKind::COMMENT, content.to_owned()),
                    (SyntaxKind::NEWLINE, newline.to_owned()),
                ],
                "{line:?}"
            );
        }
    }

    #[test]
    fn a_moved_inline_comment_keeps_its_tokens() {
        let options = crate::ParseOptions {
            inline_comments: true,
            ..Default::default()
        };
        let parse = crate::parse_with("k=v \t; note", &options);
        let comment_tokens: Vec<_> = parse
            .syntax()
            .descendants_with_tokens()
            .filter_map(NodeOrToken::into_token)
            .skip_while(|token| token.kind() != SyntaxKind::VALUE_TEXT)
            .skip(1)
            .collect();
        let line = SyntaxNode::new_root(comment_line_node(&comment_tokens, "\r\n"));
        let parsed = crate::parse(" \t; note\r\n");
        assert_eq!(line.green(), parsed.syntax().first_child().unwrap().green());
    }

    #[test]
    fn loose_tokens_are_mutable_and_outside_any_document() {
        let newline = newline_token("\r\n");
        let whitespace = whitespace_token(" \t");
        for element in [&newline, &whitespace] {
            let container = element.parent().expect("a token in its container");
            assert!(container.is_mutable());
            assert!(container.parent().is_none());
        }
        assert_eq!(
            texts_of(&[newline, whitespace]),
            [
                (SyntaxKind::NEWLINE, "\r\n".to_owned()),
                (SyntaxKind::WHITESPACE, " \t".to_owned()),
            ]
        );
        assert_eq!(
            texts_of(&separator_tokens(SyntaxKind::COLON, SPACED)),
            [
                (SyntaxKind::WHITESPACE, " ".to_owned()),
                (SyntaxKind::COLON, ":".to_owned()),
                (SyntaxKind::WHITESPACE, " ".to_owned()),
            ]
        );
        let compact = SeparatorGap {
            before: "",
            after: "",
        };
        assert_eq!(
            texts_of(&separator_tokens(SyntaxKind::EQ, compact)),
            [(SyntaxKind::EQ, "=".to_owned())]
        );
    }
}
