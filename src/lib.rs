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

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
// Users copy the examples, so they compile without a warning.
#![doc(test(attr(deny(warnings))))]
// TigerStyle: names are not abbreviated. benches/parse.rs enables the lint as well; integration
// tests are exempt.
#![warn(clippy::min_ident_chars)]

pub mod ast;
pub mod editor;
pub mod lexer;
// Two exceptions in parser.rs, which the lexer and parser changes in flight are rewriting: each
// goes with its code there, and an expectation that is no longer met fails clippy. On the module,
// each covers all of parser.rs, so until then a new instance anywhere in it passes clippy too.
#[expect(
    clippy::let_underscore_must_use,
    reason = "ParseError::display ignores the Ok of each writeln! into a String"
)]
#[cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "malformed_lines_are_single_line_nodes_with_unchanged_diagnostics is a long test"
    )
)]
pub mod parser;
pub mod syntax_kind;

pub use parser::{Parse, ParseError, ParseOptions, parse, parse_with};
pub use syntax_kind::{IniLang, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// The longest input, in bytes, that the parser can represent.
///
/// rowan measures text positions and lengths in `u32` (`TextSize`), so a tree cannot describe more
/// than `u32::MAX` bytes. The [`editor`] holds one byte less: it refuses documents and edits past
/// its limit with [`EditError::DocumentTooLarge`](editor::EditError::DocumentTooLarge).
pub const MAX_INPUT_LEN: usize = u32::MAX as usize;

// The limit is rowan's `TextSize` range.
const _: () = assert!(MAX_INPUT_LEN == u32::MAX as usize);

/// The README's examples, compiled and run as doctests so they cannot rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
