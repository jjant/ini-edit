//! The line-node shape that editor positions rely on, checked in debug builds.

use rowan::NodeOrToken;

use crate::syntax_kind::{IniLang, SyntaxKind, SyntaxNode};

/// The text of a single line break, as the lexer reads it.
const LINE_BREAKS: [&str; 3] = ["\n", "\r\n", "\r"];

/// Assert the shape that body-line positions rely on: the document under
/// `root` is a sequence of line nodes, and each one owns only its own line
/// break.
///
/// - The root holds sections and lines, and a section holds only lines.
/// - The only loose token is a byte order mark at the start of the root.
/// - A line holds at most one `NEWLINE` token. It is the line's last child,
///   and it holds one line break.
/// - A `NEWLINE` token that holds LF does not follow a CR. The lexer reads the
///   two as one CRLF line break, which joins two lines or takes the CR from
///   the text before it.
/// - Only the last line of the document may lack a line break.
///
/// Only `NEWLINE` tokens count as line breaks here: the `VALUE_TEXT` of a
/// value with continuation lines holds its own, which the check does not see,
/// except for a final CR right before an LF `NEWLINE` token.
///
/// The parser builds this shape and checks it on every parse; the editor keeps
/// it because every line it writes comes from the parser and its debug builds
/// check that the tree equals a reparse after every edit. The walk covers the
/// whole document, so only native debug builds compile it: tests, coverage,
/// mutation testing and fuzzing run it. Release builds skip it, and so does
/// Miri, which looks for undefined behavior rather than logic errors. The walk
/// reads the green tree, so it creates no cursor and copies no text.
#[cfg(all(debug_assertions, not(miri)))]
pub(crate) fn debug_assert_line_nodes(root: &SyntaxNode) {
    use rowan::Language;

    assert_eq!(
        root.kind(),
        SyntaxKind::ROOT,
        "a {:?} node in place of the root",
        root.kind()
    );
    let newline = rowan::SyntaxKind::from(SyntaxKind::NEWLINE);
    let mut after_carriage_return = false;
    let mut previous_is_unterminated = false;
    for line in line_nodes(root.green()) {
        assert!(
            !previous_is_unterminated,
            "only the last line may be unterminated"
        );
        let kind = IniLang::kind_from_raw(line.kind());
        assert!(
            matches!(
                kind,
                SyntaxKind::SECTION_HEADER
                    | SyntaxKind::ENTRY
                    | SyntaxKind::COMMENT_LINE
                    | SyntaxKind::BLANK_LINE
                    | SyntaxKind::ERROR_LINE
            ),
            "a {kind:?} node in place of a line"
        );
        let newlines = count_line_breaks(line, &mut after_carriage_return);
        assert!(newlines <= 1, "a line node holds {newlines} line breaks");
        let terminated = line
            .children()
            .last()
            .and_then(NodeOrToken::into_token)
            .is_some_and(|last| last.kind() == newline);
        assert_eq!(
            newlines,
            usize::from(terminated),
            "a line break that is not the last child of its line"
        );
        previous_is_unterminated = !terminated;
    }
}

/// The number of `NEWLINE` tokens in the green line node `line`. Each one
/// must hold one line break that does not join a CR before it.
/// `after_carriage_return` says whether the text before `line` ends in CR. It
/// is updated to say whether the text through `line` does.
#[cfg(all(debug_assertions, not(miri)))]
fn count_line_breaks(line: &rowan::GreenNodeData, after_carriage_return: &mut bool) -> usize {
    let newline = rowan::SyntaxKind::from(SyntaxKind::NEWLINE);
    let mut newlines = 0;
    for token in line_tokens(line) {
        let text = token.text();
        if token.kind() == newline {
            newlines += 1;
            assert!(
                LINE_BREAKS.contains(&text),
                "a NEWLINE token that is not one line break: {text:?}"
            );
            if *after_carriage_return {
                assert_ne!(
                    text, "\n",
                    "a CR and the LF NEWLINE token after it read as one line break"
                );
            }
        }
        // An empty token leaves the text before the next one as it was.
        if !text.is_empty() {
            *after_carriage_return = text.ends_with('\r');
        }
    }
    newlines
}

/// The lines of the document whose `ROOT` is the green node `root`, in
/// document order: the lines outside any section and the lines of each
/// section. A section holds only nodes, and the only loose token is a byte
/// order mark at the start of the root.
#[cfg(all(debug_assertions, not(miri)))]
fn line_nodes(root: &rowan::GreenNodeData) -> Vec<&rowan::GreenNodeData> {
    use rowan::Language;

    let section = rowan::SyntaxKind::from(SyntaxKind::SECTION);
    let mut lines = Vec::new();
    for (index, element) in root.children().enumerate() {
        match element {
            NodeOrToken::Token(token) => {
                assert_eq!(index, 0, "a loose token after the start of the document");
                assert_eq!(
                    IniLang::kind_from_raw(token.kind()),
                    SyntaxKind::WHITESPACE,
                    "a loose token that is not whitespace"
                );
                assert_eq!(
                    token.text(),
                    "\u{FEFF}",
                    "a loose token that is not a byte order mark"
                );
            }
            NodeOrToken::Node(node) if node.kind() == section => {
                for line in node.children() {
                    let line = line.into_node();
                    assert!(line.is_some(), "a loose token in a section");
                    lines.extend(line);
                }
            }
            NodeOrToken::Node(line) => lines.push(line),
        }
    }
    lines
}

/// The tokens of the green line node `line`, in document order: its own and
/// those of its KEY and VALUE nodes. Those nodes hold only tokens, so the walk
/// goes two levels deep and no further.
#[cfg(all(debug_assertions, not(miri)))]
fn line_tokens(line: &rowan::GreenNodeData) -> impl Iterator<Item = &rowan::GreenTokenData> {
    line.children().flat_map(|child| {
        let nested = child.into_node().into_iter().flat_map(|node| {
            node.children().map(|grandchild| {
                grandchild
                    .into_token()
                    .expect("the nodes in a line hold only tokens")
            })
        });
        child.into_token().into_iter().chain(nested)
    })
}

// No parse or edit breaks these invariants, so only a test can show that the
// assertion guarding each one is live. Each `should_panic` test below breaks
// one invariant in a tree built by hand and expects that assertion's message.
#[cfg(all(test, debug_assertions, not(miri)))]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    type GreenElement = NodeOrToken<rowan::GreenNode, rowan::GreenToken>;

    fn token(kind: SyntaxKind, text: &str) -> GreenElement {
        NodeOrToken::Token(rowan::GreenToken::new(kind.into(), text))
    }

    fn node(kind: SyntaxKind, children: Vec<GreenElement>) -> GreenElement {
        NodeOrToken::Node(rowan::GreenNode::new(kind.into(), children))
    }

    fn key(text: &str) -> GreenElement {
        node(SyntaxKind::KEY, vec![token(SyntaxKind::IDENT, text)])
    }

    /// A `ROOT` node holding `children`.
    fn root_of(children: Vec<GreenElement>) -> SyntaxNode {
        SyntaxNode::new_root(rowan::GreenNode::new(SyntaxKind::ROOT.into(), children))
    }

    /// A `COMMENT_LINE` holding `;`, ended by `newline` if it is not empty.
    fn comment_line(newline: &str) -> GreenElement {
        let mut tokens = vec![token(SyntaxKind::COMMENT, ";")];
        if !newline.is_empty() {
            tokens.push(token(SyntaxKind::NEWLINE, newline));
        }
        node(SyntaxKind::COMMENT_LINE, tokens)
    }

    /// A `BLANK_LINE` holding only `newline`.
    fn blank_line(newline: &str) -> GreenElement {
        node(
            SyntaxKind::BLANK_LINE,
            vec![token(SyntaxKind::NEWLINE, newline)],
        )
    }

    /// A `COMMENT_LINE` with an empty `COMMENT` token and `newline`.
    fn empty_comment_line(newline: &str) -> GreenElement {
        node(
            SyntaxKind::COMMENT_LINE,
            vec![
                token(SyntaxKind::COMMENT, ""),
                token(SyntaxKind::NEWLINE, newline),
            ],
        )
    }

    #[test]
    fn the_line_node_check_accepts_every_parsed_shape() {
        // A byte order mark, lines before the first section, all three line
        // breaks, a continued value, a malformed line, an empty section and
        // an unterminated last line.
        let source = "\u{FEFF}k=1\r\n\n[a]\rx=\n=bad\r\nv = a \\\nb\n[b]\n[c]\n; note\nlast";
        debug_assert_line_nodes(&crate::parse(source).syntax());
        debug_assert_line_nodes(&root_of(Vec::new()));
        debug_assert_line_nodes(&root_of(vec![token(SyntaxKind::WHITESPACE, "\u{FEFF}")]));
        // After a CR, a NEWLINE token can hold CR or CRLF, also past an empty
        // token: `;\r\r\r\n` is three lines.
        let lines = vec![
            comment_line("\r"),
            blank_line("\r"),
            empty_comment_line("\r\n"),
        ];
        debug_assert_line_nodes(&root_of(lines));
    }

    #[test]
    #[should_panic(expected = "a SECTION node in place of the root")]
    fn the_line_node_check_starts_at_the_root() {
        let root = root_of(vec![node(SyntaxKind::SECTION, Vec::new())]);
        debug_assert_line_nodes(&root.first_child().unwrap());
    }

    #[test]
    #[should_panic(expected = "a loose token after the start of the document")]
    fn the_line_node_check_rejects_a_loose_token_after_the_first_line() {
        let byte_order_mark = token(SyntaxKind::WHITESPACE, "\u{FEFF}");
        debug_assert_line_nodes(&root_of(vec![comment_line("\n"), byte_order_mark]));
    }

    #[test]
    #[should_panic(expected = "a loose token that is not whitespace")]
    fn the_line_node_check_rejects_a_loose_token_of_another_kind() {
        debug_assert_line_nodes(&root_of(vec![token(SyntaxKind::COMMENT, "\u{FEFF}")]));
    }

    #[test]
    #[should_panic(expected = "a loose token that is not a byte order mark")]
    fn the_line_node_check_rejects_loose_whitespace() {
        debug_assert_line_nodes(&root_of(vec![token(SyntaxKind::WHITESPACE, " ")]));
    }

    #[test]
    #[should_panic(expected = "a loose token in a section")]
    fn the_line_node_check_rejects_a_loose_token_in_a_section() {
        let section = node(
            SyntaxKind::SECTION,
            vec![comment_line("\n"), token(SyntaxKind::NEWLINE, "\n")],
        );
        debug_assert_line_nodes(&root_of(vec![section]));
    }

    #[test]
    #[should_panic(expected = "a KEY node in place of a line")]
    fn the_line_node_check_rejects_a_node_that_is_not_a_line() {
        debug_assert_line_nodes(&root_of(vec![key("k")]));
    }

    #[test]
    #[should_panic(expected = "a line node holds 2 line breaks")]
    fn the_line_node_check_rejects_a_line_with_two_line_breaks() {
        let newline = || token(SyntaxKind::NEWLINE, "\n");
        let blank_line = node(SyntaxKind::BLANK_LINE, vec![newline(), newline()]);
        debug_assert_line_nodes(&root_of(vec![blank_line]));
    }

    #[test]
    #[should_panic(expected = "a line break that is not the last child of its line")]
    fn the_line_node_check_rejects_a_line_break_before_the_end_of_its_line() {
        let line = node(
            SyntaxKind::COMMENT_LINE,
            vec![
                token(SyntaxKind::NEWLINE, "\n"),
                token(SyntaxKind::COMMENT, ";"),
            ],
        );
        debug_assert_line_nodes(&root_of(vec![line]));
    }

    #[test]
    #[should_panic(expected = "a NEWLINE token that is not one line break")]
    fn the_line_node_check_rejects_a_newline_token_with_two_line_breaks() {
        debug_assert_line_nodes(&root_of(vec![comment_line("\n\n")]));
    }

    #[test]
    #[should_panic(expected = "a CR and the LF NEWLINE token after it read as one line break")]
    fn the_line_node_check_rejects_an_lf_line_break_after_a_cr() {
        // `;\r\n` reads as one line, not as a comment line and a blank line.
        debug_assert_line_nodes(&root_of(vec![comment_line("\r"), blank_line("\n")]));
    }

    #[test]
    #[should_panic(expected = "a CR and the LF NEWLINE token after it read as one line break")]
    fn the_line_node_check_sees_a_cr_past_empty_tokens() {
        let lines = vec![comment_line("\r"), empty_comment_line("\n")];
        debug_assert_line_nodes(&root_of(lines));
    }

    #[test]
    #[should_panic(expected = "a CR and the LF NEWLINE token after it read as one line break")]
    fn the_line_node_check_sees_a_cr_at_the_end_of_a_key() {
        // `k\r\n` reads as the key `k` and a CRLF line break.
        let entry = node(
            SyntaxKind::ENTRY,
            vec![key("k\r"), token(SyntaxKind::NEWLINE, "\n")],
        );
        debug_assert_line_nodes(&root_of(vec![entry]));
    }

    #[test]
    #[should_panic(expected = "the nodes in a line hold only tokens")]
    fn the_line_node_check_rejects_a_node_nested_in_a_key() {
        let entry = node(
            SyntaxKind::ENTRY,
            vec![node(SyntaxKind::KEY, vec![key("k")])],
        );
        debug_assert_line_nodes(&root_of(vec![entry]));
    }

    #[test]
    #[should_panic(expected = "only the last line may be unterminated")]
    fn the_line_node_check_rejects_an_unterminated_line_before_another() {
        debug_assert_line_nodes(&root_of(vec![comment_line(""), comment_line("")]));
    }
}
