//! Metamorphic testing of editing with and without a save/reload between steps.

#[path = "support/editor_reparse.rs"]
mod editor_reparse;

#[test]
fn editing_is_independent_of_save_and_reload_boundaries() {
    for source in editor_reparse::SOURCES {
        for first in 0..editor_reparse::OPERATIONS {
            for second in 0..editor_reparse::OPERATIONS {
                for third in 0..editor_reparse::OPERATIONS {
                    editor_reparse::check(source, &[first, second, third]);
                }
            }
        }
    }
}
