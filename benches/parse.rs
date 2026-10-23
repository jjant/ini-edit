//! Parse, traversal and edit benchmarks, and comparisons with `rust-ini` and `configparser`.
//!
//! The harness uses std only. `cargo bench --bench parse` times every case and prints the median
//! time of one call over [`SAMPLES`] samples, with the throughput of the case's input if it reads
//! one. Without `--bench`, as under `cargo test --all-targets`, every case runs once, which keeps
//! the benchmark compiling and running at little cost to the tests.
//!
//! The harness reads libtest's arguments. Filters, `--skip`, `--exact` and `--ignored` select
//! cases as in libtest, and `--list` lists them: `cargo bench --bench parse -- compare` times the
//! cases whose names contain `compare`, and `-- --skip malformed` every case but the malformed
//! ones. libtest's other options change nothing here, any other option is an error, as in
//! libtest, and `--help` lists the options. cargo-nextest lists the cases with `--list` and runs
//! each one with `--exact`, also once.

// TigerStyle: names are not abbreviated, here as in the library.
#![warn(clippy::min_ident_chars)]

use std::fmt::Write;
use std::hint::black_box;
use std::time::{Duration, Instant};

use ini_edit::ast::{AstNode, Entry, File};
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};
use ini_edit::{Parse, ParseOptions};

/// The samples of each case. Odd, so that the median is one of them.
const SAMPLES: usize = 51;

/// The shortest sample. The calls in a sample double until one lasts this long, so that reading
/// the clock is a small part of every sample.
const SAMPLE_TIME_MIN: Duration = Duration::from_millis(10);

/// The most times that the calls in a sample double, from one call.
const SAMPLE_DOUBLINGS_MAX: u32 = 24;

/// The most calls in a sample: enough for calls of one nanosecond to fill [`SAMPLE_TIME_MIN`],
/// and a bound on the doubling for faster ones.
const SAMPLE_CALLS_MAX: u32 = 1 << SAMPLE_DOUBLINGS_MAX;

const _: () = assert!(SAMPLES % 2 == 1);
const _: () = assert!(SAMPLE_TIME_MIN.as_nanos() <= SAMPLE_CALLS_MAX as u128);

/// The width of the case names in a report.
const NAME_COLUMNS: usize = 42;

/// libtest's options that take no value.
const FLAGS: [&str; 19] = [
    "--bench",
    "--ensure-time",
    "--exact",
    "--exclude-should-panic",
    "--fail-fast",
    "--force-run-in-process",
    "--help",
    "--ignored",
    "--include-ignored",
    "--list",
    "--no-capture",
    "--nocapture",
    "--quiet",
    "--report-time",
    "--show-output",
    "--shuffle",
    "--test",
    "-h",
    "-q",
];

/// libtest's options that take a value: the next argument, or what follows `=` in the same one,
/// as in `--skip=edit`, or for `-Z` what follows the `Z`.
const VALUE_OPTIONS: [&str; 7] = [
    "--color",
    "--format",
    "--logfile",
    "--shuffle-seed",
    "--skip",
    "--test-threads",
    "-Z",
];

/// What `--help` prints.
const USAGE: &str = "\
Usage: cargo bench --bench parse -- [OPTIONS] [FILTERS...]

Runs once every case whose name contains one of the filters, or every case if there are none.

Options:
    --bench          Time the cases instead; `cargo bench` passes it
    --exact          Match the filters and skips against whole names
    --skip FILTER    Leave out the cases that FILTER matches; can be repeated
    --list           Print the names of the cases instead
    --ignored        Select only the ignored cases, of which there are none
    -h, --help       Print this message

libtest's other options change nothing here.
";

/// The parser options of every ini-edit case, spelled out.
const PARSE_OPTIONS: ParseOptions = ParseOptions {
    allow_no_value: false,
    inline_comments: false,
};

/// The editor options of every ini-edit edit, spelled out.
const EDIT_OPTIONS: EditOptions = EditOptions {
    separator_spacing: SeparatorSpacing::Preserve,
};

const GITEA: &str = include_str!("../tests/fixtures/gitea-app.example.ini");

const SMALL: &str = "\
[server]
host = 0.0.0.0
port = 8080

[database]
url = postgres://localhost/db
timeout = 30
";

fn main() {
    let mut runner = Runner::from_args(std::env::args().skip(1)).unwrap_or_else(|message| {
        // As libtest does, with the same messages.
        eprintln!("error: {message}");
        std::process::exit(101)
    });
    runner.header();
    bench_parse(&mut runner);
    bench_malformed(&mut runner);
    bench_round_trip(&mut runner);
    bench_ast_traversal(&mut runner);
    bench_lookup(&mut runner);
    bench_edit(&mut runner);
    bench_compare_parse(&mut runner);
    bench_compare_edit(&mut runner);
    runner.summary();
}

fn bench_parse(runner: &mut Runner) {
    runner.input_case("parse/small", SMALL, parse);
    runner.input_case("parse/gitea", GITEA, parse);
}

// Error-heavy inputs must scale like valid inputs. A round-trip-only fuzzer
// rarely grows enough repeated errors to expose quadratic diagnostic work.
fn bench_malformed(runner: &mut Runner) {
    for lines in [1_000, 10_000, 100_000] {
        let source = "=bad\n".repeat(lines);
        let name = format!("malformed/error_lines/{lines}");
        runner.input_case(&name, &source, parse);
    }
}

fn bench_round_trip(runner: &mut Runner) {
    runner.input_case("round_trip/gitea", GITEA, |source| {
        parse(source).syntax().text().to_string()
    });
}

fn bench_ast_traversal(runner: &mut Runner) {
    let gitea = parse(GITEA);
    runner.case("ast_traversal/gitea_sections_and_entries", || {
        let file = File::cast(gitea.syntax()).unwrap();
        let mut key_bytes = 0;
        for section in file.sections() {
            for entry in section.entries() {
                key_bytes += entry.key().map_or(0, |key| key.len());
            }
        }
        key_bytes
    });
    runner.case("ast_traversal/gitea_every_accessor", || {
        read_every_accessor(&gitea)
    });
    let sections = parse(&document(200, 50));
    runner.case("ast_traversal/200x50_every_accessor", || {
        read_every_accessor(&sections)
    });
}

/// The bytes of every section name, key, value, inline comment and comment line of `parse`, and
/// the number of entries with a colon, all read through the AST.
fn read_every_accessor(parse: &Parse) -> usize {
    let file = File::cast(parse.syntax()).unwrap();
    let entry_bytes = |entry: Entry| {
        let text = [entry.key(), entry.value(), entry.inline_comment()];
        let bytes: usize = text.iter().flatten().map(String::len).sum();
        bytes + usize::from(entry.uses_colon())
    };
    let mut bytes: usize = file.preamble_entries().map(entry_bytes).sum();
    for section in file.sections() {
        bytes += section.name().map_or(0, |name| name.len());
        bytes += section.entries().map(entry_bytes).sum::<usize>();
        let comments = section.comment_lines().filter_map(|comment| comment.text());
        bytes += comments.map(|text| text.len()).sum::<usize>();
    }
    bytes
}

// The editor finds a section by its name and an entry by its key.
fn bench_lookup(runner: &mut Runner) {
    let source = document(200, 50);
    let mut editor = open(&source);
    let names: Vec<String> = (0..200)
        .step_by(10)
        .map(|section| format!("s{section}"))
        .collect();
    runner.case("lookup/200x50_every_tenth_section_last_key", || {
        let mut value_bytes = 0;
        for name in &names {
            let mut section = editor.section(black_box(name)).unwrap();
            let entry = section.entry(black_box("key49")).unwrap();
            value_bytes += entry.value().map_or(0, |value| value.len());
        }
        value_bytes
    });
}

fn bench_edit(runner: &mut Runner) {
    runner.case("edit/set_value", || set_port(black_box(SMALL)));
    runner.case("edit/append_entry", || {
        let mut editor = open(black_box(SMALL));
        let mut server = editor.section("server").unwrap();
        server.append_entry("timeout", "30").unwrap();
        editor.finish()
    });
    runner.case("edit/create_section", || {
        let mut editor = open(black_box(SMALL));
        let mut section = editor.section_or_insert("new_section").unwrap();
        section.set("key", "value").unwrap();
        editor.finish()
    });
}

// rust-ini and configparser must accept each input: a parser that stopped at an error would be
// timed for less work.
fn bench_compare_parse(runner: &mut Runner) {
    for (input, source) in [("small", SMALL), ("gitea", GITEA)] {
        let name = format!("compare_parse/ini_edit/{input}");
        runner.input_case(&name, source, parse);
        let name = format!("compare_parse/rust_ini/{input}");
        runner.input_case(&name, source, rust_ini_parse);
        let name = format!("compare_parse/configparser/{input}");
        runner.input_case(&name, source, |source| {
            configparser_reader().read(source.to_owned()).unwrap()
        });
    }
}

// Each crate parses the document, sets a value, and writes the document out.
fn bench_compare_edit(runner: &mut Runner) {
    runner.case("compare_edit/ini_edit/set_value", || {
        set_port(black_box(SMALL))
    });
    runner.case("compare_edit/rust_ini/set_value", || {
        let mut ini = rust_ini_parse(black_box(SMALL));
        ini.set_to(Some("server"), "port".into(), "9090".into());
        // rust-ini's defaults, spelled out.
        let options = ini::WriteOption {
            escape_policy: ini::EscapePolicy::Basics,
            line_separator: ini::LineSeparator::SystemDefault,
            kv_separator: "=",
        };
        let mut text = Vec::new();
        ini.write_to_opt(&mut text, options).unwrap();
        text
    });
    runner.case("compare_edit/configparser/set_value", || {
        let mut config = configparser_reader();
        config.read(black_box(SMALL).to_owned()).unwrap();
        config.set("server", "port", Some("9090".into()));
        // configparser's defaults, spelled out: no spaces around `=`, continuation lines indented
        // by four spaces, and no blank line between sections.
        let options = configparser::ini::WriteOptions::new_with_params(false, 4, 0);
        config.pretty_writes(&options)
    });
}

/// `sections` sections named `s0`, `s1` and so on, each with a comment line and then `entries`
/// entries keyed `key0`, `key1` and so on, and a blank line after them.
fn document(sections: usize, entries: usize) -> String {
    let mut text = String::new();
    for section in 0..sections {
        writeln!(text, "[s{section}]\n; settings of s{section}").unwrap();
        for entry in 0..entries {
            writeln!(text, "key{entry} = value {entry} ; note").unwrap();
        }
        text.push('\n');
    }
    text
}

/// ini-edit's parse of `source`.
fn parse(source: &str) -> Parse {
    ini_edit::parse_with(source, &PARSE_OPTIONS)
}

/// An editor over `source`.
fn open(source: &str) -> Editor {
    Editor::new(source, &PARSE_OPTIONS, &EDIT_OPTIONS).unwrap()
}

/// `source` with `port` in `[server]` set to 9090.
fn set_port(source: &str) -> String {
    let mut editor = open(source);
    editor
        .section("server")
        .unwrap()
        .set("port", "9090")
        .unwrap();
    editor.finish()
}

/// rust-ini's parse of `source`, with the options that the differential tests use: rust-ini's
/// defaults, spelled out.
fn rust_ini_parse(source: &str) -> ini::Ini {
    let options = ini::ParseOption {
        enabled_quote: true,
        enabled_escape: true,
        enabled_indented_mutiline_value: false,
        enabled_preserve_key_leading_whitespace: false,
    };
    ini::Ini::load_from_str_opt(source, options).unwrap()
}

/// configparser, set up as the differential tests set it up: keys keep their case, the entries
/// before the first section go to `__preamble__`, and no comment ends a value's line.
fn configparser_reader() -> configparser::ini::Ini {
    let mut config = configparser::ini::Ini::new_cs();
    config.set_default_section("__preamble__");
    config.set_inline_comment_symbols(Some(&[]));
    config
}

/// What the harness does with each case that the arguments select.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Print its name, as libtest's `--list` does.
    List,
    /// Run it once, as `cargo test` runs a benchmark.
    Smoke,
    /// Time it, for `cargo bench`, which passes `--bench`.
    Measure,
}

/// The cases that the arguments select, and what to do with them.
struct Runner {
    mode: Mode,
    /// The case filters. Without any, every case is selected; otherwise a case is selected if a
    /// filter matches it: if the filter is part of its name, or with `--exact` its whole name.
    filters: Vec<String>,
    /// The filters of `--skip`, which match as the case filters do. A case that one of them
    /// matches is not selected.
    skips: Vec<String>,
    exact: bool,
    /// `--ignored` selects only the ignored cases, and there are none.
    ignored: bool,
    /// The cases selected so far.
    selected: u32,
}

impl Runner {
    /// The runner for `args`, the process's arguments after its path, read as libtest reads
    /// them. An `Err` holds the message that libtest gives for arguments that it refuses. With
    /// `-h` or `--help`, prints [`USAGE`] and exits instead. Under `cargo bench` the arguments
    /// include `--bench`; cargo-nextest passes `--list --format terse`, with and without
    /// `--ignored`, and then `--exact NAME --nocapture` for each case.
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut runner = Self {
            mode: Mode::Smoke,
            filters: Vec::new(),
            skips: Vec::new(),
            exact: false,
            ignored: false,
            selected: 0,
        };
        let mut list = false;
        let mut help = false;
        while let Some(arg) = args.next() {
            let (option, attached) = split_value(&arg);
            let name = option.trim_start_matches('-');
            if FLAGS.contains(&option) {
                if attached.is_some() {
                    return Err(format!("Option '{name}' does not take an argument"));
                }
                match option {
                    "--bench" => runner.mode = Mode::Measure,
                    "--exact" => runner.exact = true,
                    "--ignored" => runner.ignored = true,
                    "--list" => list = true,
                    "--help" | "-h" => help = true,
                    // The others change how libtest reports, orders and runs tests, or select
                    // ignored and `should_panic` ones, of which there are none.
                    _ => {}
                }
            } else if VALUE_OPTIONS.contains(&option) {
                let value = attached.map(str::to_owned).or_else(|| args.next());
                let value = value.ok_or_else(|| format!("Argument to option '{name}' missing"))?;
                // The others change how libtest reports, orders and runs tests, or allow its
                // unstable options.
                if option == "--skip" {
                    runner.skips.push(value);
                }
            } else if arg == "--" {
                // Every argument after `--` is a filter, even one that starts with `-`.
                runner.filters.extend(args.by_ref());
            } else if arg.len() > 1 && arg.starts_with('-') {
                return Err(format!("Unrecognized option: '{name}'"));
            } else {
                runner.filters.push(arg);
            }
        }
        if help {
            print!("{USAGE}");
            std::process::exit(0);
        }
        if list {
            runner.mode = Mode::List;
        }
        Ok(runner)
    }

    /// Whether the arguments select the case `name`.
    fn selects(&self, name: &str) -> bool {
        let matches = |filter: &String| {
            if self.exact {
                name == filter
            } else {
                name.contains(filter.as_str())
            }
        };
        let filtered = self.filters.is_empty() || self.filters.iter().any(matches);
        !self.ignored && filtered && !self.skips.iter().any(matches)
    }

    /// The case `name`, which calls `routine` on `input`. Its report gives the throughput of
    /// `input`.
    fn input_case<R>(&mut self, name: &str, input: &str, mut routine: impl FnMut(&str) -> R) {
        self.run(name, Some(input.len()), || routine(black_box(input)));
    }

    /// The case `name`, which calls `routine`.
    fn case<R>(&mut self, name: &str, routine: impl FnMut() -> R) {
        self.run(name, None, routine);
    }

    /// Lists, runs or times the case `name` if the arguments select it. Its input has `bytes`
    /// bytes, if it reads one.
    fn run<R>(&mut self, name: &str, bytes: Option<usize>, mut routine: impl FnMut() -> R) {
        if !self.selects(name) {
            return;
        }
        self.selected += 1;
        match self.mode {
            Mode::List => println!("{name}: benchmark"),
            Mode::Smoke => {
                black_box(routine());
                println!("{name}: ok");
            }
            Mode::Measure => {
                let seconds = median_call_seconds(&mut routine);
                let time = time_text(seconds);
                let throughput = bytes.map_or_else(String::new, |bytes| {
                    format!("{:>10.1} MiB/s", mebibytes(bytes) / seconds)
                });
                println!("{name:<NAME_COLUMNS$}{time:>12}{throughput}");
            }
        }
    }

    /// Prints what the columns of a report hold, before the first case is timed.
    fn header(&self) {
        if self.mode == Mode::Measure {
            let samples = format!("median of {SAMPLES} samples");
            println!(
                "{samples:<NAME_COLUMNS$}{:>12}{:>16}",
                "per call", "throughput"
            );
        }
    }

    /// Prints how many cases ran, after a run that did not time them.
    fn summary(&self) {
        if self.mode == Mode::Smoke {
            let selected = self.selected;
            let cases = if selected == 1 { "case" } else { "cases" };
            println!("{selected} {cases} ran once; `cargo bench --bench parse` times them");
        }
    }
}

/// `arg` as an option and the value that it carries, if any: `--skip=edit` is `--skip` with
/// `edit`, `-Zunstable-options` is `-Z` with `unstable-options`, and `--skip` is `--skip` alone.
fn split_value(arg: &str) -> (&str, Option<&str>) {
    match (arg.split_once('='), arg.strip_prefix("-Z")) {
        (Some((option, value)), _) if option.starts_with("--") => (option, Some(value)),
        (_, Some(value)) if !value.is_empty() => ("-Z", Some(value)),
        _ => (arg, None),
    }
}

/// The median time of one call to `routine`, in seconds, over [`SAMPLES`] samples of
/// [`sample_calls`] calls each.
fn median_call_seconds<R>(routine: &mut impl FnMut() -> R) -> f64 {
    let calls = sample_calls(routine);
    let mut times: Vec<Duration> = (0..SAMPLES).map(|_| time_calls(routine, calls)).collect();
    times.sort_unstable();
    let median = times[SAMPLES / 2];
    // Each sample makes as many calls as lasted SAMPLE_TIME_MIN once, or SAMPLE_CALLS_MAX calls,
    // which take time on any clock that a benchmark can use, so the throughput is finite.
    assert!(!median.is_zero());
    median.as_secs_f64() / f64::from(calls)
}

/// The calls in each sample of `routine`: the fewest, in powers of two, that last
/// [`SAMPLE_TIME_MIN`], or else [`SAMPLE_CALLS_MAX`]. Counting them also warms up the caches and
/// the branch predictors.
fn sample_calls<R>(routine: &mut impl FnMut() -> R) -> u32 {
    let mut calls = 1;
    for _ in 0..SAMPLE_DOUBLINGS_MAX {
        if time_calls(routine, calls) >= SAMPLE_TIME_MIN {
            break;
        }
        calls *= 2;
    }
    assert!(calls.is_power_of_two());
    assert!(calls <= SAMPLE_CALLS_MAX);
    calls
}

/// How long `calls` calls to `routine` take.
fn time_calls<R>(routine: &mut impl FnMut() -> R, calls: u32) -> Duration {
    let start = Instant::now();
    for _ in 0..calls {
        black_box(routine());
    }
    start.elapsed()
}

/// `seconds` with two decimals, in the largest of seconds, milliseconds, microseconds and
/// nanoseconds that it holds one of, as `{:.2?}` prints a [`Duration`]. A [`Duration`] would
/// round a call down to whole nanoseconds.
fn time_text(seconds: f64) -> String {
    for (unit, scale) in [("s", 1.0), ("ms", 1e3), ("µs", 1e6)] {
        if seconds * scale >= 1.0 {
            return format!("{:.2}{unit}", seconds * scale);
        }
    }
    format!("{:.2}ns", seconds * 1e9)
}

/// `bytes` in MiB.
fn mebibytes(bytes: usize) -> f64 {
    let bytes = u32::try_from(bytes).expect("every input is shorter than 4 GiB");
    f64::from(bytes) / f64::from(1_u32 << 20)
}
