//! The oracles behind the fuzz targets, run by ordinary `cargo test`, and the editor's other
//! oracles.
//!
//! Every fuzz target is a thin wrapper around a `check(&[u8])` oracle in
//! `tests/support/`. These tests run each oracle over a fixed sweep and over
//! every committed fuzz input, like SQLite's fuzzcheck.
//! `tests/fuzz_regressions/<target>/` holds the `cargo fuzz cmin` corpus and
//! every crash or timeout reproducer, so past findings are checked on every
//! platform and toolchain. The hostile-input oracle and the random walk check
//! the editor's guarantees over generated input.
//!
//! Four extended tests are `#[ignore]`d: the reload triples, the full two- and
//! three-piece hostile products, and a 60,000-seed walk. No CI job runs them
//! yet; until one does, run them by hand, which takes under two minutes:
//! `cargo test --release --test fuzz_regressions -- --ignored`.
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
#[ignore = "extended; run by hand: cargo test --release --test fuzz_regressions -- --ignored"]
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
    let counts = editor_inputs::run(
        2,
        &pieces,
        &documents,
        &options,
        &[SeparatorSpacing::Preserve],
    );
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
    let counts = editor_inputs::run(
        count,
        &editor_inputs::PIECES,
        &editor_inputs::DOCUMENTS,
        &options,
        &spacings,
    );
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
#[ignore = "extended; run by hand: cargo test --release --test fuzz_regressions -- --ignored"]
fn hostile_input_products_of_two_pieces() {
    hostile_products(2);
}

#[test]
#[ignore = "extended; run by hand: cargo test --release --test fuzz_regressions -- --ignored"]
fn hostile_input_products_of_three_pieces() {
    hostile_products(3);
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
#[ignore = "extended; run by hand: cargo test --release --test fuzz_regressions -- --ignored"]
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
/// tracks entry identity.
#[test]
fn duplicate_keys_and_refused_input_follow_the_identity_model() {
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
