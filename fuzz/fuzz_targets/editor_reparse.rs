#![no_main]

use libfuzzer_sys::fuzz_target;

// The reload oracle runs its steps through the editor oracle's checks; the editor oracle's own
// decoder is not used here.
#[allow(dead_code)]
#[path = "../../tests/support/editor_ops.rs"]
mod editor_ops;
#[path = "../../tests/support/editor_reparse.rs"]
mod editor_reparse;

fuzz_target!(|data: &[u8]| editor_reparse::check(data));
