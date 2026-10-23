//! Editing with and without a save and reload between steps must not differ: every operation
//! runs on the live editor and on one reopened from the text before it, through
//! [`editor_ops::step`](crate::editor_ops::step), which compares their results, texts and trees
//! and checks the editor's guarantees. Documents may be malformed: the tree must equal a parse of
//! the text (C1) whatever the text is.

use ini_edit::ParseOptions;
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};

use crate::editor_ops::{self, Operation, Target};

pub const SOURCES: [&str; 12] = [
    "",
    "[a]",
    "[a]\nx=0",
    "[a]\nx=0\n",
    "[a]\n; tail",
    "\u{FEFF}[a]\r\nx=0\r\n",
    "[a]\rx=0\r",
    // A CR document with an LF blank line, which rule D concerns.
    "[a]\ra=1\n\n[b]\ny=2\n",
    "g=1\n[a]\nx=0\n",
    // A continuing last line, which rule C concerns.
    "[a]\nx=C:\\d\\",
    "[a]\n=bad\nkey junk\n",
    "\u{FEFF}k=1\n",
];
/// Number of distinct operations; an operation byte selects one modulo this.
pub const OPERATION_KINDS: u8 = 25;
/// Later operations are ignored, which bounds the work per input.
const OPERATIONS_MAX: usize = 64;

fn name(name: &str) -> Target {
    Target::Name(name.to_owned())
}

fn lines(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).to_owned()).collect()
}

fn text(text: &str) -> String {
    text.to_owned()
}

/// Create section `section` with `section_or_insert`, as 0.4's lookups did, then run `operation`
/// on it.
fn in_section(section: &str, operation: Operation) -> Vec<(Target, Operation)> {
    vec![
        (name(section), Operation::SectionOrInsert),
        (name(section), operation),
    ]
}

/// One operation on the preamble.
fn in_preamble(operation: Operation) -> Vec<(Target, Operation)> {
    vec![(Target::Preamble, operation)]
}

/// Raw lines appended to section `a`.
fn append(raw: &[&str]) -> Vec<(Target, Operation)> {
    in_section("a", Operation::AppendBodyLines { lines: lines(raw) })
}

/// The steps of operation `kind`. The 0.4 operations that created their section first do so
/// with `section_or_insert`.
fn steps(kind: u8) -> Vec<(Target, Operation)> {
    match kind % OPERATION_KINDS {
        kind @ (0..=6 | 23 | 24) => entry_steps(kind),
        kind @ 7..=14 => raw_line_steps(kind),
        kind @ 15..=17 => preamble_steps(kind),
        kind => refused_steps(kind),
    }
}

/// Entries and removals in sections `a` and `b`.
fn entry_steps(kind: u8) -> Vec<(Target, Operation)> {
    match kind {
        0 => in_section(
            "a",
            Operation::AppendEntry {
                key: text("x"),
                value: text("1"),
            },
        ),
        1 => in_section(
            "a",
            Operation::InsertEntry {
                at: 0,
                key: text("y"),
                value: text("2"),
            },
        ),
        2 => in_section("a", Operation::RemoveBodyLines { start: 0, end: 1 }),
        3 => in_section(
            "b",
            Operation::Set {
                key: text("z"),
                value: text("3"),
            },
        ),
        // 0.4 created a missing section to remove it, which left bytes behind.
        4 => vec![(name("a"), Operation::RemoveContainer)],
        5 => in_section("b", Operation::ClearBody),
        6 => in_section(
            "a",
            Operation::Set {
                key: text("x"),
                value: text(""),
            },
        ),
        23 => in_section("a", Operation::RemoveEntry { key: text("x") }),
        24 => in_section(
            "a",
            Operation::SetKeyAt {
                index: 0,
                key: text("k2"),
            },
        ),
        _ => unreachable!("not an entry operation: {kind}"),
    }
}

/// Raw lines at the end of section `a`, some refused.
fn raw_line_steps(kind: u8) -> Vec<(Target, Operation)> {
    match kind {
        7 => append(&["; tail"]),
        // An element with its own terminator is refused.
        8 => append(&["; tail\r"]),
        9 => append(&["; tail", ""]),
        10 => append(&["k=2"]),
        11 => append(&["=bad"]),
        12 => append(&["  "]),
        13 => append(&["\u{FEFF}x"]),
        14 => in_section(
            "a",
            Operation::InsertBodyLinesAtEnd {
                lines: lines(&["; end"]),
            },
        ),
        _ => unreachable!("not a raw line operation: {kind}"),
    }
}

/// Edits of the preamble, the lines above the first header.
fn preamble_steps(kind: u8) -> Vec<(Target, Operation)> {
    match kind {
        15 => in_preamble(Operation::InsertBodyLinesAtEnd {
            lines: lines(&["; above"]),
        }),
        16 => in_preamble(Operation::Set {
            key: text("p"),
            value: text("1"),
        }),
        17 => in_preamble(Operation::RemoveBodyLines { start: 0, end: 1 }),
        _ => unreachable!("not a preamble operation: {kind}"),
    }
}

/// Edits that the editor refuses: values, a section name, a position and a raw header.
fn refused_steps(kind: u8) -> Vec<(Target, Operation)> {
    match kind {
        18 => in_section(
            "a",
            Operation::Set {
                key: text("x"),
                value: text("C:\\d\\"),
            },
        ),
        19 => in_section(
            "a",
            Operation::Set {
                key: text("x"),
                value: text(" v"),
            },
        ),
        20 => vec![(name(""), Operation::SectionOrInsert)],
        21 => in_section(
            "a",
            Operation::InsertBodyLines {
                at: 99,
                lines: lines(&["; x"]),
            },
        ),
        22 => append(&["[x]"]),
        _ => unreachable!("not a refused operation: {kind}"),
    }
}

/// The first byte selects one of [`SOURCES`]; each later byte is an operation.
pub fn check(data: &[u8]) {
    let Some((&source, operations)) = data.split_first() else {
        return;
    };
    let source = SOURCES[usize::from(source) % SOURCES.len()];
    let parse_options = ParseOptions {
        allow_no_value: false,
        inline_comments: false,
    };
    let edit_options = EditOptions {
        separator_spacing: SeparatorSpacing::Preserve,
    };
    let mut live = Editor::new(source, &parse_options, &edit_options).expect("a small source");
    editor_ops::assert_agrees_with_reparse(&live, &source);
    for &operation in &operations[..operations.len().min(OPERATIONS_MAX)] {
        for (target, step) in steps(operation) {
            // The step checks every guarantee itself; its outcome only reports what happened.
            let _ = editor_ops::step(&mut live, &target, &step);
        }
    }
}
