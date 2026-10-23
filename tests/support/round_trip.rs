//! Arbitrary text must parse losslessly under every parser-option combination,
//! with one node per physical line and diagnostics at valid input boundaries.

#[path = "line_nodes.rs"]
mod line_nodes;

use ini_edit::{ParseOptions, parse_with};

/// Check `data` as source text; input that is not UTF-8 is ignored.
pub fn check(data: &[u8]) {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    for flags in 0u8..4 {
        let options = ParseOptions {
            allow_no_value: flags & 1 != 0,
            inline_comments: flags & 2 != 0,
        };
        let parse = parse_with(source, &options);
        // Every supported grammar must remain lossless.
        assert_eq!(parse.syntax().text().to_string(), source);
        // Editor line indices require one node per physical line.
        line_nodes::check(source, &parse.syntax());
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
    }
}
