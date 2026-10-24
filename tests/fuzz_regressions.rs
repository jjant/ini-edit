//! The oracles behind the fuzz targets, run by ordinary `cargo test`, and the editor's other
//! oracles.
//!
//! Every fuzz target is a thin wrapper around a `check(&[u8])` oracle in
//! `tests/support/`. These tests run each oracle over a fixed sweep and over
//! every committed fuzz input, like SQLite's fuzzcheck.
//! `tests/fuzz_regressions/<target>/` holds the `cargo fuzz cmin` corpus and
//! every crash or timeout reproducer, so past findings are checked on every
//! platform and toolchain. The hostile-input oracle and the random walk check
//! the editor's guarantees over generated input, and the grammar oracle checks
//! that the editor accepts exactly what its documentation says it accepts.
//!
//! Five extended tests are `#[ignore]`d: the reload triples, the full two- and
//! three-piece hostile products, a 60,000-seed walk, and the grammar over
//! three pieces. CI's `extended-tests` job runs them with release speed and
//! debug assertions, which takes about two minutes on four threads:
//! `cargo test --profile release-checked --test fuzz_regressions -- --ignored`.
//!
//! The tests share one binary because cargo-mutants rebuilds every test
//! binary for every mutant.

#[path = "support/editor_inputs.rs"]
mod editor_inputs;
#[path = "support/editor_ops.rs"]
mod editor_ops;
#[path = "support/editor_reparse.rs"]
mod editor_reparse;
#[path = "support/editor_stateful.rs"]
mod editor_stateful;
#[path = "support/editor_walk.rs"]
mod editor_walk;
#[path = "support/generated_edits.rs"]
mod generated_edits;
#[path = "support/round_trip.rs"]
mod round_trip;

use ini_edit::ParseOptions;
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};

/// Every operation sequence of `length` over every source, with and without a save and reload
/// between steps.
fn reload_sequences(length: u32) {
    let sources = u8::try_from(editor_reparse::SOURCES.len()).unwrap();
    let kinds = u32::from(editor_reparse::OPERATION_KINDS);
    for source in 0..sources {
        for sequence in 0..kinds.pow(length) {
            let mut data = vec![source];
            let mut rest = sequence;
            for _ in 0..length {
                data.push(u8::try_from(rest % kinds).unwrap());
                rest /= kinds;
            }
            editor_reparse::check(&data);
        }
    }
}

/// Metamorphic testing of editing with and without a save/reload between steps: every pair of
/// operations on every source.
#[test]
fn editing_is_independent_of_save_and_reload_boundaries() {
    reload_sequences(2);
}

/// Every triple of operations on every source.
#[test]
#[ignore = "extended; run in the extended-tests CI job with --profile release-checked"]
fn editing_is_independent_of_save_and_reload_boundaries_for_triples() {
    reload_sequences(3);
}

fn options(allow_no_value: bool, inline_comments: bool) -> ParseOptions {
    ParseOptions {
        allow_no_value,
        inline_comments,
    }
}

/// Hostile strings of up to two pieces: the editor accepts exactly what a reader reads back.
#[test]
fn hostile_input_is_refused_exactly_when_a_reader_would_not_read_it_back() {
    let pieces = ["", " ", "\n", "\r", "=", "[", ";", "\\", "\u{FEFF}", "a"];
    let documents = [
        "[s]\na=1\nb=2\n",
        "[s]\ra=1\n\n",
        "[s]\na=C:\\x\\",
        "",
        "g=1 ; c\n",
    ];
    let options = [options(false, false), options(true, true)];
    let counts = editor_inputs::run(&editor_inputs::Product {
        count: 2,
        pieces: &pieces,
        documents: &documents,
        options: &options,
        spacings: &[SeparatorSpacing::Preserve],
    });
    assert_eq!(counts.cases, 8_736);
    assert_eq!(
        (
            counts.false_accepts,
            counts.false_rejects,
            counts.byte_mismatches
        ),
        (0, 0, 0),
        "{counts:?}"
    );
}

/// The full products of the hostile pieces, at every site of every document.
fn hostile_products(count: usize) {
    let options = [
        options(false, false),
        options(true, false),
        options(false, true),
        options(true, true),
    ];
    let spacings = [
        SeparatorSpacing::Preserve,
        SeparatorSpacing::Compact,
        SeparatorSpacing::exact("\t", "  ").unwrap(),
    ];
    let counts = editor_inputs::run(&editor_inputs::Product {
        count,
        pieces: &editor_inputs::PIECES,
        documents: &editor_inputs::DOCUMENTS,
        options: &options,
        spacings: &spacings,
    });
    eprintln!("{counts:?}");
    assert_eq!(
        (
            counts.false_accepts,
            counts.false_rejects,
            counts.byte_mismatches
        ),
        (0, 0, 0),
        "{counts:?}"
    );
}

#[test]
#[ignore = "extended; run in the extended-tests CI job with --profile release-checked"]
fn hostile_input_products_of_two_pieces() {
    hostile_products(2);
}

#[test]
#[ignore = "extended; run in the extended-tests CI job with --profile release-checked"]
fn hostile_input_products_of_three_pieces() {
    hostile_products(3);
}

/// `section_or_insert` with the name of an existing section returns that section and writes
/// nothing. The reference predicted a second `[s]` instead, so the `inputs` fuzz target
/// reported a byte mismatch for a correct editor; the reproducer is
/// `tests/fuzz_regressions/inputs/crash-existing-section-name`. No hostile piece is a section
/// name of the documents, so the products never reached the lookup.
#[test]
fn the_hostile_input_reference_finds_an_existing_section() {
    let document = "[s]\na=1\nb=2\n";
    let reference = editor_inputs::reference(editor_inputs::Case {
        document,
        options: &options(false, false),
        spacing: &SeparatorSpacing::Preserve,
        site: editor_inputs::Site::Section,
        text: "s",
    });
    assert_eq!(reference, Some((true, document.to_owned())));
}

/// A random walk of 300 documents with 12 steps each.
#[test]
fn random_edits_keep_every_guarantee() {
    let [succeeded, refused, missed] = editor_walk::walk(300, 12);
    // The walk reaches both outcomes.
    assert!(succeeded > 0, "{succeeded} {refused} {missed}");
    assert!(refused > 0, "{succeeded} {refused} {missed}");
}

#[test]
#[ignore = "extended; run in the extended-tests CI job with --profile release-checked"]
fn random_edits_keep_every_guarantee_over_many_seeds() {
    let [succeeded, refused, missed] = editor_walk::walk(60_000, 12);
    eprintln!("succeeded {succeeded}, refused {refused}, no target {missed}");
    assert!(succeeded > 0);
    assert!(refused > 0);
}

/// Every read through every kind of handle agrees with the file: lookups by key find the first
/// of duplicate keys, in the preamble, in duplicate sections and in a nameless one.
#[test]
fn every_read_agrees_with_the_file() {
    let source = "a=1\nb=2\na=3\n[s]\nk=1\n; c\nk=2\n[s]\nk=3\nk=4\n[]\nx=1\n";
    let edit_options = EditOptions {
        separator_spacing: SeparatorSpacing::Preserve,
    };
    let mut editor = Editor::new(source, &options(false, false), &edit_options).unwrap();
    let targets = [
        editor_ops::Target::Preamble,
        editor_ops::Target::Name("s".to_owned()),
        editor_ops::Target::Index(1),
        editor_ops::Target::Index(2),
    ];
    for target in &targets {
        for index in 0..4 {
            let read = editor_ops::Operation::Read { index };
            let outcome = editor_ops::step(&mut editor, target, &read);
            assert!(
                matches!(outcome, Ok(editor_ops::Value::Reads(_))),
                "{target:?} {index}: {outcome:?}"
            );
        }
    }
    assert_eq!(editor.finish(), source);
}

/// Longer edit sequences, hostile keys and values among them, checked against a model that
/// tracks entry identity and the body lines of every container. Bit 3 of the first byte starts
/// half of the sequences with mixed line endings, where removals merge an LF blank line into a CR
/// line break (rule D).
#[test]
fn duplicate_keys_and_refused_input_follow_the_identity_model() {
    for flags in 0u8..16 {
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

/// Generated documents have independent semantic and exact-byte expectations.
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

/// The grammar that the editor module documents for keys, values, section names and separator
/// spacing, written as plain predicates and compared with what the editor accepts. Each predicate
/// states one sentence of the module documentation, and a test requires that sentence to be
/// there, so the documentation and the behavior cannot drift apart.
mod grammar {
    use ini_edit::ParseOptions;
    use ini_edit::editor::{
        EditError, EditOptions, Editor, SectionEditor, SeparatorSpacing, Whitespace,
    };

    use crate::editor_inputs;

    /// The sentences of "What the editor refuses" in `src/editor/mod.rs`.
    const KEYS: &str = "**Keys**: not empty; no `=`, `:`, space, tab, CR or LF; not starting \
        with `;`, `#` or `[`.";
    const VALUES: &str = "**Values**: a value does not start or end with a space or tab; every \
        line break directly follows a backslash, with only spaces and tabs between them; the last \
        line does not end in a backslash; with [`inline_comments`](ParseOptions::inline_comments), \
        its last line has no `;` or `#` directly after a run of spaces or tabs that follows other \
        text on that line; it may be empty.";
    const SECTION_NAMES: &str = "**Section names**: not empty; no `]`, CR or LF; not starting or \
        ending with a space or tab.";
    const SPACING: &str = "**Separator spacing**: only spaces and tabs ([`Whitespace::new`]).";

    /// Where the editor decides: the middle of a document, after a terminated line, with an entry
    /// to assign, so that no rule about the document's context applies.
    const DOCUMENT: &str = "[s]\nk=1\n";

    fn is_spacing(character: char) -> bool {
        matches!(character, ' ' | '\t')
    }

    /// Whether [`KEYS`] allows `key`.
    fn key_is_valid(key: &str) -> bool {
        !key.is_empty()
            && !key.contains(['=', ':', ' ', '\t', '\r', '\n'])
            && !key.starts_with([';', '#', '['])
    }

    /// The lines of `text`, split at every line break: CRLF, CR or LF.
    fn lines(text: &str) -> Vec<&str> {
        let mut lines = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find(['\r', '\n']) {
            lines.push(&rest[..start]);
            let width = if rest[start..].starts_with("\r\n") {
                2
            } else {
                1
            };
            rest = &rest[start + width..];
        }
        lines.push(rest);
        lines
    }

    /// Whether `line` has a `;` or `#` directly after a run of spaces or tabs that follows other
    /// text on the line.
    fn has_marker_after_spacing(line: &str) -> bool {
        line.char_indices().any(|(index, character)| {
            let before = &line[..index];
            let text = before.trim_end_matches(is_spacing);
            matches!(character, ';' | '#') && text.len() < before.len() && !text.is_empty()
        })
    }

    /// Whether [`VALUES`] allows `value`, with or without inline comments.
    fn value_is_valid(value: &str, inline_comments: bool) -> bool {
        let lines = lines(value);
        let (last, continued) = lines.split_last().expect("text has a line");
        !value.starts_with(is_spacing)
            && !value.ends_with(is_spacing)
            && continued
                .iter()
                .all(|line| line.trim_end_matches(is_spacing).ends_with('\\'))
            && !last.ends_with('\\')
            && (!inline_comments || !has_marker_after_spacing(last))
    }

    /// Whether [`SECTION_NAMES`] allows `name`.
    fn section_name_is_valid(name: &str) -> bool {
        !name.is_empty()
            && !name.contains([']', '\r', '\n'])
            && !name.starts_with(is_spacing)
            && !name.ends_with(is_spacing)
    }

    /// Whether [`SPACING`] allows `text` on either side of the separator.
    fn spacing_is_valid(text: &str) -> bool {
        text.chars().all(is_spacing)
    }

    fn section(editor: &mut Editor) -> SectionEditor<'_> {
        editor.section("s").expect("the document has [s]")
    }

    /// Whether `edit` succeeds on [`DOCUMENT`]. A refusal must be `expected` and change nothing.
    fn accepts(
        options: &ParseOptions,
        edit: impl FnOnce(&mut Editor) -> Result<(), EditError>,
        expected: &EditError,
    ) -> bool {
        let edit_options = EditOptions {
            separator_spacing: SeparatorSpacing::Preserve,
        };
        let mut editor = Editor::new(DOCUMENT, options, &edit_options).expect("a small document");
        match edit(&mut editor) {
            Ok(()) => true,
            Err(error) => {
                assert_eq!(&error, expected);
                assert_eq!(editor.finish(), DOCUMENT);
                false
            }
        }
    }

    /// The documented decision and the editor's for `text` at each site, under `options`.
    fn decisions(text: &str, options: &ParseOptions) -> [(&'static str, bool, bool); 4] {
        let key = EditError::InvalidKey { key: text.into() };
        let value = EditError::InvalidValue { value: text.into() };
        let name = EditError::InvalidSectionName { name: text.into() };
        let valid_value = value_is_valid(text, options.inline_comments);
        [
            (
                "append_entry key",
                key_is_valid(text),
                accepts(
                    options,
                    |editor| section(editor).append_entry(text, "v"),
                    &key,
                ),
            ),
            (
                "append_entry value",
                valid_value,
                accepts(
                    options,
                    |editor| section(editor).append_entry("n", text),
                    &value,
                ),
            ),
            (
                "set value",
                valid_value,
                accepts(options, |editor| section(editor).set("k", text), &value),
            ),
            (
                "section_or_insert name",
                section_name_is_valid(text),
                accepts(
                    options,
                    |editor| editor.section_or_insert(text).map(drop),
                    &name,
                ),
            ),
        ]
    }

    /// Compare the documented and the actual decision for every string of up to `count` of the
    /// hostile pieces and `λ`, under every option set, and return the mismatches. Every site
    /// both accepts and refuses something, so the comparison is not vacuous.
    fn mismatches(count: usize) -> usize {
        let mut pieces = editor_inputs::PIECES.to_vec();
        pieces.push("λ");
        let mut mismatches = 0;
        let mut outcomes = [[false; 2]; 5];
        for text in editor_inputs::strings(&pieces, count) {
            for flags in 0..4 {
                let options = ParseOptions {
                    allow_no_value: flags & 1 != 0,
                    inline_comments: flags & 2 != 0,
                };
                for (site, (name, documented, accepted)) in
                    decisions(&text, &options).into_iter().enumerate()
                {
                    outcomes[site][usize::from(accepted)] = true;
                    if documented != accepted {
                        mismatches += 1;
                        eprintln!("{name} {text:?}, {options:?}: documented {documented}");
                    }
                }
            }
            // Spacing is validated without a document or parser options.
            let accepted = match Whitespace::new(&text) {
                Ok(spacing) => {
                    assert_eq!(spacing.as_str(), text);
                    true
                }
                Err(error) => {
                    assert_eq!(error, EditError::InvalidSpacing { text: text.clone() });
                    false
                }
            };
            outcomes[4][usize::from(accepted)] = true;
            if spacing_is_valid(&text) != accepted {
                mismatches += 1;
                eprintln!("Whitespace::new {text:?}: documented {}", !accepted);
            }
        }
        assert_eq!(outcomes, [[true; 2]; 5]);
        mismatches
    }

    /// The comparison words, ignoring how the documentation wraps its lines.
    fn words(text: &str) -> Vec<&str> {
        text.split_whitespace().collect()
    }

    #[test]
    fn the_module_documentation_states_the_tested_grammar() {
        let source = include_str!("../src/editor/mod.rs");
        let documentation: Vec<&str> = source
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("//!"))
            .flat_map(words)
            .collect();
        for sentence in [KEYS, VALUES, SECTION_NAMES, SPACING] {
            let sentence = words(sentence);
            assert!(
                documentation
                    .windows(sentence.len())
                    .any(|window| window == sentence.as_slice()),
                "src/editor/mod.rs no longer says {sentence:?}; update the predicates with it"
            );
        }
    }

    /// Every string of up to two pieces.
    #[test]
    fn the_editor_accepts_exactly_the_documented_grammar() {
        assert_eq!(mismatches(2), 0);
    }

    #[test]
    #[ignore = "extended; run in the extended-tests CI job with --profile release-checked"]
    fn the_editor_accepts_exactly_the_documented_grammar_for_three_pieces() {
        assert_eq!(mismatches(3), 0);
    }
}

/// Every committed fuzz input, run through the oracle of its target.
mod replay {
    use std::fs;
    use std::panic;
    use std::path::Path;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::thread;
    use std::time::Duration;

    /// Committed inputs, relative to the crate root.
    const DIRECTORY: &str = "tests/fuzz_regressions";
    /// A slower input fails, as it does when CI fuzzes with `-timeout=10`.
    const INPUT_SECONDS_MAX: u64 = 10;
    /// Stop after this many failing inputs; each one prints its panic message.
    const FAILURES_MAX: usize = 8;

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    /// A worker thread checks the inputs in order and reports progress after
    /// each one, so that this thread can fail a hang and name its input
    /// instead of blocking. A hung worker ends with the test process.
    fn replay(target: &str, check: fn(&[u8])) {
        let directory = root().join(DIRECTORY).join(target);
        let mut names: Vec<String> = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
            .map(|entry| {
                let name = entry.unwrap().file_name().into_string().unwrap();
                format!("{DIRECTORY}/{target}/{name}")
            })
            .collect();
        names.sort();
        assert!(!names.is_empty(), "{}: no inputs", directory.display());
        let inputs: Vec<(String, Vec<u8>)> = names
            .iter()
            .map(|name| {
                let data =
                    fs::read(root().join(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
                (name.clone(), data)
            })
            .collect();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(format!("replay::{target}"))
            .spawn(move || {
                let mut failures = Vec::new();
                for (name, data) in inputs {
                    if panic::catch_unwind(|| check(&data)).is_err() {
                        // Printed right after the panic message it explains.
                        eprintln!("failing input: {name}");
                        failures.push(name);
                    }
                    // Sending fails only after this input timed out.
                    if sender.send(()).is_err() || failures.len() == FAILURES_MAX {
                        break;
                    }
                }
                failures
            })
            .expect("spawn the replay thread");
        for name in &names {
            match receiver.recv_timeout(Duration::from_secs(INPUT_SECONDS_MAX)) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => panic!(
                    "{name} ran longer than {INPUT_SECONDS_MAX} s; reproduce it with \
                     `cargo +nightly fuzz run {target} {name} -- -timeout={INPUT_SECONDS_MAX}`"
                ),
                // The worker stopped after FAILURES_MAX failures.
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let failures = worker
            .join()
            .expect("the replay thread catches every panic");
        assert!(
            failures.is_empty(),
            "{target}: {} failing input(s) (the replay stops after {FAILURES_MAX}); \
             reproduce one with `cargo +nightly fuzz run {target} <input>`: {failures:#?}",
            failures.len()
        );
    }

    /// One test per fuzz target, so targets replay in parallel and fail separately.
    macro_rules! replay_tests {
        ($($target:ident => $check:path),* $(,)?) => {
            const TARGETS: &[&str] = &[$(stringify!($target)),*];
            $(
                #[test]
                #[cfg_attr(miri, ignore = "reads files and replays hundreds of inputs")]
                fn $target() {
                    replay(stringify!($target), $check);
                }
            )*
        };
    }

    replay_tests! {
        editor => crate::editor_ops::check,
        editor_reparse => crate::editor_reparse::check,
        editor_stateful => crate::editor_stateful::check,
        generated_edits => crate::generated_edits::check,
        inputs => crate::editor_inputs::check,
        round_trip => crate::round_trip::check,
    }

    /// A new fuzz target must come with committed inputs and a replay test, and
    /// every input directory must belong to a target.
    #[test]
    #[cfg_attr(miri, ignore = "reads files")]
    fn every_fuzz_target_is_replayed() {
        let manifest = root().join("fuzz/Cargo.toml");
        let manifest = fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("{}: {error}", manifest.display()));
        let mut declared = Vec::new();
        let mut in_binary = false;
        for line in manifest.lines() {
            if line.starts_with('[') {
                in_binary = line == "[[bin]]";
            } else if let Some(name) = line.strip_prefix("name = \"") {
                if in_binary {
                    declared.push(name.strip_suffix('"').unwrap().to_owned());
                }
            }
        }
        declared.sort();
        let mut committed: Vec<String> = fs::read_dir(root().join(DIRECTORY))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        committed.sort();
        let mut replayed: Vec<String> = TARGETS.iter().map(|&name| name.to_owned()).collect();
        replayed.sort();
        assert_eq!(
            replayed, declared,
            "replay_tests! must list exactly the [[bin]] targets of fuzz/Cargo.toml"
        );
        assert_eq!(
            committed, declared,
            "tests/fuzz_regressions/ must have one directory per fuzz target"
        );
    }
}
