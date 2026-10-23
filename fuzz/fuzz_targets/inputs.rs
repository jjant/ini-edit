//! Arbitrary keys, values, section names, spacing and raw lines against the bidirectional
//! reference: the oracle in `tests/support/editor_inputs.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[expect(
    dead_code,
    reason = "the target runs the shared oracle, not the sweeps of the tests"
)]
#[path = "../../tests/support/editor_inputs.rs"]
mod editor_inputs;

fuzz_target!(|data: &[u8]| editor_inputs::check(data));
