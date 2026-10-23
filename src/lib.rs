//! # ini-edit
//!
//! A lossless, format-preserving INI parser and editor built on [`rowan`].
//!
//! ## Parsing
//!
//! ```
//! use ini_edit::ast::{AstNode, File};
//!
//! let parse = ini_edit::parse("[server]\nhost = 0.0.0.0\nport = 8080\n");
//! assert!(parse.errors().is_empty());
//!
//! // Lossless round-trip
//! assert_eq!(parse.syntax().text().to_string(), "[server]\nhost = 0.0.0.0\nport = 8080\n");
//!
//! // Typed traversal
//! let file = File::cast(parse.syntax()).unwrap();
//! let server = file.sections().next().unwrap();
//! assert_eq!(server.name().as_deref(), Some("server"));
//! ```
//!
//! ## Editing
//!
//! Every edit is checked against the parser before it is applied: an edit that a reader would not
//! read back as written returns an [`EditError`](editor::EditError) and leaves the document
//! unchanged. See the [`editor`] module.
//!
//! ```
//! use ini_edit::ParseOptions;
//! use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};
//!
//! let parse_options = ParseOptions { allow_no_value: false, inline_comments: false };
//! let edit_options = EditOptions { separator_spacing: SeparatorSpacing::Preserve };
//! let mut editor = Editor::new("[server]\nport = 8080\n", &parse_options, &edit_options)?;
//! editor.section_or_insert("server")?.set("port", "9090")?;
//! assert_eq!(editor.finish(), "[server]\nport = 9090\n");
//! # Ok::<(), ini_edit::editor::EditError>(())
//! ```
//!
//! ## Threads
//!
//! [`Parse`] and [`Editor`](editor::Editor) own rowan's green tree, which is immutable and shares
//! its nodes through atomic reference counts. Both are `Send` and `Sync`: move one to another
//! thread, or share `&Parse` or `&Editor` between threads that read it at the same time. So are
//! the editor's handles, [`SectionEditor`](editor::SectionEditor) and
//! [`EntryEditor`](editor::EntryEditor), and the types that hold no tree: the options, the
//! errors, [`SyntaxKind`], [`IniLang`] and [`lexer::Token`]. All of them are also `UnwindSafe` and
//! `RefUnwindSafe`, except that the handles, like every `&mut` borrow, are not `UnwindSafe`.
//!
//! A syntax tree, such as the ones that [`Parse::syntax`] and
//! [`Editor::file`](editor::Editor::file) return, is a rowan cursor, which counts its references
//! without atomics. [`SyntaxNode`], [`SyntaxToken`], [`SyntaxElement`] and the [`ast`] types are
//! therefore not `Send`, `Sync`, `UnwindSafe` or `RefUnwindSafe`: a cursor stays in the thread
//! that made it. To read one tree in several threads, share the parse or the editor and build a
//! cursor in each thread. A green tree crosses threads too: [`Parse::green`] returns one,
//! `node.green().to_owned()` returns the one under any cursor in O(1), sharing its nodes, and
//! `SyntaxNode::new_root` builds a cursor on it.
//!
//! ```
//! use ini_edit::ast::{AstNode, File};
//!
//! let parse = ini_edit::parse("[a]\nk = 1\n[b]\n");
//! std::thread::scope(|scope| {
//!     for _ in 0..2 {
//!         scope.spawn(|| {
//!             let file = File::cast(parse.syntax()).expect("the root is a file");
//!             assert_eq!(file.sections().count(), 2);
//!         });
//!     }
//! });
//! ```
//!
//! A cursor that moves to another thread does not compile:
//!
//! ```compile_fail,E0277
//! use ini_edit::ast::{AstNode, File};
//!
//! let file = File::cast(ini_edit::parse("[a]\n").syntax()).expect("the root is a file");
//! let count = std::thread::spawn(move || file.sections().count());
//! assert_eq!(count.join().expect("no panic"), 1);
//! ```
//!
//! These traits are part of the API: tests pin every one that each public type has or lacks. The
//! crate itself starts no thread and holds no lock.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
// Users copy the examples, so they compile without a warning.
#![doc(test(attr(deny(warnings))))]
// TigerStyle: names are not abbreviated. benches/parse.rs enables the lint as well; integration
// tests are exempt.
#![warn(clippy::min_ident_chars)]

pub mod ast;
pub mod editor;
pub mod lexer;
pub mod parser;
pub mod syntax_kind;

pub use parser::{Parse, ParseError, ParseOptions, parse, parse_with};
pub use syntax_kind::{IniLang, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// The longest input, in bytes, that the parser can represent.
///
/// rowan measures text positions and lengths in `u32` (`TextSize`), so a tree cannot describe more
/// than `u32::MAX` bytes. [`parse`] and [`parse_with`] panic on a longer input before they read any
/// of it, in every build, rather than build a tree whose length has wrapped around. The
/// [`editor`] holds one byte less: it refuses documents and edits past its limit with
/// [`EditError::DocumentTooLarge`](editor::EditError::DocumentTooLarge).
pub const MAX_INPUT_LEN: usize = u32::MAX as usize;

// The limit is rowan's `TextSize` range.
const _: () = assert!(MAX_INPUT_LEN == u32::MAX as usize);

/// The README's examples, compiled and run as doctests so they cannot rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

#[cfg(doctest)]
pub mod auto_traits;
