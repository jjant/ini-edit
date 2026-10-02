//! Longer edit sequences checked against a model that tracks entry identity.

#[path = "support/editor_stateful.rs"]
mod editor_stateful;

#[test]
fn retained_handles_and_duplicate_keys_follow_the_model() {
    for flags in 0u8..8 {
        for spacing in 0u8..3 {
            let mut state = u64::from(flags) * 7 + u64::from(spacing) + 1;
            let mut data = vec![flags, spacing, flags % 3];
            for _ in 0..32 {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                data.extend_from_slice(&state.to_le_bytes()[4..]);
            }
            editor_stateful::check(&data);
        }
    }
}
