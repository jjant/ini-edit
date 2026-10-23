#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/editor_reparse.rs"]
mod editor_reparse;

fuzz_target!(|data: &[u8]| editor_reparse::check(data));
