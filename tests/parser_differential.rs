//! Compare a shared INI subset against two independent parsers and the model
//! used to generate it. Dialect-specific quoting, escapes, duplicate keys,
//! continuations, and inline comments are deliberately outside this subset.

use std::collections::BTreeMap;
use std::fmt::Write;

use ini_edit::ast::{AstNode, File};
use ini_edit::{ParseOptions, parse_with};

type Document = BTreeMap<Option<String>, BTreeMap<String, String>>;

fn next(state: &mut usize, count: usize) -> usize {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) & 0xffff_ffff;
    (*state >> 16) % count
}

fn generate(seed: usize) -> (String, Document) {
    let mut state = seed;
    let endings = ["\n", "\r\n"];
    let spaces = ["", " ", "\t", " \t"];
    let values = [
        "",
        "0",
        "two words",
        "λ",
        "https://host/path?x=1",
        "a:b=c",
        "hash#fragment",
        "comma,list",
        "-12",
        "true",
    ];
    let mut source = String::new();
    let mut model = Document::new();
    for section in [None, Some("server"), Some("数据库")] {
        let ending = endings[next(&mut state, endings.len())];
        writeln!(source, "; generated case {seed}").unwrap();
        if let Some(name) = section {
            let indent = spaces[next(&mut state, spaces.len())];
            write!(source, "{indent}[{name}]{ending}").unwrap();
        }
        let mut entries = BTreeMap::new();
        for key in ["first", "Second", "κλειδί"] {
            let value = values[next(&mut state, values.len())];
            let indent = spaces[next(&mut state, spaces.len())];
            let before = spaces[next(&mut state, spaces.len())];
            let after = spaces[next(&mut state, spaces.len())];
            let trailing = spaces[next(&mut state, spaces.len())];
            let separator = ["=", ":"][next(&mut state, 2)];
            write!(
                source,
                "{indent}{key}{before}{separator}{after}{value}{trailing}{ending}"
            )
            .unwrap();
            entries.insert(key.to_owned(), value.to_owned());
        }
        model.insert(section.map(str::to_owned), entries);
    }
    if seed % 2 == 0 {
        source.truncate(source.trim_end_matches(['\r', '\n']).len());
    }
    (source, model)
}

fn read_ours(source: &str, options: &ParseOptions) -> Document {
    let parsed = parse_with(source, options);
    assert!(
        parsed.errors().is_empty(),
        "{source:?}: {:?}",
        parsed.errors()
    );
    assert_eq!(parsed.syntax().text().to_string(), source);
    let file = File::cast(parsed.syntax()).unwrap();
    let entries = |items: Vec<ini_edit::ast::Entry>| {
        items
            .into_iter()
            .map(|entry| (entry.key().unwrap(), entry.value().unwrap()))
            .collect()
    };
    let mut document = Document::new();
    document.insert(None, entries(file.preamble_entries().collect()));
    for section in file.sections() {
        document.insert(section.name(), entries(section.entries().collect()));
    }
    document
}

#[test]
fn generated_documents_agree_with_independent_parsers_and_their_model() {
    for seed in 0..1_024 {
        let (source, expected) = generate(seed);
        let reference = ini::Ini::load_from_str(&source).unwrap();
        let rust_ini: Document = reference
            .iter()
            .map(|(section, properties)| {
                (
                    section.map(str::to_owned),
                    properties
                        .iter()
                        .map(|(k, v)| (k.to_owned(), v.to_owned()))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(rust_ini, expected, "rust-ini seed={seed} source={source:?}");

        let mut reference = configparser::ini::Ini::new_cs();
        reference.set_default_section("__preamble__");
        reference.set_inline_comment_symbols(Some(&[]));
        let configparser: Document = reference
            .read(source.clone())
            .unwrap()
            .into_iter()
            .map(|(section, properties)| {
                (
                    (section != "__preamble__").then_some(section),
                    properties
                        .into_iter()
                        .map(|(k, v)| (k, v.unwrap()))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            configparser, expected,
            "configparser seed={seed} source={source:?}"
        );

        for flags in 0..4 {
            let options = ParseOptions {
                allow_no_value: flags & 1 != 0,
                inline_comments: flags & 2 != 0,
            };
            assert_eq!(
                read_ours(&source, &options),
                expected,
                "ini-edit seed={seed} flags={flags} source={source:?}"
            );
        }
    }
}
