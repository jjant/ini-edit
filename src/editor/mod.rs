//! A format-preserving editor whose tree always equals a parse of its text.
//!
//! ```
//! use ini_edit::ParseOptions;
//! use ini_edit::editor::{EditError, EditOptions, Editor, SeparatorSpacing};
//!
//! let parse_options = ParseOptions { allow_no_value: false, inline_comments: false };
//! let edit_options = EditOptions { separator_spacing: SeparatorSpacing::Preserve };
//! let source = "[server]\nhost = 0.0.0.0\nport = 8080\n";
//! let mut editor = Editor::new(source, &parse_options, &edit_options)?;
//!
//! let mut server = editor.section_or_insert("server")?;
//! server.set("port", "9090")?;
//! server.append_entry("timeout", "30")?;
//! assert_eq!(editor.finish(), "[server]\nhost = 0.0.0.0\nport = 9090\ntimeout = 30\n");
//!
//! // An edit that a reader would not read back as written is refused, and changes nothing.
//! let mut server = editor.section("server").expect("the section exists");
//! assert!(matches!(server.set("host", "a\n[evil]"), Err(EditError::InvalidValue { .. })));
//! assert_eq!(editor.finish(), "[server]\nhost = 0.0.0.0\nport = 9090\ntimeout = 30\n");
//! # Ok::<(), EditError>(())
//! ```
//!
//! # Guarantees
//!
//! - **What you see is what a reader gets.** After every call, successful or not, the editor's
//!   tree ([`Editor::file`]) equals a parse of its text ([`Editor::finish`]) with the editor's
//!   [`ParseOptions`]: malformed lines, byte order marks, continued values, mixed line endings,
//!   duplicates and inline comments included. So reopening a saved document never changes the
//!   meaning of the next edit.
//! - **A refused edit changes nothing.** Every operation that returns `Err` leaves the text and
//!   the tree exactly as they were.
//! - **Edits are local.** A successful edit changes exactly what its documentation says: bytes
//!   outside the inserted text, the rewritten line or the removed lines stay byte for byte.
//! - **Caller input never panics.** No key, value, name, line, position, range, option or size
//!   makes the editor panic; only a violated internal invariant, a bug, would.
//!
//! Every check is the crate's own parser: an edit renders the exact text it will write, parses it
//! with the editor's options in the middle of a document, and is accepted only if the parse has
//! the requested meaning. The editor then inserts the parser's own nodes.
//!
//! # The model
//!
//! A document is an optional byte order mark, the *preamble* (the lines before the first header),
//! and its sections (a header, then lines). A *line* is one line node of the tree: an entry
//! (whose value may continue over several physical lines with a trailing backslash), a comment
//! line, a blank line or a malformed line. It owns its line break; only the document's last line
//! may have none.
//!
//! - **Body lines.** A container, the preamble or a section, has *body lines*: its lines without
//!   the header. [`File::preamble_lines`] and
//!   [`Section::body_lines`](crate::ast::Section::body_lines) list them, so `editor.file()` shows
//!   every position.
//! - **Positions.** Position `p` is in `0..=body_line_count()` and stands directly before body
//!   line `p`; `body_line_count()` is after the last one. Position 0 of a section is right after
//!   its header. Positions and ranges past the end are errors; nothing is clamped.
//! - **Content end.** Appends ([`append_entry`](SectionEditor::append_entry),
//!   [`append_body_lines`](SectionEditor::append_body_lines), and [`set`](SectionEditor::set) of
//!   a new key) go directly after the last body line that is not blank, so trailing blank lines,
//!   which usually separate sections, stay below.
//! - **Above a header.** A reader puts every line above a header in the container before it, so
//!   lines meant to go above section `k`'s header go at the end of section `k - 1`, or of the
//!   preamble when `k` is 0.
//! - **First match.** With duplicates, the lookups by name or key ([`Editor::section`],
//!   [`Editor::section_or_insert`], [`SectionEditor::entry`], `set`, `remove_entry`,
//!   `rename_key`) use the first match in document order; [`Editor::section_at`] and
//!   [`SectionEditor::entry_at`] reach every one. Lookups never change the document.
//!
//! # What the editor writes
//!
//! - **Separators.** New entries are `key{before}={after}value`, with the gap of the editor's
//!   [`SeparatorSpacing`]: `key = value` under `Preserve`, `key=value` under `Compact`, or exactly
//!   the given [`Whitespace`] under `Exact`. [`EntryEditor::set_value`] documents how an existing
//!   entry's gap, trailing spacing and inline comment are kept.
//! - **Line breaks (rule T).** New lines use the document's line ending, its first line break, or
//!   `\n` if it has none, except that a line break that would read together with an adjacent CR
//!   before it or LF after it as one CRLF is written as CRLF. No existing byte changes.
//! - **The unterminated last line (rule C).** A line added after the document's last line, which
//!   has no line break, first gives that line one.
//! - **New sections** go at the end of the document, after one blank line unless the document is
//!   empty or ends with a blank line.
//!
//! # What the editor refuses
//!
//! An edit is refused with an [`EditError`] when a reader would not read back what it writes:
//!
//! - **Keys**: not empty; no `=`, `:`, space, tab, CR or LF; not starting with `;`, `#` or `[`.
//!   Accepted: `k`, `a.b-c`, `λ`, `k]`, `k;c`, `k#c`, `k\`, `\`, and U+FEFF anywhere in the middle
//!   of a document. Key validity does not depend on [`ParseOptions`].
//! - **Values**: a value does not start or end with a space or tab; every line break directly
//!   follows a backslash, with only spaces and tabs between them; the last line does not end in a
//!   backslash; with [`inline_comments`](ParseOptions::inline_comments), its last line has no `;`
//!   or `#` directly after a run of spaces or tabs that follows other text on that line; it may be
//!   empty. A value whose last line ends in a backslash is refused even where it would read back
//!   (as the last line, or before a kept inline comment), so that no successful edit can make a
//!   later one continue it; drop the trailing backslash.
//! - **Section names**: not empty; no `]`, CR or LF; not starting or ending with a space or tab.
//!   `""` names no section, so [`Editor::section`] never finds it; edit headless entries through
//!   [`Editor::preamble`].
//! - **Separator spacing**: only spaces and tabs ([`Whitespace::new`]).
//! - **Raw lines**: each element of a slice is exactly one body line, without its terminator,
//!   that is not a section header and does not end with a continuation backslash; only an
//!   entry's value may continue onto further lines within its element. Create sections with
//!   [`Editor::section_or_insert`].
//! - **The byte order mark (rule B)**: an edit may not make U+FEFF the first character of a
//!   document without a byte order mark, where a reader would take it as one.
//! - **A continuing last line (rule C)**: if the document's last line has no line break and ends
//!   with a continuation backslash, nothing can be added after it: the new line would join its
//!   value. Edit or remove that line first.
//! - **Size**: a document can grow to one byte less than [`MAX_INPUT_LEN`].
//!
//! When several checks fail, the error is the first of: positions and ranges, then a missing key
//! (`Ok(false)`) or a duplicate one, then the size, then the caller's text (key before value), then
//! the document's context (rule C before rule B).
//!
//! # Removals and CR/LF (rule D)
//!
//! A removal deletes exactly the removed lines' bytes. If that leaves a CR line break directly
//! before a blank line made of one LF, a reader reads the two as one CRLF line break, so the tree
//! says so too: the blank line goes, and the body has one line fewer than requested. No byte
//! changes. A removal only ever merges the blank line *after* the removed range, so removing
//! while iterating in reverse stays correct.
//!
//! # Handles
//!
//! [`SectionEditor`] and [`EntryEditor`] borrow the editor exclusively, so a handle never sees a
//! stale or removed line: acquire one, edit, and drop it before acquiring the next. Read the
//! whole document through a handle's `editor()`:
//!
//! ```
//! # use ini_edit::ParseOptions;
//! # use ini_edit::editor::{EditError, EditOptions, Editor};
//! let mut editor = Editor::new("[s]\na=1\n", &ParseOptions::default(), &EditOptions::default())?;
//! let mut section = editor.section("s").expect("the section exists");
//! section.set("a", "2")?;
//! assert_eq!(section.editor().finish(), "[s]\na=2\n");
//! section.set("b", "3")?;
//! assert_eq!(editor.finish(), "[s]\na=2\nb = 3\n");
//! # Ok::<(), EditError>(())
//! ```
//!
//! Batch edits re-acquire entries by index. Iterate in reverse to remove. A comment that
//! [`set_value`](EntryEditor::set_value) moves above its entry shifts later body positions, not
//! entry indices.
//!
//! ```
//! # use ini_edit::ParseOptions;
//! # use ini_edit::editor::{EditError, EditOptions, Editor};
//! let source = "[s]\na = 1\nb = 2\nc = 3\n";
//! let mut editor = Editor::new(source, &ParseOptions::default(), &EditOptions::default())?;
//! let mut section = editor.section_or_insert("s")?;
//! for index in 0..section.entry_count() {
//!     let mut entry = section.entry_at(index).expect("index < entry_count");
//!     let key = format!("k{}", entry.index());
//!     entry.set_key(&key)?;
//! }
//! for index in (0..section.entry_count()).rev() {
//!     let entry = section.entry_at(index).expect("index < entry_count");
//!     if entry.value().as_deref() != Some("2") {
//!         entry.remove()?;
//!     }
//! }
//! assert_eq!(editor.finish(), "[s]\nk1 = 2\n");
//! # Ok::<(), EditError>(())
//! ```
//!
//! What the borrows rule out, at compile time. Two handles at once:
//!
//! ```compile_fail,E0499
//! # use ini_edit::{ParseOptions, editor::{EditOptions, Editor}};
//! let (parse_options, edit_options) = (ParseOptions::default(), EditOptions::default());
//! let mut editor = Editor::new("[s]\n", &parse_options, &edit_options).unwrap();
//! let first = editor.section_at(0);
//! let second = editor.section_at(0);
//! drop((first, second));
//! ```
//!
//! An entry handle outliving its section handle's editor:
//!
//! ```compile_fail,E0597
//! # use ini_edit::{ParseOptions, editor::{EditOptions, Editor}};
//! let entry;
//! {
//!     let mut editor =
//!         Editor::new("[s]\nk=v\n", &ParseOptions::default(), &EditOptions::default()).unwrap();
//!     let mut section = editor.section_at(0).unwrap();
//!     entry = section.entry_at(0);
//! }
//! drop(entry);
//! ```
//!
//! Dropping the editor while a handle lives:
//!
//! ```compile_fail,E0505
//! # use ini_edit::{ParseOptions, editor::{EditOptions, Editor}};
//! let mut editor =
//!     Editor::new("[s]\nk=v\n", &ParseOptions::default(), &EditOptions::default()).unwrap();
//! let mut section = editor.section_at(0).unwrap();
//! let entry = section.entry_at(0).unwrap();
//! drop(editor);
//! let _ = entry.index();
//! ```
//!
//! Using a section handle after removing its section:
//!
//! ```compile_fail,E0382
//! # use ini_edit::{ParseOptions, editor::{EditOptions, Editor}};
//! let (parse_options, edit_options) = (ParseOptions::default(), EditOptions::default());
//! let mut editor = Editor::new("[s]\n", &parse_options, &edit_options).unwrap();
//! let section = editor.section_at(0).unwrap();
//! section.remove();
//! let _ = section.body_line_count();
//! ```
//!
//! # Costs
//!
//! The editor's tree is immutable and shares its nodes: an edit builds a new tree that reuses
//! every line and section it does not touch. Each call rebuilds, once, the section whose lines it
//! changes and then the document's root: O(s + r + k), where s counts that section's lines, r
//! counts the root's children (the preamble's lines and the sections), and k counts the lines the
//! call inserts or removes. An edit of the preamble rebuilds only the root, O(r + k). So a call
//! that inserts or removes k lines pays for the rebuild once, not k times:
//! [`append_body_lines`](SectionEditor::append_body_lines) and
//! [`insert_body_lines`](SectionEditor::insert_body_lines) with k elements,
//! [`remove_body_lines`](SectionEditor::remove_body_lines) of k lines, and
//! [`remove`](SectionEditor::remove) on a preamble of k lines.
//!
//! Each call also finds its handle's section or entry again by binary search, O(log r + log s),
//! walks to its position, O(lines), renders and parses the text it writes, O(bytes written),
//! reads the line ending, O(first line), and, once per editor, terminates an unterminated last
//! line, O(last line). Lookups are O(sections) or O(lines). [`Editor::file`] is O(1), and
//! [`Editor::finish`] is O(document).
//!
//! Filling or clearing a large section one line per call is therefore quadratic in its size, and
//! every edit of a document with many sections pays for all of them; one call with all the lines
//! is linear. To load or replace a whole document, build the text and parse it with
//! [`Editor::new`]. Debug builds also reparse the whole document after every edit, to check the
//! first guarantee.
//!
//! # Migrating from 0.4
//!
//! ```text
//! // 0.4
//! let ed = Editor::new(src);
//! ed.section("server").set("port", "9090");
//! ed.section("server").append_entry("timeout", "30");
//! assert!(ed.section("db").rename_key("url", "dsn"));
//! ed.section("s").insert_raw_lines_at(usize::MAX, &["; x"]);
//! ed.section("s").remove_lines(1..3);
//! for e in ed.section("s").entries_mut() { e.set_key(&format!("k{}", e.index())); }
//! ed.section("").set("root", "false");  // appended "[]" and a second `root` under it
//! let out = ed.finish();
//! ```
//!
//! ```
//! // 0.5
//! use ini_edit::ParseOptions;
//! use ini_edit::editor::{EditError, EditOptions, Editor, SeparatorSpacing};
//!
//! let source = "root = true\n[server]\nport = 8080\n[db]\nurl = x\n[s]\na = 1\nb = 2\nc = 3\n";
//! let parse_options = ParseOptions { allow_no_value: false, inline_comments: false };
//! let edit_options = EditOptions { separator_spacing: SeparatorSpacing::Preserve };
//! let mut editor = Editor::new(source, &parse_options, &edit_options)?;
//! let mut server = editor.section_or_insert("server")?;
//! server.set("port", "9090")?;
//! server.append_entry("timeout", "30")?;
//! if let Some(mut db) = editor.section("db") {           // a lookup never creates
//!     db.rename_key("url", "dsn")?;                      // Ok(false) if absent
//! }
//! let mut section = editor.section_or_insert("s")?;
//! let end = section.body_line_count();
//! section.insert_body_lines(end, &["; x"])?;             // no clamping
//! section.remove_body_lines(0..2)?;                      // body lines: 0.4's 1..3
//! for index in 0..section.entry_count() {
//!     let mut entry = section.entry_at(index).expect("index < entry_count");
//!     let key = format!("k{}", entry.index());
//!     entry.set_key(&key)?;
//! }
//! editor.preamble().set("root", "false")?;               // keys before the first header
//! assert_eq!(
//!     editor.finish(),
//!     "root = false\n[server]\nport = 9090\ntimeout = 30\n[db]\ndsn = x\n[s]\nk0 = 3\n; x\n"
//! );
//! # Ok::<(), EditError>(())
//! ```
//!
//! The calls, mapped:
//!
//! - `Editor::new(source)`, `with_parse_options`, `with_edit_options`, `with_options`: one
//!   constructor, `Editor::new(source, &parse_options, &edit_options)?`.
//! - `editor.section(name)`, which created a missing section: `editor.section_or_insert(name)?`
//!   creates; `editor.section(name)` looks up and returns an `Option`.
//! - `editor.section("")`: `editor.preamble()`; `""` is not a section name.
//! - `section.set`, `section.append_entry`, `entry.set_key`, `entry.set_value`, `entry.remove()`:
//!   add `?`.
//! - `section.insert_entry_at_line(line, key, value)` with `line == 0`:
//!   `section.insert_entry(0, key, value)?`. With `line >= 1`:
//!   `section.insert_entry(position, key, value)?`, where `position` is one past the body position
//!   of the `line`-th entry or comment; past the last one, `section.append_entry(key, value)?`.
//! - `section.remove_entry(key)` and `section.rename_key(old_key, new_key)` return
//!   `Result<bool, EditError>`; a rename collision is `Err(DuplicateKey)` instead of `false`.
//! - `section.entries_mut()`: a loop over `0..section.entry_count()` that takes each entry with
//!   `section.entry_at(index)`, as in the example above; in reverse to remove.
//! - `EntryEditor::index()`, a snapshot: `index()`, computed from the tree; `body_line()` is
//!   new. `EntryEditor::key()` returns `String`.
//! - `section.append_raw_lines(&["a", "b\n"])`: `section.append_body_lines(&["a", "b"])?`, one
//!   line per element, without terminators.
//! - `section.insert_raw_lines_at(index, lines)` with `index >= 1`:
//!   `section.insert_body_lines(index - 1, lines)?`. With `index == 0`, above the header: insert
//!   at the end of the previous container,
//!   `previous.insert_body_lines(previous.body_line_count(), lines)?`. With `usize::MAX`:
//!   `let end = section.body_line_count(); section.insert_body_lines(end, lines)?`.
//! - `section.remove_lines(start..end)` with `start >= 1`:
//!   `section.remove_body_lines(start - 1..end - 1)?`. With `start == 0`, which removed the
//!   header: `section.remove()` for the section, or `section.remove_body_lines(0..end - 1)?` for
//!   the body. With `usize::MAX` as the end:
//!   `let count = section.body_line_count(); section.remove_body_lines(start - 1..count)?`. A
//!   reversed range, a no-op in 0.4, is `Err(RangeOutOfRange)`.
//! - `SeparatorSpacing::exact("\t", " ")`: add `?`; the `Exact` fields are [`Whitespace`].
//! - Retained or detached handles: re-acquire a handle after other edits, and read through
//!   `handle.editor()`.
//! - Values ending in `\`: refused; drop the trailing backslash.
//! - Raw text with a header: `editor.section_or_insert(name)?`, which appends the section at the
//!   end of the document.
//! - rowan's types are rowan 0.17's: the tree types [`SyntaxNode`](crate::SyntaxNode),
//!   [`SyntaxToken`](crate::SyntaxToken) and [`SyntaxElement`](crate::SyntaxElement), the
//!   `rowan::GreenNode` of [`Parse::green`](crate::Parse::green), the `rowan::Language` that
//!   [`IniLang`](crate::IniLang) implements and the `rowan::SyntaxKind` that
//!   [`SyntaxKind`](crate::SyntaxKind) converts into. Code that names rowan's types depends on
//!   `rowan = "0.17"`; with rowan 0.16, `rowan::SyntaxNode<IniLang>` fails to compile because
//!   `IniLang` does not implement 0.16's `Language`. `node.green()` returns `&GreenNodeData`
//!   instead of a `Cow`, and the `From<Cow<GreenNodeData>>` conversions into `GreenNode` and
//!   `GreenElement` are gone: `.green().to_owned()` instead of `.green().into_owned()` or
//!   `GreenNode::from(node.green())`. rowan 0.17 has no mutable trees (`clone_for_update`,
//!   `detach`, `SyntaxNode::splice_children`): edit through the editor, or build a new tree with
//!   `GreenNodeData::replace_child`, `insert_child`, `remove_child` or `splice_children` and
//!   `SyntaxNode::replace_with`.
//!
//! Behavior changes: every mutator and the constructor return `Result`, and refused edits change
//! nothing; lookups have no side effects; positions count body lines; raw lines are parsed into
//! real lines; keys, values, names and spacing that a reader would not read back are refused, and
//! so are values ending in a backslash; U+FEFF can no longer become the first character of a
//! document without a byte order mark; a new line break next to a CR, or before an LF blank line,
//! is CRLF instead of rewriting the neighbor, and a removal between a CR and an LF blank line
//! merges them in the tree; documents and edits past the size limit are refused; handles are
//! exclusive borrows; an [`Editor`] is `Send` and `Sync`.

mod entry;
mod error;
mod fragment;
// The parser checks its trees with it in debug builds; the editor's own
// checks compare every edit with a reparse instead.
#[cfg(all(debug_assertions, not(miri)))]
mod line_nodes;
mod options;
mod plan;
mod section;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;

pub use entry::EntryEditor;
pub use error::EditError;
pub use options::{EditOptions, SeparatorSpacing, Whitespace};
pub use section::SectionEditor;

#[cfg(all(debug_assertions, not(miri)))]
pub(crate) use line_nodes::debug_assert_line_nodes;

use crate::ast::{AstNode, File};
use crate::{MAX_INPUT_LEN, ParseOptions, parse_with};
use plan::Address;

/// The length of the longest `String`, which bounds documents on 32-bit targets.
const ISIZE_MAX: usize = isize::MAX.unsigned_abs();

/// The longest document the editor holds, in bytes.
///
/// The editor parses every line it writes after a one-byte line break (`parse_fragment`), so it
/// stops one byte short of the parser's limit, [`MAX_INPUT_LEN`]: every probe then fits the
/// parser too. It is also below `isize::MAX`, the length of the longest `String`, which is the
/// smaller bound on 32-bit targets, so `finish()` and every probe can be built.
///
/// Both bounds are runs of one bits, `2^k - 1`, so the smaller one is their bitwise AND: one
/// expression for every target, without a branch that a 64-bit test run could not reach.
const LEN_MAX: usize = (MAX_INPUT_LEN & ISIZE_MAX) - 1;

// Both bounds are runs of one bits from bit 0, so their AND is the smaller one.
const _: () = assert!(MAX_INPUT_LEN.leading_zeros() + MAX_INPUT_LEN.count_ones() == usize::BITS);
const _: () = assert!(ISIZE_MAX.leading_zeros() + ISIZE_MAX.count_ones() == usize::BITS);
// The probe of a document at the limit, one byte longer, fits the parser and a `String`.
const _: () = assert!(LEN_MAX < MAX_INPUT_LEN);
const _: () = assert!(LEN_MAX < ISIZE_MAX);

/// A format-preserving INI editor. Its tree always equals a parse of its text (C1).
///
/// `Send` and `Sync`: it holds rowan's immutable green tree, whose nodes are shared through atomic
/// reference counts, and every edit replaces the tree with a new one.
///
/// # Cost of an edit
///
/// An edit builds a new tree that shares every line and section it does not touch. A call
/// rebuilds the section whose lines it changes, if any, and the document's root, once: O(lines of
/// that section, plus the preamble's lines and the sections, plus the lines it inserts or
/// removes). It also walks to its position, O(lines), and renders and parses what it writes,
/// O(bytes written). Lookups by name or key are O(sections) or O(lines). Filling or clearing a
/// large section one line per call is therefore quadratic in its size, while one call with all
/// the lines is linear; to load a whole document, build the text and parse it instead. Debug
/// builds also reparse the whole document after every edit to check C1. The
/// [module documentation](crate::editor#costs) lists every term.
#[derive(Debug)]
pub struct Editor {
    /// The document's tree. A commit replaces it with a tree that shares every node the edit
    /// does not touch.
    green: rowan::GreenNode,
    /// The options every parse uses, the reader's options.
    parse_options: ParseOptions,
    /// The formatting of written entries.
    edit_options: EditOptions,
    /// The longest document this editor holds: [`LEN_MAX`] outside unit tests.
    len_max: usize,
}

impl Editor {
    /// Parse `source` with `parse_options`; later edits write entries with `edit_options`. The
    /// editor keeps both and never changes them: every edit is checked against a reader that
    /// uses the same `parse_options`.
    ///
    /// # Errors
    ///
    /// [`EditError::DocumentTooLarge`] if `source` is longer than the editor holds: one byte less
    /// than [`MAX_INPUT_LEN`], or less than `isize::MAX` on targets where that
    /// is smaller. The same limit bounds every edit.
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn new(
        source: &str,
        parse_options: &ParseOptions,
        edit_options: &EditOptions,
    ) -> Result<Self, EditError> {
        // Only unit tests of the limit use a smaller one. No test can reach the error, which
        // needs a 4 GiB source, so the check runs inside `inspect` rather than after a `?`.
        Self::with_len_max(source, parse_options, edit_options, LEN_MAX)
            .inspect(|editor| assert_eq!(editor.len_max, LEN_MAX))
    }

    /// [`Editor::new`] with a smaller size limit, for tests of the limit.
    fn with_len_max(
        source: &str,
        parse_options: &ParseOptions,
        edit_options: &EditOptions,
        len_max: usize,
    ) -> Result<Self, EditError> {
        assert!(len_max <= LEN_MAX);
        if source.len() > len_max {
            return Err(EditError::DocumentTooLarge {
                len: source.len(),
                max: len_max,
            });
        }
        let green = parse_with(source, parse_options).green().clone();
        let editor = Self {
            green,
            parse_options: parse_options.clone(),
            edit_options: edit_options.clone(),
            len_max,
        };
        // The parse lost no byte of the source.
        assert_eq!(editor.len(), source.len());
        assert_eq!(editor.root().text(), source);
        Ok(editor)
    }

    /// The document's text.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn finish(&self) -> String {
        let text = self.green.to_string();
        // rowan's length bookkeeping, which every commit checks, agrees with the text.
        assert_eq!(text.len(), self.len());
        assert!(text.len() <= self.len_max);
        text
    }

    /// A snapshot of the current tree, O(1): later edits replace the editor's tree and leave this
    /// one as it is. Its lines are the editor's body lines: [`File::preamble_lines`] and
    /// [`Section::body_lines`](crate::ast::Section::body_lines) list them in position order.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn file(&self) -> File {
        let root = self.root();
        assert_eq!(usize::from(root.text_range().len()), self.len());
        File::cast(root).expect("the editor's root is a ROOT node")
    }

    /// The options every parse uses.
    #[must_use]
    pub fn parse_options(&self) -> &ParseOptions {
        &self.parse_options
    }

    /// The formatting of written entries.
    #[must_use]
    pub fn edit_options(&self) -> &EditOptions {
        &self.edit_options
    }

    /// The number of sections, nameless `[]` sections included. O(sections).
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections().count()
    }

    /// The preamble: the lines before the first header, after a byte order mark. It always
    /// exists, possibly with no lines.
    #[must_use]
    pub fn preamble(&mut self) -> SectionEditor<'_> {
        SectionEditor::new(self, Address::Preamble)
    }

    /// The first section whose name is exactly `name`. Never changes the document: `None` if
    /// there is none, including for names that no header can have (`""`, `" a"`).
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn section(&mut self, name: &str) -> Option<SectionEditor<'_>> {
        let section = self.find_section(name)?;
        // First match (NEG-12): no section before it has the name.
        #[cfg(all(debug_assertions, not(miri)))]
        assert!(
            self.sections()
                .take_while(|before| *before != section)
                .all(|before| !plan::header_name_is(&before, name))
        );
        let start = section.text_range().start();
        Some(SectionEditor::new(self, Address::Section(start)))
    }

    /// Section `index` in document order, duplicate and nameless sections included.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn section_at(&mut self, index: usize) -> Option<SectionEditor<'_>> {
        let section = self.sections().nth(index)?;
        // Two walks agree: section `index` is at position `index`.
        #[cfg(all(debug_assertions, not(miri)))]
        assert_eq!(
            self.sections().position(|other| other == section),
            Some(index)
        );
        let start = section.text_range().start();
        Some(SectionEditor::new(self, Address::Section(start)))
    }

    /// The first section named `name`, or, if there is none, a new `[name]` section appended at
    /// the end of the document. A blank line separates it from the line before it, unless the
    /// document is empty or that line is blank.
    ///
    /// # Errors
    ///
    /// Only when creating the section, in this order: [`EditError::DocumentTooLarge`];
    /// [`EditError::InvalidSectionName`]; [`EditError::LastLineContinues`].
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn section_or_insert(&mut self, name: &str) -> Result<SectionEditor<'_>, EditError> {
        if let Some(section) = self.find_section(name) {
            let start = section.text_range().start();
            return Ok(SectionEditor::new(self, Address::Section(start)));
        }
        let (plan, section) = self.plan_section(name)?;
        self.commit(plan);
        // The new section is the document's last child, the very node the commit inserted, and the
        // lookup now finds it.
        let last = self.root().last_child().expect("the new section");
        assert!(plan::is_node(&last, &section));
        assert!(self.find_section(name).is_some_and(|found| found == last));
        // A name that reads back is the whole header line, so the section has no body line.
        let address = Address::Section(last.text_range().start());
        assert_eq!(self.body_lines(&self.container(address)).next(), None);
        Ok(SectionEditor::new(self, address))
    }
}
