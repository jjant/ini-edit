//! Structural oracle: every line is exactly one CST node. A line is one line
//! node, so an entry whose value continues over several physical lines with
//! trailing backslashes is one line.

use ini_edit::{SyntaxElement, SyntaxKind, SyntaxNode};

/// Editor positions address the line nodes of the root and of each section, so
/// every line must be a single node owning exactly its terminator. Only a
/// leading BOM may be a loose token. Line breaks occur only as `NEWLINE`
/// tokens and, after a continuation backslash, inside `VALUE_TEXT`.
pub fn check(source: &str, root: &SyntaxNode) {
    let mut lines = Vec::new();
    for (index, element) in root.children_with_tokens().enumerate() {
        match element.as_node() {
            Some(node) if node.kind() == SyntaxKind::SECTION => {
                for line in node.children_with_tokens() {
                    let line = line.into_node();
                    assert!(line.is_some(), "{source:?}: loose token in a section");
                    lines.extend(line);
                }
            }
            Some(node) => lines.push(node.clone()),
            None => assert!(
                index == 0 && element.to_string() == "\u{FEFF}",
                "{source:?}: loose token at the root"
            ),
        }
    }
    let count = lines.len();
    for (index, line) in lines.into_iter().enumerate() {
        let tokens: Vec<_> = line
            .descendants_with_tokens()
            .filter_map(SyntaxElement::into_token)
            .collect();
        let newlines = tokens
            .iter()
            .filter(|token| token.kind() == SyntaxKind::NEWLINE)
            .count();
        let ends_with_newline = line
            .last_token()
            .is_some_and(|token| token.kind() == SyntaxKind::NEWLINE);
        if index + 1 < count {
            assert!(
                ends_with_newline,
                "{source:?}: line {index} is unterminated"
            );
        }
        assert!(
            newlines <= 1 && newlines == usize::from(ends_with_newline),
            "{source:?}: line {index} must own only its own terminator"
        );
        for token in &tokens {
            let text = token.text();
            match token.kind() {
                SyntaxKind::NEWLINE => {}
                SyntaxKind::VALUE_TEXT => assert_breaks_follow_continuations(source, text),
                _ => assert!(
                    !text.contains(['\r', '\n']),
                    "{source:?}: line {index} has a line break in a {:?} token",
                    token.kind()
                ),
            }
        }
    }
}

/// Every line break inside a value directly follows a backslash, with only
/// spaces and tabs between them.
fn assert_breaks_follow_continuations(source: &str, value: &str) {
    let mut rest = value;
    while let Some(line_break) = rest.find(['\r', '\n']) {
        let line = &rest[..line_break];
        assert!(
            line.trim_end_matches([' ', '\t']).ends_with('\\'),
            "{source:?}: a value breaks a line without a continuation: {value:?}"
        );
        let width = if rest[line_break..].starts_with("\r\n") {
            2
        } else {
            1
        };
        rest = &rest[line_break + width..];
    }
}
