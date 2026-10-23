//! Edits of a generated document against its independently generated text and meaning: the
//! oracle in `tests/support/generated_edits.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/generated_edits.rs"]
mod generated_edits;

fuzz_target!(|input: &[u8]| generated_edits::check(input));
