#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/editor_reparse.rs"]
mod editor_reparse;

fuzz_target!(|data: &[u8]| {
    if let Some((&source, operations)) = data.split_first() {
        let source = editor_reparse::SOURCES[usize::from(source) % editor_reparse::SOURCES.len()];
        editor_reparse::check(source, &operations[..operations.len().min(64)]);
    }
});
