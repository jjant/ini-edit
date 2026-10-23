//! The one error type of every editing operation.

use std::fmt;

/// Why the editor refused an edit. A refused edit changes nothing: the text and the tree are
/// exactly as they were before the call.
///
/// The enum is exhaustive, so callers can handle every variant; adding one is a breaking change.
/// Positions (`at`, `start`, `end`) count body lines, the lines of a section after its header or
/// the lines of the preamble; lengths count bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The document would grow past the size the editor can hold.
    DocumentTooLarge {
        /// The length in bytes the document would have, saturating at `usize::MAX`.
        len: usize,
        /// The most bytes the editor holds.
        max: usize,
    },
    /// Separator spacing that is not a run of spaces and tabs.
    InvalidSpacing {
        /// The refused spacing.
        text: String,
    },
    /// A key that a reader would not read back as that key.
    InvalidKey {
        /// The refused key.
        key: String,
    },
    /// A value that a reader would not read back as that value, or one whose last line ends in a
    /// backslash.
    InvalidValue {
        /// The refused value.
        value: String,
    },
    /// A section name that a reader would not read back as that name.
    InvalidSectionName {
        /// The refused name.
        name: String,
    },
    /// An element of a raw line slice that is not exactly one body line.
    InvalidLine {
        /// The index of the element in the slice.
        index: usize,
        /// The refused element.
        line: String,
    },
    /// A rename onto a key that another entry of the same section or preamble already has.
    DuplicateKey {
        /// The key that already exists.
        key: String,
    },
    /// An insertion position past the last body line.
    PositionOutOfRange {
        /// The refused position.
        at: usize,
        /// The number of body lines of the section or preamble.
        body_line_count: usize,
    },
    /// A removal range that is reversed or ends past the last body line.
    RangeOutOfRange {
        /// The start of the refused range.
        start: usize,
        /// The end of the refused range.
        end: usize,
        /// The number of body lines of the section or preamble.
        body_line_count: usize,
    },
    /// An insertion after the document's last line, which has no line break and ends in a
    /// continuation backslash, so the new line would become part of its value.
    LastLineContinues,
    /// An edit that would make U+FEFF the first character of a document without a byte order
    /// mark, where a reader would take it as one.
    ByteOrderMarkAtStart,
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DocumentTooLarge { len, max } => write!(
                formatter,
                "the document would be {len} bytes long, more than the {max} bytes the editor \
                 can hold"
            ),
            Self::InvalidSpacing { text } => write!(
                formatter,
                "{text:?} is not separator spacing: spacing may contain only spaces and tabs"
            ),
            Self::InvalidKey { key } => write!(
                formatter,
                "{key:?} cannot be written as a key: keys must not be empty, must not contain \
                 '=', ':', a space, a tab or a line break, and must not start with ';', '#' or \
                 '['"
            ),
            Self::InvalidValue { value } => write!(
                formatter,
                "{value:?} cannot be written as a value: values must not start or end with a \
                 space or tab, may break lines only right after a continuation backslash, must \
                 not end their last line with a backslash, and, with inline comments enabled, \
                 must not contain ';' or '#' after a space or tab that follows other text on \
                 their last line"
            ),
            Self::InvalidSectionName { name } => write!(
                formatter,
                "{name:?} cannot be written as a section name: names must not be empty, must not \
                 contain ']' or a line break, and must not start or end with a space or tab"
            ),
            Self::InvalidLine { index, line } => write!(
                formatter,
                "line {index} ({line:?}) cannot be inserted: each element must be exactly one \
                 line that is not a section header and does not end with a continuation \
                 backslash; only an entry's value may continue onto further lines"
            ),
            Self::DuplicateKey { key } => write!(
                formatter,
                "another entry in the same section or preamble already has the key {key:?}"
            ),
            Self::PositionOutOfRange {
                at,
                body_line_count,
            } => write!(
                formatter,
                "position {at} is past the end of a body with {body_line_count} lines"
            ),
            Self::RangeOutOfRange {
                start,
                end,
                body_line_count,
            } => write!(
                formatter,
                "lines {start}..{end} are not a range within a body with {body_line_count} lines"
            ),
            Self::LastLineContinues => formatter.write_str(
                "the document's last line has no line break and ends with a continuation \
                 backslash, so a line added after it would become part of its value",
            ),
            Self::ByteOrderMarkAtStart => formatter.write_str(
                "the edit would start a document that has no byte order mark with U+FEFF, which \
                 a reader would take as a byte order mark",
            ),
        }
    }
}

impl std::error::Error for EditError {}
