//! Arbitrary text must parse losslessly under every parser-option combination,
//! with one node per line (an entry whose value continues over several physical
//! lines is one line) and diagnostics at valid input boundaries.
//! Diagnostics also depend only on the text before them: parsing just the
//! lines before a line reports exactly the diagnostics that precede it.

#[path = "line_nodes.rs"]
mod line_nodes;

use ini_edit::{Parse, ParseOptions, parse_with};

/// Every parser-option combination.
pub(crate) const OPTIONS: [ParseOptions; 4] = [
    ParseOptions {
        allow_no_value: false,
        inline_comments: false,
    },
    ParseOptions {
        allow_no_value: true,
        inline_comments: false,
    },
    ParseOptions {
        allow_no_value: false,
        inline_comments: true,
    },
    ParseOptions {
        allow_no_value: true,
        inline_comments: true,
    },
];

/// Check `data` as source text; input that is not UTF-8 is ignored.
pub(crate) fn check(data: &[u8]) {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let starts = line_starts(source);
    let boundary = starts[starts.len() - 1];
    if !source.is_empty() {
        // Even a single line, or one ending in a line break, loses a line.
        assert!(boundary < source.len(), "{source:?}");
    }
    // The last line has no line break but its own terminator.
    let last = &source[boundary..];
    let body = last
        .strip_suffix("\r\n")
        .or_else(|| last.strip_suffix(['\n', '\r']))
        .unwrap_or(last);
    assert!(
        !body.contains(['\n', '\r']),
        "{source:?}: two lines after byte {boundary}"
    );
    for options in &OPTIONS {
        let parse = check_parse(source, options);
        let prefix = parse_with(&source[..boundary], options);
        assert_eq!(
            errors_before(&parse, boundary),
            errors_before(&prefix, boundary),
            "{source:?} without the line at byte {boundary}, {options:?}"
        );
    }
}

/// Parse `source` and check what every parse guarantees: the tree is
/// lossless, each line is one node (a continued entry is one line), and
/// diagnostics are in order at character boundaries.
pub(crate) fn check_parse(source: &str, options: &ParseOptions) -> Parse {
    let parse = parse_with(source, options);
    let root = parse.syntax();
    // Every supported grammar must remain lossless.
    assert_eq!(root.text(), source, "{options:?}");
    // Editor positions address line nodes, one per line.
    line_nodes::check(source, &root);
    let mut previous = 0;
    for error in parse.errors() {
        // Diagnostics must point forwards to valid input boundaries.
        assert!(error.offset >= previous, "{source:?}: {error:?}");
        assert!(
            source.is_char_boundary(error.offset),
            "{source:?}: {error:?}"
        );
        previous = error.offset;
    }
    parse
}

/// The offset and message of each diagnostic that starts before `boundary`.
pub(crate) fn errors_before(parse: &Parse, boundary: usize) -> Vec<(usize, &str)> {
    let errors = parse.errors();
    let before: Vec<_> = errors
        .iter()
        .take_while(|error| error.offset < boundary)
        .map(|error| (error.offset, error.message.as_str()))
        .collect();
    // Diagnostics are in order, so none of the rest starts before `boundary`.
    for error in &errors[before.len()..] {
        assert!(
            error.offset >= boundary,
            "{error:?} is out of order: {errors:?}"
        );
    }
    before
}

/// Where each line of `source` starts. A final line break ends the last line
/// instead of starting an empty one, so that the usual file, which ends with
/// a newline, has one line per line break.
pub(crate) fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let mut end = line_end(source, 0);
    while end < source.len() {
        assert!(
            source[..end].ends_with(['\n', '\r']),
            "{source:?}: line {} starts at byte {end}",
            starts.len()
        );
        starts.push(end);
        end = line_end(source, end);
    }
    starts
}

/// The end of the line that holds byte `at`, after its line break. Like the
/// lexer's NEWLINE tokens, a line ends after "\n", after "\r\n", and after a
/// "\r" that no "\n" follows.
pub(crate) fn line_end(source: &str, at: usize) -> usize {
    let bytes = source.as_bytes();
    let end = match bytes[at..]
        .iter()
        .position(|&byte| byte == b'\n' || byte == b'\r')
    {
        None => bytes.len(),
        Some(offset) if bytes[at + offset..].starts_with(b"\r\n") => at + offset + 2,
        Some(offset) => at + offset + 1,
    };
    // Every line holds at least one byte, and "\r\n" is one line break.
    if at < bytes.len() {
        assert!(end > at, "{source:?}: line at byte {at}");
    }
    if bytes[..end].ends_with(b"\r") {
        assert_ne!(bytes.get(end), Some(&b'\n'), "{source:?}: byte {end}");
    }
    end
}
