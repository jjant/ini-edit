//! A random walk over hostile documents and operations. Every step goes through
//! [`editor_ops::step`](crate::editor_ops::step): the result, text and tree must equal those of
//! an editor reopened from the text before the step, the tree must equal a parse of the text
//! (C1), a refused step must change nothing (C2), and an insertion or a removal must change one
//! contiguous run of bytes (C3).

use ini_edit::ParseOptions;
use ini_edit::editor::{EditOptions, Editor, SeparatorSpacing};

use crate::editor_ops::{self, Operation, Target, Value};

/// A linear congruential generator, so every failing seed is reproducible.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, count: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(count).unwrap()).unwrap()
    }

    fn pick(&mut self, items: &[&str]) -> String {
        items[self.below(items.len())].to_owned()
    }
}

/// Document lines: headers, malformed headers, entries, inline comments, bare keys, `key junk`,
/// continuations (CR ones too), U+FEFF-leading lines, a final backslash, blanks and errors.
const LINES: [&str; 22] = [
    "[s]",
    "[t]",
    " [s] ; c",
    "[]",
    "[x",
    "k=v",
    "a = 1",
    "dup=1",
    "dup=2",
    "flag",
    "k = v ; c",
    "key junk",
    "=bad",
    "k=a \\",
    "  b",
    "k=c \\\r",
    "\u{FEFF}k=1",
    "; c",
    "# c",
    "",
    "  ",
    "path=C:\\d\\",
];
const ENDINGS: [&str; 4] = ["\n", "\r\n", "\r", "\n"];
const KEYS: [&str; 12] = [
    "k",
    "a",
    "dup",
    "flag",
    "n",
    "\u{FEFF}k",
    "a=b",
    "",
    " k",
    ";k",
    "[k]",
    "λ",
];
const VALUES: [&str; 14] = [
    "",
    "v",
    "1",
    "a ; b",
    "#fff",
    "x \\\n y",
    "head \\\n",
    "one \\\r",
    " v",
    "v ",
    "a\nb",
    "C:\\d\\",
    "a;b",
    "=[x]",
];
const RAW: [&str; 14] = [
    "",
    "  ",
    "k=2",
    "=bad",
    "; c",
    "\u{FEFF}x=1",
    "[h]",
    "x\r",
    "a\nb",
    "k = a \\",
    "k = a \\\n  b",
    "key junk",
    "dup=3",
    "# t",
];
const NAMES: [&str; 8] = ["s", "t", "u", "", " s", "a]b", "[x", "λ"];

fn document(random: &mut Random) -> String {
    let mut text = String::new();
    if random.below(4) == 0 {
        text.push('\u{FEFF}');
    }
    let count = random.below(7);
    for index in 0..count {
        text.push_str(&random.pick(&LINES));
        // The last line is sometimes unterminated.
        if index + 1 < count || random.below(3) != 0 {
            text.push_str(&random.pick(&ENDINGS));
        }
    }
    text
}

fn raw_lines(random: &mut Random) -> Vec<String> {
    (0..=random.below(3)).map(|_| random.pick(&RAW)).collect()
}

fn operation(random: &mut Random) -> Operation {
    match random.below(13) {
        0 => Operation::Set {
            key: random.pick(&KEYS),
            value: random.pick(&VALUES),
        },
        1 => Operation::AppendEntry {
            key: random.pick(&KEYS),
            value: random.pick(&VALUES),
        },
        2 => Operation::InsertEntry {
            at: random.below(6),
            key: random.pick(&KEYS),
            value: random.pick(&VALUES),
        },
        3 => Operation::RemoveEntry {
            key: random.pick(&KEYS),
        },
        4 => Operation::RenameKey {
            old: random.pick(&KEYS),
            new: random.pick(&KEYS),
        },
        5 => Operation::SetKeyAt {
            index: random.below(3),
            key: random.pick(&KEYS),
        },
        6 => Operation::SetValueAt {
            index: random.below(3),
            value: random.pick(&VALUES),
        },
        7 => Operation::RemoveEntryAt {
            index: random.below(3),
        },
        8 => Operation::AppendBodyLines {
            lines: raw_lines(random),
        },
        9 => Operation::InsertBodyLines {
            at: random.below(6),
            lines: raw_lines(random),
        },
        10 => {
            let start = random.below(5);
            Operation::RemoveBodyLines {
                start,
                end: start + random.below(3),
            }
        }
        11 => Operation::RemoveContainer,
        _ => Operation::SectionOrInsert,
    }
}

/// The steps of one walk step: an operation on a target, which a quarter of the time is a
/// section created first.
fn steps(random: &mut Random) -> Vec<(Target, Operation)> {
    let operation = operation(random);
    match random.below(4) {
        0 => vec![(Target::Preamble, operation)],
        1 => vec![(Target::Name(random.pick(&NAMES)), operation)],
        2 => vec![(Target::Index(random.below(3)), operation)],
        _ => {
            let name = random.pick(&NAMES);
            let create = (Target::Name(name.clone()), Operation::SectionOrInsert);
            vec![create, (Target::Name(name), operation)]
        }
    }
}

/// Walk `seeds` random documents for `steps` random operations each, and return how many calls
/// succeeded, were refused, or found no target.
pub(crate) fn walk(seeds: u64, steps: usize) -> [u64; 3] {
    let spacings = [
        SeparatorSpacing::Preserve,
        SeparatorSpacing::Compact,
        SeparatorSpacing::exact("\t", " ").expect("spaces and tabs"),
    ];
    let mut outcomes = [0; 3];
    for seed in 0..seeds {
        let mut random = Random(seed * 7919 + 17);
        let parse_options = ParseOptions {
            allow_no_value: random.below(2) == 0,
            inline_comments: random.below(2) == 0,
        };
        let edit_options = EditOptions {
            separator_spacing: spacings[random.below(3)].clone(),
        };
        let source = document(&mut random);
        let mut editor =
            Editor::new(&source, &parse_options, &edit_options).expect("a small source");
        editor_ops::assert_agrees_with_reparse(&editor, &(seed, &source));
        for _ in 0..steps {
            for (target, operation) in self::steps(&mut random) {
                let index = match editor_ops::step(&mut editor, &target, &operation) {
                    Ok(Value::NoTarget | Value::NoEntry) => 2,
                    Ok(_) => 0,
                    Err(_) => 1,
                };
                outcomes[index] += 1;
            }
        }
    }
    outcomes
}
