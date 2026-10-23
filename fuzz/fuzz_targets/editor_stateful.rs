//! Edits, duplicate keys and refused input against an independent identity-based model: the
//! oracle in `tests/support/editor_stateful.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/editor_stateful.rs"]
mod editor_stateful;

fuzz_target!(|data: &[u8]| editor_stateful::check(data));
