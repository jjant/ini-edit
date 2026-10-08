//! Structural oracle: every physical line is exactly one CST node.

use ini_edit::{SyntaxElement, SyntaxKind, SyntaxNode};

/// Editor line indices address children of the root and of each section, so
/// every physical line must be a single node owning exactly its terminator.
/// Only a leading BOM may be a loose token.
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
        let newlines: Vec<_> = line
            .descendants_with_tokens()
            .filter_map(SyntaxElement::into_token)
            .filter(|token| token.kind() == SyntaxKind::NEWLINE)
            .collect();
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
            newlines.len() <= 1 && newlines.len() == usize::from(ends_with_newline),
            "{source:?}: line {index} must own only its own terminator"
        );
    }
}
