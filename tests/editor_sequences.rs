//! Compare edits against a small independent ordered-entry model after every
//! operation, both through the live AST and after serialization and reparsing.
//! Exhausting short sequences catches interactions that isolated edits miss.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};
use ini_edit::parse;

fn entries(file: &File) -> Vec<(String, String)> {
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].name().as_deref(), Some("s"));
    sections[0]
        .entries()
        .map(|entry| (entry.key().unwrap(), entry.value().unwrap()))
        .collect()
}

#[test]
fn short_edit_sequences_agree_with_an_ordered_model() {
    for source in ["[s]", "[s]\n", "[s]\r\n", "[s]\r"] {
        for spacing in [
            SeparatorSpacing::Preserve,
            SeparatorSpacing::Compact,
            SeparatorSpacing::exact("\t", "  "),
        ] {
            // Eight operations, all sequences of length three. Integer digits
            // encode the sequence, so every failing run is reproducible.
            for sequence in 0..8_usize.pow(3) {
                let editor = Editor::with_edit_options(
                    source,
                    &EditOptions {
                        separator_spacing: spacing.clone(),
                    },
                );
                let mut model = Vec::<(String, String)>::new();
                let mut remaining = sequence;
                for step in 0..3 {
                    let section = editor.section("s");
                    match remaining % 8 {
                        0 => {
                            section.set("a", "one");
                            if let Some(entry) = model.iter_mut().find(|entry| entry.0 == "a") {
                                entry.1 = "one".into();
                            } else {
                                model.push(("a".into(), "one".into()));
                            }
                        }
                        1 => {
                            section.append_entry("b", "two");
                            model.push(("b".into(), "two".into()));
                        }
                        2 => {
                            section.insert_entry_at_line(0, "a", "first");
                            model.insert(0, ("a".into(), "first".into()));
                        }
                        3 => {
                            section.insert_entry_at_line(1, "b", "middle");
                            model.insert(model.len().min(1), ("b".into(), "middle".into()));
                        }
                        4 => {
                            let index = model.iter().position(|entry| entry.0 == "a");
                            assert_eq!(section.remove_entry("a"), index.is_some());
                            if let Some(index) = index {
                                model.remove(index);
                            }
                        }
                        5 => {
                            let index = model.iter().position(|entry| entry.0 == "a");
                            let can_rename =
                                index.is_some() && !model.iter().any(|entry| entry.0 == "b");
                            assert_eq!(section.rename_key("a", "b"), can_rename);
                            if can_rename {
                                model[index.unwrap()].0 = "b".into();
                            }
                        }
                        6 => {
                            if let Some(entry) = section.entries_mut().first() {
                                entry.set_value("");
                                model[0].1.clear();
                            }
                        }
                        _ => {
                            if let Some(entry) = section.entries_mut().into_iter().next() {
                                entry.remove();
                                model.remove(0);
                            }
                        }
                    }
                    remaining /= 8;
                    assert_eq!(
                        entries(&editor.file()),
                        model,
                        "sequence={sequence} step={step}"
                    );
                    let output = editor.finish();
                    let parsed = parse(&output);
                    assert!(parsed.errors().is_empty(), "{output:?}");
                    assert_eq!(
                        entries(&File::cast(parsed.syntax()).unwrap()),
                        model,
                        "source={source:?} sequence={sequence} step={step} output={output:?}"
                    );
                }
            }
        }
    }
}
