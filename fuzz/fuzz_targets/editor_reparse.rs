//! Operation sequences with and without a save and reload between steps, which must not
//! differ: the oracle in `tests/support/editor_reparse.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;

// The reload oracle runs its steps through the editor oracle's checks.
#[expect(dead_code, reason = "the editor oracle's own decoder is not used here")]
#[path = "../../tests/support/editor_ops.rs"]
mod editor_ops;
#[path = "../../tests/support/editor_reparse.rs"]
mod editor_reparse;

fuzz_target!(|data: &[u8]| editor_reparse::check(data));
