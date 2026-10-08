#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/line_nodes.rs"]
mod line_nodes;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        for flags in 0..4 {
            let options = ini_edit::ParseOptions {
                allow_no_value: flags & 1 != 0,
                inline_comments: flags & 2 != 0,
            };
            let parse = ini_edit::parse_with(s, &options);
            // Every supported grammar must remain lossless.
            assert_eq!(parse.syntax().text().to_string(), s);
            // Editor line indices require one node per physical line.
            line_nodes::check(s, &parse.syntax());
            let mut previous = 0;
            for error in parse.errors() {
                // Diagnostics must point forwards to valid input boundaries.
                assert!(error.offset >= previous);
                assert!(s.is_char_boundary(error.offset));
                previous = error.offset;
            }
        }
    }
});
