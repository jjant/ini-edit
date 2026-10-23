//! Lossless parsing of arbitrary text under every parser-option combination: the oracle in
//! `tests/support/round_trip.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/round_trip.rs"]
mod round_trip;

fuzz_target!(|data: &[u8]| round_trip::check(data));
