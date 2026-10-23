//! The oracles behind the fuzz targets, run by ordinary `cargo test`.
//!
//! Every fuzz target is a thin wrapper around a `check(&[u8])` oracle in
//! `tests/support/`. These tests run each oracle over a fixed sweep and over
//! every committed fuzz input, like SQLite's fuzzcheck.
//! `tests/fuzz_regressions/<target>/` holds the `cargo fuzz cmin` corpus and
//! every crash or timeout reproducer, so past findings are checked on every
//! platform and toolchain.
//!
//! The tests share one binary because cargo-mutants rebuilds every test
//! binary for every mutant.

#[path = "support/editor_ops.rs"]
mod editor_ops;
#[path = "support/editor_reparse.rs"]
mod editor_reparse;
#[path = "support/editor_stateful.rs"]
mod editor_stateful;
#[path = "support/generated_edits.rs"]
mod generated_edits;
#[path = "support/round_trip.rs"]
mod round_trip;

/// Metamorphic testing of editing with and without a save/reload between steps.
#[test]
fn editing_is_independent_of_save_and_reload_boundaries() {
    let sources = u8::try_from(editor_reparse::SOURCES.len()).unwrap();
    for source in 0..sources {
        for first in 0..editor_reparse::OPERATION_KINDS {
            for second in 0..editor_reparse::OPERATION_KINDS {
                for third in 0..editor_reparse::OPERATION_KINDS {
                    editor_reparse::check(&[source, first, second, third]);
                }
            }
        }
    }
}

/// Longer edit sequences checked against a model that tracks entry identity.
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
