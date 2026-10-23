//! Arbitrary editor operations on arbitrary text, each checked against the editor's guarantees
//! by the oracle in `tests/support/editor_ops.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/editor_ops.rs"]
mod editor_ops;

fuzz_target!(|data: &[u8]| editor_ops::check(data));
