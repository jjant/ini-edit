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
//! ```
//! use ini_edit::editor::Editor;
//!
//! let ed = Editor::new("[server]\nport = 8080\n");
//! ed.section("server").set("port", "9090");
//! assert!(ed.finish().contains("port = 9090"));
//! ```

pub mod ast;
pub mod editor;
pub(crate) mod green_builders;
pub mod lexer;
pub mod parser;
pub mod syntax_kind;

pub use parser::{Parse, ParseError, parse};
pub use syntax_kind::{IniLang, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
