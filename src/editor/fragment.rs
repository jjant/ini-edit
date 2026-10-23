//! Reading text the way the parser reads it in the middle of a document.
//!
//! Every check the editor makes is the crate's own lexer or parser (guarantee C5): an edit renders
//! the exact text it will write, parses it here, and inserts the parser's own nodes. Line breaks
//! are read with the lexer's own `line_break_len`, which decides rules T and D and the document's
//! line ending. The two plain string functions restate lexer rules, and tests pair each with the
//! lexer: [`line_continues`] with the lines a value continues over, and [`final_line`] with the
//! line the lexer searches for an inline comment and ends a value at.

use crate::lexer;
use crate::syntax_kind::{SyntaxKind, SyntaxNode};
use crate::{ParseOptions, parse_with};

/// The terminator written where another one would read together with a neighboring CR or LF.
pub(super) const CRLF: &str = "\r\n";

/// The three line breaks the lexer reads: every line ending and terminator is one of them.
pub(super) const LINE_BREAKS: [&str; 3] = ["\n", CRLF, "\r"];

/// The kinds of the lines of a body: what the preamble and a section hold after the header.
pub(super) const BODY_LINE_KINDS: [SyntaxKind; 4] = [
    SyntaxKind::ENTRY,
    SyntaxKind::COMMENT_LINE,
    SyntaxKind::BLANK_LINE,
    SyntaxKind::ERROR_LINE,
];

/// The kinds of every line node: a section header, or a body line.
pub(super) const LINE_KINDS: [SyntaxKind; 5] = [
    SyntaxKind::SECTION_HEADER,
    SyntaxKind::ENTRY,
    SyntaxKind::COMMENT_LINE,
    SyntaxKind::BLANK_LINE,
    SyntaxKind::ERROR_LINE,
];

/// The root children the parser makes of `text` when `text` starts at a line start in the middle
/// of a document: line nodes, and a `SECTION` for each header.
///
/// The lexer carries no state from one line to the next except one rule: a document that starts
/// with U+FEFF starts with a byte order mark. A line break in front of `text` puts it at a line
/// start in the middle of a document, where that rule cannot apply, so `text` lexes and parses
/// exactly as the same lines do inside a document. The line break then parses to a blank line of
/// its own, the root's first child, because a lone LF ends a line whatever follows it; that blank
/// line is dropped.
///
/// The parsed text is one byte longer than `text`; the editor's size limit leaves room for it.
pub(super) fn parse_fragment(text: &str, options: &ParseOptions) -> Vec<SyntaxNode> {
    // Callers check sizes before parsing, so the probe fits the parser's limit (`LEN_MAX`).
    assert!(text.len() <= super::LEN_MAX);
    let root = parse_with(&format!("\n{text}"), options).syntax();
    let mut children = root.children();
    let blank = children
        .next()
        .expect("a leading line break is a blank line");
    assert_eq!(blank.kind(), SyntaxKind::BLANK_LINE);
    assert_eq!(blank.text(), "\n");
    let fragment: Vec<SyntaxNode> = children.collect();
    // Only a byte order mark is a token at the root, and the leading line break rules it out.
    assert_eq!(root.children_with_tokens().count(), fragment.len() + 1);
    fragment
}

/// The line nodes of [`parse_fragment`], with each `SECTION` replaced by its lines: the parser's
/// own nodes, whose green nodes a commit puts into the editor's tree.
pub(super) fn fragment_lines(text: &str, options: &ParseOptions) -> Vec<SyntaxNode> {
    let mut lines = Vec::new();
    for child in parse_fragment(text, options) {
        if child.kind() == SyntaxKind::SECTION {
            lines.extend(child.children());
        } else {
            lines.push(child);
        }
    }
    // Lines only, and together exactly the text: the parse is lossless.
    assert!(lines.iter().all(|line| LINE_KINDS.contains(&line.kind())));
    let len: usize = lines
        .iter()
        .map(|line| usize::from(line.text_range().len()))
        .sum();
    assert_eq!(len, text.len());
    lines
}

/// Whether text ending with the character `before`, followed by text starting with `after`, reads
/// one line break across the boundary: a CR followed by an LF, which the lexer reads as one CRLF
/// wherever it reads line breaks. `None` stands for the start or the end of the document.
pub(super) fn joins_line_breaks(before: Option<char>, after: Option<char>) -> bool {
    let boundary: String = before.into_iter().chain(after).collect();
    let joins = lexer::line_break_len(&boundary) == CRLF.len();
    // The lexer's decision, paired with the rule it implements: only a CR then an LF join.
    if joins {
        assert_eq!(before, Some('\r'));
        assert_eq!(after, Some('\n'));
    }
    joins
}

/// The line break at the start of `text`, as the lexer reads it: `"\r\n"`, `"\r"` or `"\n"`.
pub(super) fn line_break_at(text: &str) -> &'static str {
    let len = lexer::line_break_len(text);
    assert!(len > 0);
    match &text[..len] {
        "\r\n" => CRLF,
        "\r" => "\r",
        line_break => {
            assert_eq!(line_break, "\n");
            "\n"
        }
    }
}

/// The text after the last CR or LF of `text`: the last line of a continued value, or all of
/// `text` without a line break. This is the line the lexer searches for an inline comment.
pub(super) fn final_line(text: &str) -> &str {
    let start = text
        .rfind(['\r', '\n'])
        .map_or(0, |line_break| line_break + 1);
    let line = &text[start..];
    assert!(!line.contains(['\r', '\n']));
    line
}

/// Whether `line`, a line without a line break, ends with a continuation backslash: the lexer
/// continues a value onto the next line when its line ends with `\`, ignoring trailing spaces and
/// tabs.
pub(super) fn line_continues(line: &str) -> bool {
    assert!(!line.contains(['\r', '\n']));
    line.trim_end_matches([' ', '\t']).ends_with('\\')
}
