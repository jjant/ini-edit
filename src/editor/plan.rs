//! Plans and commits. Every edit is planned first, which can fail but changes nothing, and
//! committed after, which cannot fail, so a refused edit leaves the document unchanged (C2).
//!
//! The boundary rules live here too:
//! - rule T, the terminators of new lines: [`terminators`];
//! - rule C, terminating the document's unterminated last line: [`Editor::terminate`];
//! - rule B, U+FEFF at the start of a document: [`Editor::check_document_start`];
//! - rule D, a removal between a CR and an LF blank line: [`Editor::plan_remove`].
//!
//! The tree is rowan's immutable green tree. A plan is one [`Splice`]: it replaces a contiguous run
//! of one node's children, the root's or one section's, with the parser's own nodes, so every edit
//! touches one contiguous region (C3) by construction. A commit rebuilds that node and each of its
//! ancestors up to the root, and the new tree shares every other node with the one before it.
//! Nodes are only ever read from the editor's current tree: a handle stores where its lines start
//! ([`Address`]) and resolves it again on every call.

use std::ops::Range;

use rowan::{GreenNode, Language, NodeOrToken, TextRange, TextSize};

use super::Editor;
use super::error::EditError;
use super::fragment::{self, BODY_LINE_KINDS, CRLF, LINE_BREAKS, LINE_KINDS};
use crate::ParseOptions;
use crate::ast::{AstNode, Entry, Section};
use crate::lexer::BOM;
use crate::syntax_kind::{IniLang, SyntaxKind, SyntaxNode, SyntaxToken};

/// Where the lines of a handle are: the preamble, or the section that starts at this byte offset.
///
/// A handle stores this, never a node: a commit replaces the tree, so a node from before it belongs
/// to a tree the editor no longer holds. No edit through a handle moves the start of its own
/// section, because every edit lies after the header's first byte, and the exclusive borrow rules
/// out every other edit while the handle lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Address {
    /// The lines before the first section header, after a byte order mark.
    Preamble,
    /// The `SECTION` node that starts at this offset.
    Section(TextSize),
}

/// The preamble or one section of the editor's current tree: an [`Address`], resolved.
#[derive(Debug, Clone)]
pub(super) enum Container {
    /// The lines before the first section header, after a byte order mark.
    Preamble,
    /// A `SECTION` node: its header, then its body lines.
    Section(SyntaxNode),
}

impl Container {
    /// The section's header, the line before its first body line. The preamble has none.
    fn header(&self) -> Option<SyntaxNode> {
        match self {
            Self::Preamble => None,
            Self::Section(section) => {
                assert!(section.parent().is_some());
                Some(header(section))
            }
        }
    }
}

/// The kinds of the nodes a splice puts into the root: sections, and the preamble's lines.
const ROOT_CHILD_KINDS: [SyntaxKind; 5] = [
    SyntaxKind::SECTION,
    SyntaxKind::ENTRY,
    SyntaxKind::COMMENT_LINE,
    SyntaxKind::BLANK_LINE,
    SyntaxKind::ERROR_LINE,
];

/// Children `range` of `parent` replaced with `new`: the one change of an edit.
pub(super) struct Splice {
    /// The node whose children change, in the editor's current tree: the root or a section.
    parent: SyntaxNode,
    /// Indices into `parent`'s children, tokens included. Empty for an insertion.
    range: Range<usize>,
    /// The new children, the parser's own green nodes: lines, or sections at the root.
    new: Vec<GreenNode>,
}

impl Splice {
    /// A splice of the children `range` of `parent`, a node of the editor's current tree.
    pub(super) fn new(parent: SyntaxNode, range: Range<usize>, new: Vec<GreenNode>) -> Self {
        assert!(range.start <= range.end);
        assert!(range.end <= parent.green().children().len());
        // A splice that neither removes nor adds a child would rebuild for nothing.
        if range.is_empty() {
            assert_ne!(new.len(), 0);
        }
        match parent.kind() {
            SyntaxKind::ROOT => assert!(
                new.iter()
                    .all(|node| ROOT_CHILD_KINDS.contains(&IniLang::kind_from_raw(node.kind())))
            ),
            kind => {
                assert_eq!(kind, SyntaxKind::SECTION);
                assert!(
                    new.iter()
                        .all(|node| LINE_KINDS.contains(&IniLang::kind_from_raw(node.kind())))
                );
            }
        }
        Self { parent, range, new }
    }

    /// The root of the tree `root` with this splice applied. Rebuilds `parent` and each ancestor:
    /// O(children of `parent` + children of the root).
    fn apply(self, root: &SyntaxNode) -> GreenNode {
        // A plan applies to the tree it was made from.
        assert_eq!(self.parent.tree_top(), *root);
        let new = self.new.into_iter().map(NodeOrToken::Node);
        let rebuilt = self.parent.green().splice_children(self.range, new);
        self.parent.replace_with(rebuilt)
    }
}

/// One edit: at most one splice, and the document's length after it.
pub(super) struct Plan {
    /// `None` only for an edit that changes nothing, the removal of an empty range.
    pub(super) splice: Option<Splice>,
    pub(super) len_after: usize,
}

/// A position between two lines of a container.
pub(super) struct Slot {
    /// The node that new lines go into.
    parent: SyntaxNode,
    /// The child of `parent` that new lines go before; `None` puts them at its end.
    before: Option<SyntaxNode>,
    /// The line directly before the position in document order, a header included.
    previous: Option<SyntaxNode>,
    /// The body line directly after the position, if any. At the end of the body, the next line
    /// is the next section's header, which starts with `[`, a space or a tab: never with the LF
    /// or U+FEFF that rules T2, D and B look for, so it is never needed.
    next: Option<SyntaxNode>,
}

impl Slot {
    /// A slot whose anchor and previous line, when present, are children of `parent`: a
    /// container's lines, a section's header, or, as the anchor of the preamble's end, the first
    /// section.
    fn new(
        parent: SyntaxNode,
        before: Option<SyntaxNode>,
        previous: Option<SyntaxNode>,
        next: Option<SyntaxNode>,
    ) -> Self {
        if let Some(before) = &before {
            assert!(before.parent().is_some_and(|node| node == parent));
        }
        if let Some(previous) = &previous {
            assert!(previous.parent().is_some_and(|node| node == parent));
            assert!(LINE_KINDS.contains(&previous.kind()));
        }
        match (&before, &next) {
            // Before a body line, new lines go directly before that very line.
            (Some(before), Some(next)) => assert_eq!(before, next),
            // At the end of the preamble, they go before the first section.
            (Some(before), None) => assert_eq!(before.kind(), SyntaxKind::SECTION),
            // At the end of a section, they go at its end, and no body line follows.
            (None, next) => assert!(next.is_none()),
        }
        Self {
            parent,
            before,
            previous,
            next,
        }
    }
}

/// What the lines of an insertion must parse to.
pub(super) enum NewLines<'a> {
    /// One entry with exactly this key and value.
    Entry { key: &'a str, value: &'a str },
    /// Exactly one body line for each element of the slice.
    Body(&'a [&'a str]),
}

impl NewLines<'_> {
    /// Check the nodes that the `index`-th new line parsed to.
    fn check(&self, index: usize, nodes: &[SyntaxNode]) -> Result<(), EditError> {
        match self {
            NewLines::Entry { key, value } => {
                assert_eq!(index, 0);
                check_new_entry(key, value, nodes)
            }
            NewLines::Body(lines) => {
                assert!(index < lines.len());
                check_body_line(index, lines[index], nodes)
            }
        }
    }
}

/// A new entry is accepted iff its line parses to an entry with exactly the requested key and
/// value. A key ends where the lexer ends a key and its first character makes the line an entry,
/// so key equality decides the key; the value is read in its exact context.
fn check_new_entry(key: &str, value: &str, nodes: &[SyntaxNode]) -> Result<(), EditError> {
    // The rendering ends with its terminator, so it holds at least one line.
    assert_ne!(nodes.len(), 0);
    let entry = nodes
        .first()
        .and_then(|node| Entry::cast(node.clone()))
        .filter(|entry| entry.key().as_deref() == Some(key));
    let Some(entry) = entry else {
        return Err(EditError::InvalidKey {
            key: key.to_owned(),
        });
    };
    if entry.value().as_deref() != Some(value) {
        return Err(EditError::InvalidValue {
            value: value.to_owned(),
        });
    }
    // A value that reads back whole leaves nothing else on its line, and no text after it can
    // reach the line break: no other line, no inline comment, and the line's own terminator.
    assert_eq!(nodes.len(), 1);
    assert!(entry.inline_comment().is_none());
    assert!(ends_with_newline(entry.syntax()));
    Ok(())
}

/// A raw element is accepted iff it parses to exactly one line node that is not a header and ends
/// with the terminator the editor wrote after it.
fn check_body_line(index: usize, line: &str, nodes: &[SyntaxNode]) -> Result<(), EditError> {
    // The rendering ends with its terminator, so it holds at least one line.
    assert_ne!(nodes.len(), 0);
    let invalid = || EditError::InvalidLine {
        index,
        line: line.to_owned(),
    };
    // A line break that is not a continuation starts another line.
    let [node] = nodes else {
        return Err(invalid());
    };
    // A header would move every later line of the section into a new one.
    if node.kind() == SyntaxKind::SECTION_HEADER {
        return Err(invalid());
    }
    // A line ending in a continuation takes the editor's terminator into its value.
    if !ends_with_newline(node) {
        return Err(invalid());
    }
    // The accepted line is a body line made of the element and one terminator.
    assert!(BODY_LINE_KINDS.contains(&node.kind()));
    let text = node.to_string();
    let terminator = text
        .strip_prefix(line)
        .expect("the line holds the element first");
    assert!(LINE_BREAKS.contains(&terminator));
    Ok(())
}

/// Lines to write: each line's content as pieces, and their texts with terminators.
struct Rendering {
    /// Each new line, with its terminator.
    texts: Vec<String>,
    /// The terminator that the document's unterminated last line gets first (rule C), when the
    /// new lines follow it.
    pending: Option<&'static str>,
    /// The document's length after the insertion.
    len_after: usize,
}

impl Editor {
    /// The document's tree: a cursor at the root of the editor's green tree. O(1).
    pub(super) fn root(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The document's length in bytes.
    pub(super) fn len(&self) -> usize {
        let len = usize::from(self.green.text_len());
        // Every edit checks the limit before it changes anything (SQL-1).
        assert!(len <= self.len_max);
        len
    }

    /// The sections, in document order.
    pub(super) fn sections(&self) -> impl Iterator<Item = SyntaxNode> {
        self.root()
            .children()
            .filter(|node| node.kind() == SyntaxKind::SECTION)
    }

    /// The container that `address` names in the current tree. O(log(children of the root)).
    pub(super) fn container(&self, address: Address) -> Container {
        match address {
            Address::Preamble => Container::Preamble,
            Address::Section(start) => {
                let section = child_covering(&self.root(), start);
                // A section's address is where its header starts.
                assert_eq!(section.kind(), SyntaxKind::SECTION);
                assert_eq!(section.text_range().start(), start);
                Container::Section(section)
            }
        }
    }

    /// The line that starts at byte `start`: a line of the preamble, or of the section that holds
    /// that byte. O(log(children of the root) + log(lines of the section)).
    pub(super) fn line_starting_at(&self, start: TextSize) -> SyntaxNode {
        let child = child_covering(&self.root(), start);
        let line = if child.kind() == SyntaxKind::SECTION {
            child_covering(&child, start)
        } else {
            child
        };
        assert!(LINE_KINDS.contains(&line.kind()));
        assert_eq!(line.text_range().start(), start);
        line
    }

    /// The first section whose header names exactly `name`.
    pub(super) fn find_section(&self, name: &str) -> Option<SyntaxNode> {
        self.sections()
            .find(|section| header_name_is(section, name))
    }

    /// The body lines of `container`, in document order.
    pub(super) fn body_lines(&self, container: &Container) -> impl Iterator<Item = SyntaxNode> {
        std::iter::successors(self.first_body_line(container), next_body_line)
    }

    /// The `ENTRY` body lines of `container`, in document order.
    pub(super) fn entries(&self, container: &Container) -> impl Iterator<Item = SyntaxNode> {
        self.body_lines(container)
            .filter(|line| line.kind() == SyntaxKind::ENTRY)
    }

    /// The first body line of `container`.
    fn first_body_line(&self, container: &Container) -> Option<SyntaxNode> {
        let first = match container {
            // A byte order mark is a token and lines are nodes, so the walk skips it.
            Container::Preamble => self
                .root()
                .first_child()
                .filter(|node| node.kind() != SyntaxKind::SECTION),
            Container::Section(section) => {
                // A handle's section stays in the tree while the handle lives.
                assert!(section.parent().is_some_and(|root| root == self.root()));
                header(section).next_sibling()
            }
        };
        if let Some(line) = &first {
            // A section has one header, its first child.
            assert!(BODY_LINE_KINDS.contains(&line.kind()));
        }
        first
    }

    /// The last body line of `container`.
    fn last_body_line(&self, container: &Container) -> Option<SyntaxNode> {
        let last = match container {
            Container::Preamble => match self.sections().next() {
                Some(section) => section.prev_sibling(),
                None => self.root().last_child(),
            },
            Container::Section(section) => {
                assert!(section.parent().is_some_and(|root| root == self.root()));
                section
                    .last_child()
                    .filter(|node| node.kind() != SyntaxKind::SECTION_HEADER)
            }
        };
        if let Some(line) = &last {
            // No section precedes a preamble line, and no header follows a body line.
            assert!(BODY_LINE_KINDS.contains(&line.kind()));
        }
        last
    }

    /// The child of the container's parent that lines appended to the container go before.
    fn end_anchor(&self, container: &Container) -> Option<SyntaxNode> {
        match container {
            // Preamble lines go after a byte order mark and before the first header.
            Container::Preamble => self.sections().next(),
            Container::Section(section) => {
                assert!(section.parent().is_some_and(|root| root == self.root()));
                None
            }
        }
    }

    /// The node that holds the container's lines.
    fn parent_of(&self, container: &Container) -> SyntaxNode {
        match container {
            Container::Preamble => {
                let root = self.root();
                assert_eq!(root.kind(), SyntaxKind::ROOT);
                root
            }
            Container::Section(section) => {
                assert_eq!(section.kind(), SyntaxKind::SECTION);
                section.clone()
            }
        }
    }

    /// The slot before body line `at` of `container`, or after its last body line when `at` is
    /// the number of body lines.
    pub(super) fn slot_at(&self, container: &Container, at: usize) -> Result<Slot, EditError> {
        let mut previous = container.header();
        let mut current = self.first_body_line(container);
        // The walk ends with the body, even for `at == usize::MAX`.
        for walked in 0..at {
            let Some(line) = current else {
                return Err(EditError::PositionOutOfRange {
                    at,
                    body_line_count: walked,
                });
            };
            current = next_body_line(&line);
            previous = Some(line);
        }
        // The walk forward and a step back from where it stopped agree on the line before.
        match &current {
            Some(line) => assert_eq!(line.prev_sibling(), previous),
            None => assert_eq!(
                self.last_body_line(container)
                    .or_else(|| container.header()),
                previous
            ),
        }
        Ok(Slot::new(
            self.parent_of(container),
            current.clone().or_else(|| self.end_anchor(container)),
            previous,
            current,
        ))
    }

    /// The slot directly after the container's last body line that is not blank, so that
    /// trailing blank lines, which usually separate sections, stay below new lines.
    pub(super) fn slot_at_content_end(&self, container: &Container) -> Slot {
        let mut line = self.last_body_line(container);
        let mut first_trailing_blank = None;
        while let Some(blank) = line.take_if(|node| node.kind() == SyntaxKind::BLANK_LINE) {
            line = previous_body_line(&blank);
            first_trailing_blank = Some(blank);
        }
        let previous = line.or_else(|| container.header());
        // The slot follows content, and every body line after it, walked forward, is blank.
        assert_ne!(
            previous.as_ref().map(SyntaxNode::kind),
            Some(SyntaxKind::BLANK_LINE)
        );
        assert!(
            std::iter::successors(first_trailing_blank.clone(), next_body_line)
                .all(|blank| blank.kind() == SyntaxKind::BLANK_LINE)
        );
        Slot::new(
            self.parent_of(container),
            first_trailing_blank
                .clone()
                .or_else(|| self.end_anchor(container)),
            previous,
            first_trailing_blank,
        )
    }

    /// The document's line ending: its first line break as the lexer reads it, or LF if it has
    /// none. It is read from the current text on every call, because edits change the first line.
    pub(super) fn line_ending(&self) -> &'static str {
        // The tree holds the lexer's tokens, and no token ending in CR is followed by one starting
        // with LF (C1), so the first line break lies within the first token holding CR or LF.
        let first_line_break = self
            .root()
            .descendants_with_tokens()
            .filter_map(NodeOrToken::into_token)
            .find_map(|token| {
                let start = token.text().find(['\r', '\n'])?;
                Some(fragment::line_break_at(&token.text()[start..]))
            });
        let ending = first_line_break.unwrap_or("\n");
        assert!(LINE_BREAKS.contains(&ending));
        ending
    }

    /// The document's length after removing `removed` bytes and adding `added`, or
    /// `DocumentTooLarge` past the editor's limit. The sum saturates, so it cannot wrap on any
    /// target.
    pub(super) fn check_len(&self, removed: usize, added: usize) -> Result<usize, EditError> {
        let len = self.len();
        assert!(len <= self.len_max);
        assert!(removed <= len);
        let len_after = (len - removed).saturating_add(added);
        if len_after <= self.len_max {
            Ok(len_after)
        } else {
            Err(EditError::DocumentTooLarge {
                len: len_after,
                max: self.len_max,
            })
        }
    }

    /// The line nodes that `text` parses to in the middle of the document.
    pub(super) fn fragment_lines(&self, text: &str) -> Vec<SyntaxNode> {
        // Sizes are checked before anything is rendered, so every probe is within the limit.
        assert!(text.len() <= self.len_max);
        fragment::fragment_lines(text, &self.parse_options)
    }

    /// Rule C: `line`, the document's unterminated last line, with `terminator` added, or
    /// `LastLineContinues` if the terminator would become part of its value instead. Only a line
    /// ending in a continuation backslash takes a following line break into its value, and the
    /// editor never writes such a line, so this succeeds at most once per editor.
    pub(super) fn terminate(
        &self,
        line: &SyntaxNode,
        terminator: &'static str,
    ) -> Result<SyntaxNode, EditError> {
        assert!(!ends_with_newline(line));
        // Only the document's last line lacks a terminator.
        assert_eq!(line.text_range().end(), self.green.text_len());
        let mut nodes = self.fragment_lines(&format!("{line}{terminator}"));
        // A line and its own terminator are one line of the same kind: a header stays a header.
        assert_eq!(nodes.len(), 1);
        let terminated = nodes.pop().expect("one line");
        assert_eq!(terminated.kind(), line.kind());
        let last = terminated.last_child_or_token().expect("a line has text");
        if last.kind() != SyntaxKind::NEWLINE {
            // The value continued and took the terminator.
            assert_eq!(terminated.kind(), SyntaxKind::ENTRY);
            return Err(EditError::LastLineContinues);
        }
        // The terminator is the line's own line break, and the rest of the line reads as before.
        let mut expected = token_texts(line);
        expected.push((SyntaxKind::NEWLINE, terminator.to_owned()));
        assert_eq!(token_texts(&terminated), expected);
        Ok(terminated)
    }

    /// Rule B: refuse an edit that makes U+FEFF the first character of a document without a byte
    /// order mark, where a reader would take it as one. `previous` is the line before the line that
    /// becomes first in its container, and `first` is that line's first character.
    pub(super) fn check_document_start(
        &self,
        previous: Option<&SyntaxNode>,
        first: Option<char>,
    ) -> Result<(), EditError> {
        // One condition per decision: a line after another line, or after a byte order mark,
        // does not start the document.
        if let Some(previous) = previous {
            assert!(LINE_KINDS.contains(&previous.kind()));
            return Ok(());
        }
        if self.has_byte_order_mark() {
            return Ok(());
        }
        if first == BOM.chars().next() {
            Err(EditError::ByteOrderMarkAtStart)
        } else {
            Ok(())
        }
    }

    /// Whether the document starts with a byte order mark.
    fn has_byte_order_mark(&self) -> bool {
        match self.root().first_child_or_token() {
            // Every line is a node, so only a byte order mark is a token at the root.
            Some(NodeOrToken::Token(token)) => {
                assert_eq!(token.text(), BOM);
                true
            }
            // A preamble line or a section; a header is only ever inside its section.
            Some(NodeOrToken::Node(node)) => {
                assert_ne!(node.kind(), SyntaxKind::SECTION_HEADER);
                false
            }
            None => false,
        }
    }

    /// Render new lines between `previous` and `next` with rule T's terminators, after checking
    /// the size the document will have. Sizes come from lengths before anything is rendered,
    /// because a slice can repeat one long string many times.
    fn render_new_lines(
        &self,
        previous: Option<&SyntaxNode>,
        next: Option<&SyntaxNode>,
        contents: &[Vec<&str>],
    ) -> Result<Rendering, EditError> {
        assert_ne!(contents.len(), 0);
        if let Some(next) = next {
            assert!(BODY_LINE_KINDS.contains(&next.kind()));
        }
        let ending = self.line_ending();
        let pending = pending_terminator(previous, ending);
        if pending.is_some() {
            // Only the document's last line lacks a terminator, so nothing follows it.
            assert!(next.is_none());
        }
        let before = match pending {
            Some(terminator) => terminator.chars().next_back(),
            None => previous.map(last_char),
        };
        let content_ends: Vec<Option<char>> =
            contents.iter().map(|pieces| content_end(pieces)).collect();
        let terminators = terminators(&content_ends, before, next.map(first_char), ending);
        let len_pending = pending.map_or(0, str::len);
        let added = rendered_len(contents, &terminators).saturating_add(len_pending);
        let len_after = self.check_len(0, added)?;
        let texts: Vec<String> = contents
            .iter()
            .zip(&terminators)
            .map(|(pieces, terminator)| [pieces.concat().as_str(), terminator].concat())
            .collect();
        assert_eq!(
            texts.iter().map(String::len).sum::<usize>() + len_pending,
            added
        );
        // One text per line, each ending with its terminator.
        assert_eq!(texts.len(), contents.len());
        assert!(texts.iter().all(|text| text.ends_with(['\r', '\n'])));
        Ok(Rendering {
            texts,
            pending,
            len_after,
        })
    }

    /// Plan inserting new lines at `slot`. `contents` holds each line's content as pieces, without
    /// a terminator, and `expected` says what each line must parse to.
    pub(super) fn plan_insert(
        &self,
        slot: Slot,
        contents: &[Vec<&str>],
        expected: &NewLines<'_>,
    ) -> Result<Plan, EditError> {
        let rendering =
            self.render_new_lines(slot.previous.as_ref(), slot.next.as_ref(), contents)?;
        let mut nodes = Vec::with_capacity(rendering.texts.len());
        for (index, text) in rendering.texts.iter().enumerate() {
            let lines = self.fragment_lines(text);
            expected.check(index, &lines)?;
            nodes.extend(lines);
        }
        // Each check accepts exactly one line node per rendered line, and never a header.
        assert_eq!(nodes.len(), contents.len());
        assert!(
            nodes
                .iter()
                .all(|node| BODY_LINE_KINDS.contains(&node.kind()))
        );
        // The new lines go before `before`, or at the end of `parent`.
        let at = slot
            .before
            .as_ref()
            .map_or_else(|| slot.parent.green().children().len(), SyntaxNode::index);
        let mut range = at..at;
        let mut new = Vec::with_capacity(nodes.len() + 1);
        if let Some(terminator) = rendering.pending {
            let line = slot
                .previous
                .clone()
                .expect("only a line lacks a terminator");
            let terminated = self.terminate(&line, terminator)?;
            // The unterminated line is the document's last, directly before the new lines, so the
            // splice takes it in and puts it back terminated.
            assert_eq!(line.index() + 1, at);
            range.start = line.index();
            new.push(terminated.green().to_owned());
        }
        let first = rendering.texts[0].chars().next();
        self.check_document_start(slot.previous.as_ref(), first)?;
        new.extend(nodes.iter().map(|node| node.green().to_owned()));
        // At most a terminated last line (rule C), then the new lines.
        assert_eq!(new.len(), contents.len() + range.len());
        Ok(Plan {
            splice: Some(Splice::new(slot.parent, range, new)),
            len_after: rendering.len_after,
        })
    }

    /// Plan removing body lines `range` of `container`.
    pub(super) fn plan_remove(
        &self,
        container: &Container,
        range: Range<usize>,
    ) -> Result<Plan, EditError> {
        let body_line_count = self.body_lines(container).count();
        let out_of_range = EditError::RangeOutOfRange {
            start: range.start,
            end: range.end,
            body_line_count,
        };
        if range.start > range.end {
            return Err(out_of_range);
        }
        if range.end > body_line_count {
            return Err(out_of_range);
        }
        // No early return for an empty range: it removes nothing, rule B cannot fire, and two
        // adjacent lines never read one line break together, so its plan is empty.
        let empty = range.is_empty();
        let (previous, removed, next) = self.lines_around(container, range);
        self.check_document_start(previous.as_ref(), next.as_ref().map(first_char))?;
        let len_removed = removed
            .iter()
            .map(|line| usize::from(line.text_range().len()))
            .sum();
        let len_after = self
            .check_len(len_removed, 0)
            .expect("removals only shrink the document");
        // The removed lines are consecutive children of the container's node.
        let start = removed.first().map(SyntaxNode::index);
        for (offset, line) in removed.iter().enumerate() {
            assert_eq!(Some(line.index()), start.map(|start| start + offset));
        }
        let mut splice = start.map(|start| (start..start + removed.len(), Vec::new()));
        if let (Some(previous), Some(next)) = (&previous, &next) {
            if fragment::joins_line_breaks(Some(last_char(previous)), Some(first_char(next))) {
                // Two lines that were adjacent already read their line breaks apart.
                assert!(!empty);
                // The splice also takes in the line before and the blank line after, and puts
                // back only the line before, with the merged line break.
                assert_eq!(Some(previous.index() + 1), start);
                assert_eq!(next.index(), previous.index() + 1 + removed.len());
                let merged = merge_line_breaks(previous, next, &self.parse_options);
                splice = Some((previous.index()..next.index() + 1, vec![merged]));
            }
        }
        if empty {
            assert!(splice.is_none());
        }
        let parent = self.parent_of(container);
        Ok(Plan {
            splice: splice.map(|(range, new)| Splice::new(parent, range, new)),
            len_after,
        })
    }

    /// The line before body line `range.start` (a header counts), the lines in `range`, and the
    /// body line after them, if any (see [`Slot::next`]). `range` lies within the body.
    fn lines_around(
        &self,
        container: &Container,
        range: Range<usize>,
    ) -> (Option<SyntaxNode>, Vec<SyntaxNode>, Option<SyntaxNode>) {
        let mut previous = container.header();
        let mut lines = self.body_lines(container);
        for _ in 0..range.start {
            previous = Some(lines.next().expect("the range lies within the body"));
        }
        if let Some(previous) = &previous {
            assert!(LINE_KINDS.contains(&previous.kind()));
        }
        let removed: Vec<SyntaxNode> = lines.by_ref().take(range.len()).collect();
        assert_eq!(removed.len(), range.len());
        let next = lines.next();
        (previous, removed, next)
    }

    /// Plan appending a section named `name` at the end of the document, and return the new
    /// section's green node with the plan.
    pub(super) fn plan_section(&self, name: &str) -> Result<(Plan, GreenNode), EditError> {
        let container = self
            .sections()
            .last()
            .map_or(Container::Preamble, Container::Section);
        let last = self
            .last_body_line(&container)
            .or_else(|| container.header());
        if let Some(last) = &last {
            // The document's last line: a header or a body line, never a following section.
            assert!(LINE_KINDS.contains(&last.kind()));
            assert!(last.next_sibling().is_none());
        }
        // A blank line separates the section from the line before it, unless that line is blank.
        // It belongs to the previous container, where a reader puts it.
        let separated = last
            .as_ref()
            .is_some_and(|line| line.kind() != SyntaxKind::BLANK_LINE);
        let mut contents = Vec::with_capacity(2);
        if separated {
            contents.push(Vec::new());
        }
        contents.push(vec!["[", name, "]"]);
        let rendering = self.render_new_lines(last.as_ref(), None, &contents)?;
        let (blank, section) =
            self.parse_new_section(&rendering.texts.concat(), name, separated)?;
        assert_eq!(blank.is_some(), separated);
        // The container is the last section, or the preamble of a document without one, so its
        // end is the end of its node. The lines that end it: its last line, terminated (rule C),
        // and the blank line that separates the new section.
        let parent = self.parent_of(&container);
        let end = parent.green().children().len();
        let mut range = end..end;
        let mut tail = Vec::with_capacity(2);
        if let Some(terminator) = rendering.pending {
            let line = last.expect("only a line lacks a terminator");
            let terminated = self.terminate(&line, terminator)?;
            // The unterminated line is the document's last: its container's last child.
            assert_eq!(line.index() + 1, end);
            range.start = line.index();
            tail.push(terminated.green().to_owned());
        }
        tail.extend(blank.map(|blank| blank.green().to_owned()));
        assert_eq!(
            tail.len(),
            usize::from(separated) + usize::from(rendering.pending.is_some())
        );
        let section = section.green().to_owned();
        let plan = Plan {
            splice: Some(self.append_section(&container, (range, tail), section.clone())),
            len_after: rendering.len_after,
        };
        Ok((plan, section))
    }

    /// The splice that replaces the children `range` of `container`'s node with `tail`, and puts
    /// `section` after `container`, the document's last.
    fn append_section(
        &self,
        container: &Container,
        (range, mut tail): (Range<usize>, Vec<GreenNode>),
        section: GreenNode,
    ) -> Splice {
        match container {
            // The preamble's node is the root: one splice at its end.
            Container::Preamble => {
                tail.push(section);
                Splice::new(self.parent_of(container), range, tail)
            }
            // The new section follows the last one. A last section that ends with a terminated
            // blank line keeps its node; any other is rebuilt with its new end.
            Container::Section(last) => {
                assert!(last.next_sibling().is_none());
                let index = last.index();
                if tail.is_empty() {
                    assert!(range.is_empty());
                    return Splice::new(self.root(), index + 1..index + 1, vec![section]);
                }
                let tail = tail.into_iter().map(NodeOrToken::Node);
                let rebuilt = last.green().splice_children(range, tail);
                Splice::new(self.root(), index..index + 1, vec![rebuilt, section])
            }
        }
    }

    /// The blank separator line, if any, and the `SECTION` that `text` parses to, or
    /// `InvalidSectionName` if the header does not read back `name`.
    fn parse_new_section(
        &self,
        text: &str,
        name: &str,
        separated: bool,
    ) -> Result<(Option<SyntaxNode>, SyntaxNode), EditError> {
        let mut children = fragment::parse_fragment(text, &self.parse_options);
        // `[` starts a header, so the text ends with a section that runs to its end.
        let section = children.pop().expect("the text ends with a header");
        assert_eq!(section.kind(), SyntaxKind::SECTION);
        let read_back = Section::cast(section.clone()).and_then(|section| section.name());
        if read_back.as_deref() != Some(name) {
            return Err(EditError::InvalidSectionName {
                name: name.to_owned(),
            });
        }
        // A name that reads back is the whole header line, so the section has no body line.
        assert_eq!(section.children().count(), 1);
        assert_eq!(children.len(), usize::from(separated));
        let blank = children.pop().inspect(|blank| {
            assert_eq!(blank.kind(), SyntaxKind::BLANK_LINE);
        });
        Ok((blank, section))
    }

    /// Apply `plan`. Only the planning above can fail, so a commit always completes.
    pub(super) fn commit(&mut self, plan: Plan) {
        // Plans are built from checked sizes only (SQL-1).
        assert!(plan.len_after <= self.len_max);
        if let Some(splice) = plan.splice {
            self.green = splice.apply(&self.root());
        }
        // A splice that took the wrong children changes the length: catch it at once, in O(1).
        assert_eq!(self.len(), plan.len_after);
        // C1, in O(document): every debug test, coverage run, mutation run and fuzz build checks
        // it after every edit. Miri runs too slowly for it.
        #[cfg(all(debug_assertions, not(miri)))]
        assert!(self.agrees_with_reparse());
    }

    /// Whether the tree equals a parse of the document's text (C1).
    ///
    /// Compiled exactly where something calls it: in unit tests, and wherever `commit` checks C1.
    /// Any other build, a release build or a Miri build of the library for an integration test,
    /// would warn that it is unused, and CI denies warnings.
    #[cfg(any(test, all(debug_assertions, not(miri))))]
    pub(super) fn agrees_with_reparse(&self) -> bool {
        let reparse = crate::parse_with(&self.finish(), &self.parse_options);
        *reparse.green() == self.green
    }
}

/// Whether `node` holds `green` itself, not an equal copy. The nodes of a fragment parse belong to
/// no other tree, so after a commit this identity finds the very entry or section it inserted.
pub(super) fn is_node(node: &SyntaxNode, green: &GreenNode) -> bool {
    std::ptr::from_ref(node.green()) == std::ptr::from_ref::<rowan::GreenNodeData>(green)
}

/// The header of `section`, its first child.
pub(super) fn header(section: &SyntaxNode) -> SyntaxNode {
    let header = section
        .first_child()
        .expect("a section starts with its header");
    assert_eq!(header.kind(), SyntaxKind::SECTION_HEADER);
    header
}

/// Whether the header of `section` names exactly `name`, read without copying the name.
pub(super) fn header_name_is(section: &SyntaxNode, name: &str) -> bool {
    assert_eq!(section.kind(), SyntaxKind::SECTION);
    header(section)
        .children_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .find(|token| token.kind() == SyntaxKind::IDENT)
        .is_some_and(|ident| ident.text() == name)
}

/// The key token of an `ENTRY` node.
pub(super) fn key_token(entry: &SyntaxNode) -> SyntaxToken {
    assert_eq!(entry.kind(), SyntaxKind::ENTRY);
    let key = entry
        .children()
        .find(|node| node.kind() == SyntaxKind::KEY)
        .expect("an entry has a key");
    key.first_token().expect("a key holds its identifier")
}

/// The container that holds `line`.
pub(super) fn container_of(line: &SyntaxNode) -> Container {
    let parent = line
        .parent()
        .expect("an edited line is in the editor's tree");
    if parent.kind() == SyntaxKind::SECTION {
        Container::Section(parent)
    } else {
        assert_eq!(parent.kind(), SyntaxKind::ROOT);
        Container::Preamble
    }
}

/// The line after `line` in the same container.
fn next_body_line(line: &SyntaxNode) -> Option<SyntaxNode> {
    assert!(BODY_LINE_KINDS.contains(&line.kind()));
    // In the preamble the first section ends the body. A section holds only its own lines.
    let next = line
        .next_sibling()
        .filter(|node| node.kind() != SyntaxKind::SECTION);
    if let Some(next) = &next {
        // A section's only header is its first child, before every body line.
        assert!(BODY_LINE_KINDS.contains(&next.kind()));
    }
    next
}

/// The line before `line` in the same container.
fn previous_body_line(line: &SyntaxNode) -> Option<SyntaxNode> {
    assert!(BODY_LINE_KINDS.contains(&line.kind()));
    // A section's header is not one of its body lines.
    let previous = line
        .prev_sibling()
        .filter(|node| node.kind() != SyntaxKind::SECTION_HEADER);
    if let Some(previous) = &previous {
        // Every section follows the preamble's lines.
        assert!(BODY_LINE_KINDS.contains(&previous.kind()));
    }
    previous
}

/// Whether `line` ends with its own `NEWLINE` token. Only the document's last line can lack one.
pub(super) fn ends_with_newline(line: &SyntaxNode) -> bool {
    assert!(LINE_KINDS.contains(&line.kind()));
    line.last_child_or_token()
        .is_some_and(|element| element.kind() == SyntaxKind::NEWLINE)
}

/// The last character of `line`'s text. Every line holds at least one character.
fn last_char(line: &SyntaxNode) -> char {
    assert!(LINE_KINDS.contains(&line.kind()));
    line.descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .filter_map(|token| token.text().chars().next_back())
        .last()
        .expect("a line holds text")
}

/// The first character of `line`'s text. Every line holds at least one character.
fn first_char(line: &SyntaxNode) -> char {
    assert!(LINE_KINDS.contains(&line.kind()));
    line.descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .find_map(|token| token.text().chars().next())
        .expect("a line holds text")
}

/// The kind and text of every token of `node`, in order.
fn token_texts(node: &SyntaxNode) -> Vec<(SyntaxKind, String)> {
    assert!(LINE_KINDS.contains(&node.kind()));
    node.descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .map(|token| (token.kind(), token.text().to_owned()))
        .collect()
}

/// The last character of a line's content given as pieces, or `None` for an empty line.
pub(super) fn content_end(pieces: &[&str]) -> Option<char> {
    let end = pieces
        .iter()
        .rev()
        .find_map(|piece| piece.chars().next_back());
    // Paired: the last character of the pieces is the last character of their concatenation.
    assert_eq!(end, pieces.concat().chars().next_back());
    end
}

/// The length of rendered lines: every piece and every terminator, saturating, so that it cannot
/// wrap on any target.
pub(super) fn rendered_len(contents: &[Vec<&str>], terminators: &[&str]) -> usize {
    contents
        .iter()
        .flatten()
        .chain(terminators)
        .fold(0, |len, piece| len.saturating_add(piece.len()))
}

/// The terminator rule C gives `line` before a line is added after it, or `None` if `line` is
/// absent or already terminated.
fn pending_terminator(line: Option<&SyntaxNode>, ending: &'static str) -> Option<&'static str> {
    assert!(LINE_BREAKS.contains(&ending));
    let line = line.filter(|line| !ends_with_newline(line))?;
    Some(terminator_after(Some(last_char(line)), ending))
}

/// The terminator of a line whose text ends with `tail` (rule T1): `ending`, or CRLF if a CR at
/// the end of the line would read together with the LF that `ending` starts with.
pub(super) fn terminator_after(tail: Option<char>, ending: &'static str) -> &'static str {
    assert!(LINE_BREAKS.contains(&ending));
    let terminator = if fragment::joins_line_breaks(tail, ending.chars().next()) {
        CRLF
    } else {
        ending
    };
    // T1, paired: the terminator never reads together with the text before it.
    assert!(!fragment::joins_line_breaks(
        tail,
        terminator.chars().next()
    ));
    assert!(LINE_BREAKS.contains(&terminator));
    terminator
}

/// The terminator of each new line (rule T), for lines whose contents end with `content_ends`
/// (`None` for an empty line), written after text ending with `before` and before text starting
/// with `after`: the document's `ending`, except that a line break that would read together with
/// a CR before it (T1) or with an LF after the last new line (T2) is CRLF. CRLF neither starts
/// with LF nor ends with CR, so neither adjustment undoes the other.
pub(super) fn terminators(
    content_ends: &[Option<char>],
    before: Option<char>,
    after: Option<char>,
    ending: &'static str,
) -> Vec<&'static str> {
    assert!(LINE_BREAKS.contains(&ending));
    let mut previous = before;
    let mut terminators = Vec::with_capacity(content_ends.len());
    for content_end in content_ends {
        // An empty line's line break directly follows the text before it.
        let terminator = terminator_after(content_end.or(previous), ending);
        terminators.push(terminator);
        previous = terminator.chars().next_back();
    }
    let last = terminators
        .last_mut()
        .expect("an insertion writes at least one line");
    if fragment::joins_line_breaks(last.chars().next_back(), after) {
        *last = CRLF;
    }
    // T2, paired: the last terminator never reads together with the text after it.
    assert!(!fragment::joins_line_breaks(
        last.chars().next_back(),
        after
    ));
    assert_eq!(terminators.len(), content_ends.len());
    terminators
}

/// Rule D: a removal left a CR line break directly before an LF blank line, and a reader reads the
/// two as one CRLF. Make the tree say so: the CR becomes that CRLF, in the line returned here, and
/// the caller drops the blank line. No byte changes.
fn merge_line_breaks(
    previous: &SyntaxNode,
    next: &SyntaxNode,
    options: &ParseOptions,
) -> GreenNode {
    // Only an exact LF blank line starts with LF, and only a CR terminator ends a line with CR:
    // `previous` is terminated, because a line follows it.
    assert_eq!(next.kind(), SyntaxKind::BLANK_LINE);
    assert_eq!(next.text(), "\n");
    let line_break = previous.last_child_or_token().expect("a line has text");
    assert_eq!(line_break.kind(), SyntaxKind::NEWLINE);
    assert_eq!(line_break.to_string(), "\r");
    let crlf_line = fragment::fragment_lines(CRLF, options)
        .pop()
        .expect("a line break is a line");
    let crlf = crlf_line
        .first_child_or_token()
        .and_then(NodeOrToken::into_token)
        .expect("a blank line holds its line break");
    assert_eq!(crlf.kind(), SyntaxKind::NEWLINE);
    let crlf = NodeOrToken::Token(crlf.green().to_owned());
    previous.green().replace_child(line_break.index(), crlf)
}

/// The child node of `parent` that holds the byte at `offset`, found by binary search. Lines and
/// sections hold at least one byte each, so exactly one child holds it.
fn child_covering(parent: &SyntaxNode, offset: TextSize) -> SyntaxNode {
    let byte = TextRange::at(offset, TextSize::from(1));
    parent
        .child_or_token_at_range(byte)
        .and_then(NodeOrToken::into_node)
        .expect("a line or a section holds the byte")
}
