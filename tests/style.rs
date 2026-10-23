//! `TigerStyle`'s layout limits, checked by every `cargo test`: no line is wider than 100 columns
//! and no function is longer than 70 lines.
//!
//! rustfmt wraps code at 100 columns, but not string literals, comments or prose, and clippy's
//! `too_many_lines` counts only the lines of a body that hold code; these checks measure what a
//! reader sees. They read only the crate's own paths and skip those that are absent, so they also
//! pass on the published crate, which leaves out CI, the fuzz workspace and the other
//! repository-only files.

use std::fs;
use std::path::Path;

/// The widest line, in characters.
const COLUMNS_MAX: usize = 100;

/// The longest function, in lines from its `fn` line through its closing brace.
const FUNCTION_LINES_MAX: usize = 70;

/// The extensions of the checked files.
const EXTENSIONS: [&str; 5] = ["md", "rs", "toml", "yaml", "yml"];

/// The checked paths, relative to the crate root: each directory with everything under it, and
/// each file. Whatever else is under the root, such as a vendored dependency, a nested clone, a
/// build directory or scratch notes, is not the crate's and is not read, so a new top-level file
/// must be listed here. `CHANGELOG.md` is left out: release-plz writes it from commit messages.
const ROOTS: [&str; 14] = [
    ".github",
    ".release-plz.toml",
    "Cargo.toml",
    "README.md",
    "TESTING.md",
    "benches",
    "clippy.toml",
    "codecov.yml",
    "deny.toml",
    "examples",
    "fuzz/Cargo.toml",
    "fuzz/fuzz_targets",
    "src",
    "tests",
];

/// Paths under [`ROOTS`] that are not checked, with everything under them: inputs and expected
/// outputs, whose lines are as long as their cases need.
const SKIPPED_PATHS: [&str; 3] = [
    "tests/fixtures",
    "tests/fuzz_regressions",
    "tests/snapshots",
];

/// The most directories that the walk reads, several times the crate's: a tree that outgrows it
/// fails here instead of slowing down every `cargo test`.
const DIRECTORIES_MAX: usize = 64;

/// The most directory entries that the walk reads, for the same reason.
const ENTRIES_MAX: usize = 512;

/// Functions longer than [`FUNCTION_LINES_MAX`] in files that another change is rewriting, as
/// `(path, name)`. Each one must still be too long: delete its entry once it fits.
const LONG_FUNCTIONS: [(&str, &str); 0] = [];

/// The words that rustfmt may put before `fn` on a function's first line, in the order that Rust
/// requires: one visibility, then `const`, `async` and `unsafe`.
const QUALIFIERS: [&str; 6] = [
    "pub ",
    "pub(crate) ",
    "pub(super) ",
    "const ",
    "async ",
    "unsafe ",
];

/// cargo-mutants appends this comment to the code it mutates, which can push a line past the
/// limit. Nobody reads a mutated line, and measuring it would count the mutant as caught by this
/// test rather than by a test of its behavior.
const MUTANTS_MARKER: &str = "/* ~ changed by cargo-mutants ~ */";

/// Functions as rustfmt lays them out, for the scanner's self-test. The text is outside the test,
/// whose length would otherwise end at the text's first closing brace (see [`function_length`]).
const RUSTFMT_LAYOUT: &str = "\
fn one_line() -> u8 { 1 }
pub(crate) fn multi_line_signature(
    first: u8,
) -> u8 {
    first
}
trait Declarations {
    fn declared(&self);
    fn declared_over_lines(
        &self,
    ) -> u8;
    const fn with_default(&self) -> u8 {
        fn nested() {}
        2
    }
}
fn bounded<T>(value: T) -> T
where
    T: Copy,
{
    value
}
pub(super) const unsafe fn qualified() -> u8 {
    3
}
fuzz_target!(|data: &[u8]| {
    run(data);
});
";

/// A function found in a source file.
#[derive(Debug, PartialEq, Eq)]
struct Function<'text> {
    /// Its name, or `fuzz_target!` for a fuzz target's body.
    name: &'text str,
    /// The 1-based number of its first line.
    line: usize,
    /// Its length in lines.
    lines: usize,
}

#[test]
fn lines_fit_in_100_columns() {
    let files = checked_files();
    let mut long = Vec::new();
    for path in &files {
        let text = read(path);
        for (line, columns) in long_lines(extension(path), &text) {
            long.push(format!("{path}:{line}: {columns} columns"));
        }
    }
    assert!(
        long.is_empty(),
        "wrap these lines to at most {COLUMNS_MAX} columns:\n{}",
        long.join("\n")
    );
}

#[test]
fn functions_fit_in_70_lines() {
    let files = checked_files();
    let mut long = Vec::new();
    let mut exempt = Vec::new();
    for path in files.iter().filter(|path| extension(path) == "rs") {
        let text = read(path);
        for function in functions(&text) {
            if function.lines <= FUNCTION_LINES_MAX {
                continue;
            }
            let name = (path.as_str(), function.name);
            if let Some(entry) = LONG_FUNCTIONS.iter().find(|entry| **entry == name) {
                exempt.push(*entry);
            } else {
                let Function { name, line, lines } = function;
                long.push(format!("{path}:{line}: {name} has {lines} lines"));
            }
        }
    }
    assert!(
        long.is_empty(),
        "split these functions to at most {FUNCTION_LINES_MAX} lines:\n{}",
        long.join("\n")
    );
    for (path, name) in LONG_FUNCTIONS {
        if files.iter().any(|file| file == path) {
            assert!(
                exempt.contains(&(path, name)),
                "{path}: {name} fits now, or is gone; delete it from LONG_FUNCTIONS"
            );
        }
    }
}

#[test]
fn walk_reads_only_the_listed_paths() {
    // In cargo's directory for the files of integration tests, one per process, so that runs in
    // parallel, debug and release for example, do not share it.
    let directory = format!("style-walk-{}", std::process::id());
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(directory);
    let checked = [
        ".github/workflows/ci.yml",
        "Cargo.toml",
        "README.md",
        "fuzz/fuzz_targets/editor.rs",
        "src/editor/mod.rs",
        "tests/support/round_trip.rs",
    ];
    let unchecked = [
        // Not the crate's: release-plz's changelog, scratch notes, a vendored dependency, a
        // nested clone and a build directory.
        "CHANGELOG.md",
        "NOTES.md",
        "vendor/dependency/.github/workflows/ci.yml",
        "vendor/dependency/src/lib.rs",
        "clone/Cargo.toml",
        "clone/src/lib.rs",
        "target/debug/build/output.rs",
        // Under a listed directory, but skipped or of another type.
        "tests/fixtures/long.md",
        "tests/snapshots/case.snap",
        ".github/scripts/check.py",
    ];
    for path in checked.iter().chain(&unchecked) {
        let path = root.join(path);
        let parent = path.parent().expect("a file has a directory");
        fs::create_dir_all(parent).expect("the test's directory is writable");
        fs::write(&path, "").expect("the test's directory is writable");
    }
    let found = checked_files_under(&root);
    // The published crate's manifest is generated; the original is checked in the repository.
    fs::write(root.join("Cargo.toml.orig"), "").expect("the test's directory is writable");
    let published = checked_files_under(&root);
    fs::remove_dir_all(&root).expect("the test's directory is removable");
    // Sorted, and without the listed paths that are absent, such as TESTING.md and benches.
    assert_eq!(found, checked);
    let original: Vec<&str> = checked
        .into_iter()
        .filter(|path| *path != "Cargo.toml")
        .collect();
    assert_eq!(published, original);
}

#[test]
fn line_check_skips_tables_links_and_mutated_lines() {
    let wide = "x".repeat(COLUMNS_MAX + 1);
    let fits = "é".repeat(COLUMNS_MAX);
    let text = format!(
        "{fits}\n{wide}\n| {wide} |\n[link]: https://{wide}\n{wide} {MUTANTS_MARKER}\n{wide}\r\n"
    );
    // Columns are characters, not bytes, and CRLF line breaks do not count.
    assert_eq!(long_lines("md", &text), [(2, 101), (6, 101)]);
    // Table rows and link definitions are Markdown's: elsewhere they are measured.
    assert_eq!(
        long_lines("rs", &text),
        [(2, 101), (3, 105), (4, 117), (6, 101)]
    );
}

#[test]
fn function_scanner_follows_rustfmt_layout() {
    let found: Vec<(&str, usize, usize)> = functions(RUSTFMT_LAYOUT)
        .into_iter()
        .map(|function| (function.name, function.line, function.lines))
        .collect();
    assert_eq!(
        found,
        [
            ("one_line", 1, 1),
            ("multi_line_signature", 2, 5),
            ("with_default", 12, 4),
            ("nested", 13, 1),
            ("bounded", 17, 6),
            ("qualified", 23, 3),
            ("fuzz_target!", 26, 3),
        ]
    );
}

/// Every checked file of the crate, as a path relative to its root with `/` separators, sorted.
fn checked_files() -> Vec<String> {
    let files = checked_files_under(Path::new(env!("CARGO_MANIFEST_DIR")));
    // The walk reached the sources, the tests and the documentation.
    for expected in ["README.md", "src/lib.rs", "tests/style.rs"] {
        assert!(files.iter().any(|file| file == expected), "{expected}");
    }
    files
}

/// Every checked file under [`ROOTS`] in `root`, as a path relative to `root` with `/`
/// separators, sorted. A listed path that is absent is skipped.
fn checked_files_under(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut pending = Vec::new();
    for path in ROOTS {
        if root.join(path).is_dir() {
            pending.push(path.to_owned());
        } else if root.join(path).is_file() && is_checked_file(root, path) {
            files.push(path.to_owned());
        }
    }
    // A work list instead of recursion, with both of its loops bounded.
    let mut directories = 0;
    let mut entries = 0;
    while let Some(directory) = pending.pop() {
        directories += 1;
        assert!(
            directories <= DIRECTORIES_MAX,
            "over {DIRECTORIES_MAX} directories"
        );
        let listing = fs::read_dir(root.join(&directory)).expect("a found directory is readable");
        for entry in listing {
            entries += 1;
            assert!(
                entries <= ENTRIES_MAX,
                "over {ENTRIES_MAX} directory entries"
            );
            let entry = entry.expect("a directory entry is readable");
            let path = format!("{directory}/{}", entry.file_name().to_string_lossy());
            let file_type = entry.file_type().expect("a directory entry has a type");
            if SKIPPED_PATHS.contains(&path.as_str()) {
                continue;
            }
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && is_checked_file(root, &path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Whether the file at `path` under `root` is checked: it has a checked extension and is not the
/// manifest that `cargo package` generates, on one line per value, next to the original
/// `Cargo.toml.orig`.
fn is_checked_file(root: &Path, path: &str) -> bool {
    let generated = path == "Cargo.toml" && root.join("Cargo.toml.orig").is_file();
    EXTENSIONS.contains(&extension(path)) && !generated
}

/// The text after the last dot of the file name in `path`, or `""`.
fn extension(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or("", |(_, extension)| extension)
}

/// The text of the file at `path`, relative to the crate root.
fn read(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// The 1-based number and the width in characters of every line of `text` wider than
/// [`COLUMNS_MAX`] that the limit covers, for a file with `extension`.
fn long_lines(extension: &str, text: &str) -> Vec<(usize, usize)> {
    let mut long = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let columns = line.chars().count();
        if columns > COLUMNS_MAX && is_measured(extension, line) {
            long.push((index + 1, columns));
        }
    }
    long
}

/// Whether the limit covers `line` of a file with `extension`. It covers every line except a
/// mutated one and, in Markdown, a table row, which cannot wrap, and a link definition, which
/// holds one URL.
fn is_measured(extension: &str, line: &str) -> bool {
    let trimmed = line.trim_start();
    let table_row = trimmed.starts_with('|');
    let link_definition = trimmed.starts_with('[') && trimmed.contains("]: ");
    let markdown_only = extension == "md" && (table_row || link_definition);
    !line.contains(MUTANTS_MARKER) && !markdown_only
}

/// The functions with a body in `text`, with their lengths, in order.
fn functions(text: &str) -> Vec<Function<'_>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(name) = function_name(line) else {
            continue;
        };
        if let Some(length) = function_length(&lines[index..]) {
            found.push(Function {
                name,
                line: index + 1,
                lines: length,
            });
        }
    }
    found
}

/// The name of the function that starts on `line`, if one does.
fn function_name(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    if rest.starts_with("fuzz_target!(") {
        return Some("fuzz_target!");
    }
    // One pass in Rust's order, which strips each qualifier that a function can have.
    for qualifier in QUALIFIERS {
        rest = rest.strip_prefix(qualifier).unwrap_or(rest);
    }
    let rest = rest.strip_prefix("fn ")?;
    let end = rest
        .find(|character: char| !(character.is_alphanumeric() || character == '_'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// The length in lines of the function whose first line is `lines[0]`, or `None` for a
/// declaration without a body. rustfmt ends a signature that opens a body with `{`, and puts the
/// body's closing brace first on a line at the indentation of the function's first line.
///
/// The scan does not lex Rust, so a line of a multi-line string literal that starts with `}` at
/// that indentation ends the function early, and the function is measured short. Clippy's
/// `too_many_lines` still bounds its lines of code.
fn function_length(lines: &[&str]) -> Option<usize> {
    let first_indentation = indentation(lines[0]);
    if lines[0].trim_end().ends_with('}') {
        return Some(1);
    }
    let mut opened = false;
    for (offset, line) in lines.iter().enumerate() {
        let code = line.trim_end();
        let closes = code.trim_start().starts_with('}') && indentation(line) == first_indentation;
        if opened && closes {
            return Some(offset + 1);
        }
        if !opened && code.ends_with(';') {
            return None;
        }
        opened = opened || code.ends_with('{');
    }
    panic!("no closing brace for {:?}", lines[0]);
}

/// The number of spaces that `line` starts with.
fn indentation(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}
