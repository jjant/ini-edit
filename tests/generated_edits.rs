//! Generated documents have independent semantic and exact-byte expectations.

#[path = "support/generated_edits.rs"]
mod generated_edits;

#[test]
fn generated_edits_preserve_meaning_and_untouched_bytes() {
    let operations = u64::from(generated_edits::OPERATIONS);
    for seed in 0..operations * 24 {
        let mut state = seed + 1;
        let mut input = Vec::new();
        for _ in 0..128 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            input.extend_from_slice(&state.to_le_bytes()[4..]);
        }
        input[0] = u8::try_from(seed % operations).unwrap();
        input[3] = u8::try_from(seed / operations % 3).unwrap();
        input[4] = u8::try_from(seed / (operations * 3) % 4).unwrap();
        generated_edits::check(&input);
    }
}
