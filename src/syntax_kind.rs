//! Syntax kind definitions and the rowan `Language` impl.

use rowan::Language;

/// Discriminant for every leaf and node in the INI syntax tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum SyntaxKind {
    /// Spaces and tabs (never includes newlines).
    WHITESPACE = 0,
    /// A single line terminator (`\n`, `\r\n`, or `\r`).
    NEWLINE,
    /// A comment line (`;` or `#` to end of line).
    COMMENT,
    /// `[`
    L_BRACK,
    /// `]`
    R_BRACK,
    /// `=`
    EQ,
    /// `:`
    COLON,
    /// A section name or key identifier.
    IDENT,
    /// The value text on the RHS of `=`/`:`.
    VALUE_TEXT,
    /// Unrecognized character sequence.
    LEX_ERROR,
    /// Root node (whole file).
    ROOT,
    /// A `[name]` section with its entries.
    SECTION,
    /// The `[name]` header.
    SECTION_HEADER,
    /// A key-value entry.
    ENTRY,
    /// Wraps the key identifier.
    KEY,
    /// Wraps the value text.
    VALUE,
}

impl SyntaxKind {
    /// True if this kind is trivia (whitespace, newlines, or comments).
    #[must_use]
    pub fn is_trivia(self) -> bool {
        matches!(self, Self::WHITESPACE | Self::NEWLINE | Self::COMMENT)
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

/// The rowan `Language` marker for INI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IniLang {}

impl Language for IniLang {
    type Kind = SyntaxKind;

    #[allow(clippy::missing_panics_doc)] // Panic is unreachable: rowan only passes values we produced via kind_to_raw.
    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        match raw.0 {
            0 => SyntaxKind::WHITESPACE,
            1 => SyntaxKind::NEWLINE,
            2 => SyntaxKind::COMMENT,
            3 => SyntaxKind::L_BRACK,
            4 => SyntaxKind::R_BRACK,
            5 => SyntaxKind::EQ,
            6 => SyntaxKind::COLON,
            7 => SyntaxKind::IDENT,
            8 => SyntaxKind::VALUE_TEXT,
            9 => SyntaxKind::LEX_ERROR,
            10 => SyntaxKind::ROOT,
            11 => SyntaxKind::SECTION,
            12 => SyntaxKind::SECTION_HEADER,
            13 => SyntaxKind::ENTRY,
            14 => SyntaxKind::KEY,
            15 => SyntaxKind::VALUE,
            _ => panic!("kind out of range: {}", raw.0),
        }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

/// A node in the INI syntax tree.
pub type SyntaxNode = rowan::SyntaxNode<IniLang>;
/// A token (leaf) in the INI syntax tree.
pub type SyntaxToken = rowan::SyntaxToken<IniLang>;
/// Either a node or a token.
pub type SyntaxElement = rowan::SyntaxElement<IniLang>;
