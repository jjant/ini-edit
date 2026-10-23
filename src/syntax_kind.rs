//! Syntax kind definitions and the rowan `Language` impl.

use rowan::Language;

/// Discriminant for every leaf and node in the INI syntax tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
#[expect(
    non_camel_case_types,
    reason = "rowan's convention: kinds are named like constants"
)]
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
    /// A full-line comment: optional leading whitespace, the `COMMENT` token,
    /// and its terminating newline.
    COMMENT_LINE,
    /// A blank line: optional whitespace followed by a newline (or, at end of
    /// input, trailing whitespace with no newline).
    BLANK_LINE,
    /// A line that is not a header, entry, comment, or blank line: optional
    /// leading whitespace, the unrecognized text, and its terminating newline.
    ERROR_LINE,
}

impl SyntaxKind {
    /// True if this kind is trivia (whitespace, newlines, or comments).
    #[must_use]
    pub fn is_trivia(self) -> bool {
        matches!(self, Self::WHITESPACE | Self::NEWLINE | Self::COMMENT)
    }
}

/// Defines `SyntaxKind::ALL` from one list of every variant. The generated
/// match has no wildcard arm, so a variant missing from the list fails to
/// compile (E0004).
macro_rules! every_kind {
    ($($kind:ident),+ $(,)?) => {
        impl SyntaxKind {
            /// Every kind in discriminant order, so `ALL[kind as usize] == kind`.
            const ALL: [Self; [$(Self::$kind),+].len()] = [$(Self::$kind),+];
        }

        const _: () = assert!(match SyntaxKind::WHITESPACE {
            $(SyntaxKind::$kind)|+ => true,
        });
    };
}

every_kind!(
    WHITESPACE,
    NEWLINE,
    COMMENT,
    L_BRACK,
    R_BRACK,
    EQ,
    COLON,
    IDENT,
    VALUE_TEXT,
    LEX_ERROR,
    ROOT,
    SECTION,
    SECTION_HEADER,
    ENTRY,
    KEY,
    VALUE,
    COMMENT_LINE,
    BLANK_LINE,
    ERROR_LINE,
);

// `kind_from_raw` indexes `ALL` by discriminant, so the list must follow the
// declaration order. The scan stays inside `assert!` so that cargo-mutants
// leaves it alone: a mutated loop could make the proof vacuous and still
// compile.
const _: () = assert!(
    {
        let mut index = 0;
        while index < SyntaxKind::ALL.len() {
            if SyntaxKind::ALL[index] as usize != index {
                break;
            }
            index += 1;
        }
        index == SyntaxKind::ALL.len()
    },
    "SyntaxKind::ALL must list the kinds in discriminant order"
);

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

/// The rowan `Language` marker for INI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IniLang {}

impl Language for IniLang {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        let index = usize::from(raw.0);
        assert!(
            index < SyntaxKind::ALL.len(),
            "kind out of range: {}",
            raw.0
        );
        SyntaxKind::ALL[index]
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

/// A node in the INI syntax tree: a cursor, which is not `Send` or `Sync`
/// ([Threads](crate#threads)).
pub type SyntaxNode = rowan::SyntaxNode<IniLang>;
/// A token (leaf) in the INI syntax tree: a cursor, which is not `Send` or `Sync`.
pub type SyntaxToken = rowan::SyntaxToken<IniLang>;
/// Either a node or a token: a cursor, which is not `Send` or `Sync`.
pub type SyntaxElement = rowan::SyntaxElement<IniLang>;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use rowan::Language;

    #[test]
    fn is_trivia_classifies_tokens() {
        assert!(SyntaxKind::WHITESPACE.is_trivia());
        assert!(SyntaxKind::NEWLINE.is_trivia());
        assert!(SyntaxKind::COMMENT.is_trivia());
        assert!(!SyntaxKind::IDENT.is_trivia());
        assert!(!SyntaxKind::ENTRY.is_trivia());
    }

    #[test]
    fn kind_raw_round_trip() {
        for kind in [
            SyntaxKind::WHITESPACE,
            SyntaxKind::NEWLINE,
            SyntaxKind::COMMENT,
            SyntaxKind::L_BRACK,
            SyntaxKind::R_BRACK,
            SyntaxKind::EQ,
            SyntaxKind::COLON,
            SyntaxKind::IDENT,
            SyntaxKind::VALUE_TEXT,
            SyntaxKind::LEX_ERROR,
            SyntaxKind::ROOT,
            SyntaxKind::SECTION,
            SyntaxKind::SECTION_HEADER,
            SyntaxKind::ENTRY,
            SyntaxKind::KEY,
            SyntaxKind::VALUE,
            SyntaxKind::COMMENT_LINE,
            SyntaxKind::BLANK_LINE,
            SyntaxKind::ERROR_LINE,
        ] {
            let raw = IniLang::kind_to_raw(kind);
            assert_eq!(IniLang::kind_from_raw(raw), kind);
        }
    }

    #[test]
    #[should_panic(expected = "kind out of range")]
    fn kind_from_raw_out_of_range_panics() {
        let _ = IniLang::kind_from_raw(rowan::SyntaxKind(999));
    }

    #[test]
    #[should_panic(expected = "kind out of range")]
    fn kind_from_raw_rejects_the_first_raw_kind_past_the_last_variant() {
        let past_the_end = u16::try_from(SyntaxKind::ALL.len()).unwrap();
        let _ = IniLang::kind_from_raw(rowan::SyntaxKind(past_the_end));
    }
}
