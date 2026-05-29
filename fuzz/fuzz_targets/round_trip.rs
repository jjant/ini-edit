#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let parse = ini_edit::parse(s);
        // Must never panic, and must always round-trip.
        assert_eq!(parse.syntax().text().to_string(), s);
    }
});
