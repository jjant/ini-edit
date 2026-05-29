//! A lossless, format-preserving INI parser built on [`rowan`].

pub mod lexer;
pub mod parser;
pub mod syntax_kind;

pub use parser::{Parse, ParseError, parse};
pub use syntax_kind::{IniLang, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
