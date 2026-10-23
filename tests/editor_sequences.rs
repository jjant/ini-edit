//! Compare edits against a small independent ordered-entry model after every
//! operation, both through the live AST and after serialization and reparsing.
//! Exhausting short sequences catches interactions that isolated edits miss.

use ini_edit::ast::{AstNode, File};
use ini_edit::editor::{EditError, EditOptions, Editor, SeparatorSpacing};
use ini_edit::{ParseOptions, parse};

fn entries(file: &File) -> Vec<(String, String)> {
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].name().as_deref(), Some("s"));
    sections[0]
        .entries()
        .map(|entry| (entry.key().unwrap(), entry.value().unwrap()))
        .collect()
}

/// Apply operation `operation` to section `s` of `editor` and to `model`.
fn apply(editor: &mut Editor, model: &mut Vec<(String, String)>, operation: usize) {
    let mut section = editor.section("s").unwrap();
    match operation {
        0 => {
            section.set("a", "one").unwrap();
            if let Some(entry) = model.iter_mut().find(|entry| entry.0 == "a") {
                entry.1 = "one".into();
            } else {
                model.push(("a".into(), "one".into()));
            }
        }
        1 => {
            section.append_entry("b", "two").unwrap();
            model.push(("b".into(), "two".into()));
        }
        2 => {
            section.insert_entry(0, "a", "first").unwrap();
            model.insert(0, ("a".into(), "first".into()));
        }
        3 => {
            // The body holds only entries here, so body position 1 is after the first entry.
            let at = section.body_line_count().min(1);
            section.insert_entry(at, "b", "middle").unwrap();
            model.insert(model.len().min(1), ("b".into(), "middle".into()));
        }
        4 => {
            let index = model.iter().position(|entry| entry.0 == "a");
            assert_eq!(section.remove_entry("a"), Ok(index.is_some()));
            if let Some(index) = index {
                model.remove(index);
            }
        }
        5 => {
            let index = model.iter().position(|entry| entry.0 == "a");
            let collides = model.iter().any(|entry| entry.0 == "b");
            let expected = match (index, collides) {
                (None, _) => Ok(false),
                (Some(_), true) => Err(EditError::DuplicateKey { key: "b".into() }),
                (Some(_), false) => Ok(true),
            };
            assert_eq!(section.rename_key("a", "b"), expected);
            if expected == Ok(true) {
                model[index.unwrap()].0 = "b".into();
            }
        }
        6 => {
            if let Some(mut entry) = section.entry_at(0) {
                entry.set_value("").unwrap();
                model[0].1.clear();
            }
        }
        _ => {
            if let Some(entry) = section.entry_at(0) {
                entry.remove().unwrap();
                model.remove(0);
            }
        }
    }
}

#[test]
fn short_edit_sequences_agree_with_an_ordered_model() {
    let parse_options = ParseOptions {
        allow_no_value: false,
        inline_comments: false,
    };
    for source in ["[s]", "[s]\n", "[s]\r\n", "[s]\r"] {
        for spacing in [
            SeparatorSpacing::Preserve,
            SeparatorSpacing::Compact,
            SeparatorSpacing::exact("\t", "  ").unwrap(),
        ] {
            // Eight operations, all sequences of length three. Integer digits
            // encode the sequence, so every failing run is reproducible.
            for sequence in 0..8_usize.pow(3) {
                let edit_options = EditOptions {
                    separator_spacing: spacing.clone(),
                };
                let mut editor = Editor::new(source, &parse_options, &edit_options).unwrap();
                let mut model = Vec::<(String, String)>::new();
                let mut remaining = sequence;
                for step in 0..3 {
                    apply(&mut editor, &mut model, remaining % 8);
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
