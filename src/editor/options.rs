//! Formatting options for the lines the editor writes.

use super::error::EditError;

/// Formatting for the entries the editor creates, and for the entries whose value it assigns.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditOptions {
    /// Whitespace around the `=` or `:` separator of written entries.
    pub separator_spacing: SeparatorSpacing,
}

/// Whitespace around the separator of the entries the editor writes.
///
/// New entries always use `=`. An assigned entry keeps its separator character.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SeparatorSpacing {
    /// Keep an assigned entry's whitespace around its separator. Write `key = value` for new
    /// entries, and for bare keys and `key junk` lines that gain a separator.
    #[default]
    Preserve,
    /// Write `key=value`.
    Compact,
    /// Write exactly `key{before}={after}value`.
    Exact {
        /// The whitespace between the key and the separator.
        before: Whitespace,
        /// The whitespace between the separator and the value.
        after: Whitespace,
    },
}

impl SeparatorSpacing {
    /// [`Exact`](Self::Exact) spacing from two strings.
    ///
    /// ```
    /// use ini_edit::editor::{EditError, SeparatorSpacing};
    ///
    /// let spacing = SeparatorSpacing::exact("\t", "  ")?;
    /// assert!(matches!(spacing, SeparatorSpacing::Exact { .. }));
    /// assert!(SeparatorSpacing::exact(" ;", "").is_err());
    /// # Ok::<(), EditError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`EditError::InvalidSpacing`] for the first string, `before` then `after`, that contains
    /// anything other than spaces and tabs.
    pub fn exact(before: &str, after: &str) -> Result<SeparatorSpacing, EditError> {
        Ok(SeparatorSpacing::Exact {
            before: Whitespace::new(before)?,
            after: Whitespace::new(after)?,
        })
    }

    /// The whitespace written before and after the separator of a new entry, and of an entry
    /// whose separator gap this policy replaces.
    pub(super) fn new_gap(&self) -> (&str, &str) {
        let (before, after) = match self {
            SeparatorSpacing::Preserve => (" ", " "),
            SeparatorSpacing::Compact => ("", ""),
            SeparatorSpacing::Exact { before, after } => (before.as_str(), after.as_str()),
        };
        // Whatever the policy, the gap is spacing that a reader reads back as spacing.
        assert_eq!(before.trim_start_matches([' ', '\t']), "");
        assert_eq!(after.trim_start_matches([' ', '\t']), "");
        (before, after)
    }
}

/// A run of spaces and tabs, possibly empty: the only separator spacing that a reader reads back
/// as spacing. Anything else would become part of the key or the value, or start a new line.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Whitespace(Box<str>);

impl Whitespace {
    /// Spacing made of `text`.
    ///
    /// # Errors
    ///
    /// [`EditError::InvalidSpacing`] if `text` contains anything other than spaces and tabs.
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn new(text: &str) -> Result<Whitespace, EditError> {
        // The lexer reads the same set of characters around a separator as spacing.
        if text.trim_start_matches([' ', '\t']).is_empty() {
            Ok(Whitespace(text.into()))
        } else {
            // Empty spacing is valid, so refused spacing holds a character that is not spacing.
            assert_ne!(text, "");
            Err(EditError::InvalidSpacing {
                text: text.to_owned(),
            })
        }
    }

    /// The spacing text.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "asserts internal invariants, never caller input"
    )]
    pub fn as_str(&self) -> &str {
        // Paired with the check in `new`, the only way to build one.
        assert_eq!(self.0.trim_start_matches([' ', '\t']), "");
        &self.0
    }
}
