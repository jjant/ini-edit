//! Exhaust a bounded language, including malformed text and Unicode boundaries,
//! and every single-character corruption of the real-world fixtures.

use ini_edit::ast::{AstNode, File};
use ini_edit::{ParseError, ParseOptions, SyntaxKind, lexer, parse_with};

#[path = "support/round_trip.rs"]
mod round_trip;

fn lexer_contract(source: &str) {
    for inline_comments in [false, true] {
        let mut offset = 0;
        for token in lexer::lex_with(source, inline_comments) {
            assert!(!token.text.is_empty(), "{source:?}");
            let end = offset + token.text.len();
            assert!(end <= source.len(), "{source:?}");
            assert!(source.is_char_boundary(offset));
            assert!(source.is_char_boundary(end));
            assert_eq!(token.text, &source[offset..end]);
            assert_eq!(token.text.as_ptr(), source[offset..].as_ptr());
            if token.kind == SyntaxKind::NEWLINE {
                assert!(matches!(token.text, "\n" | "\r" | "\r\n"));
            }
            offset = end;
        }
        assert_eq!(offset, source.len(), "{source:?}");
    }
}

fn diagnostic_contract(source: &str, offset: usize) {
    // A leading BOM marks the encoding; it is not a column of the first line.
    let prefix = &source[..offset];
    let prefix = prefix.strip_prefix('\u{FEFF}').unwrap_or(prefix);
    let prefix = prefix.replace("\r\n", "\n").replace('\r', "\n");
    let expected = (
        prefix.split('\n').count(),
        prefix.rsplit('\n').next().unwrap().chars().count() + 1,
    );
    let error = ParseError {
        message: "test".into(),
        offset,
    };
    assert_eq!(
        error.line_col(source),
        expected,
        "{source:?} offset={offset}"
    );
    assert!(
        error
            .display(source)
            .contains(&format!("line {}, column {}", expected.0, expected.1))
    );
}

// These contracts never construct rowan trees. Run this test under strict Miri
// even while the upstream tree implementation needs a provenance workaround.
#[test]
fn lexer_and_diagnostic_contracts() {
    for source in [
        "",
        " \t",
        "\u{FEFF} [λ]\r\nκ=🙂\rnext=value\n",
        "[bad\r\nk= \t#fff\nk=v \\\t\r  tail # note\n",
        "a\u{301}\r\n🙂\rλ\n",
        "a=\\",
        "\u{FEFF}",
        "[[]]\n",
    ] {
        lexer_contract(source);
        for offset in source
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([source.len()])
        {
            diagnostic_contract(source, offset);
        }
    }
}

fn single_line_meaning(source: &str, options: &ParseOptions, file: &File) {
    let line = source
        .strip_prefix('\u{FEFF}')
        .unwrap_or(source)
        .trim_matches([' ', '\t']);
    if line.contains(['\r', '\n', '[', ']']) {
        return;
    }
    let actual: Vec<_> = file
        .preamble_entries()
        .map(|entry| (entry.key().unwrap(), entry.value()))
        .collect();
    if line.is_empty() || line.starts_with([';', '#']) {
        assert!(actual.is_empty(), "{source:?}");
        return;
    }
    if let Some((key, value)) = line.split_once(['=', ':']) {
        let key = key.trim_end_matches([' ', '\t']);
        if !key.is_empty() && !key.contains([' ', '\t']) {
            // Four scalar values cannot encode both a nonempty assignment and
            // a whitespace-separated inline comment (which needs five).
            assert_eq!(
                actual,
                [(key.into(), Some(value.trim_matches([' ', '\t']).into()))],
                "{source:?}, {options:?}"
            );
        }
    } else if !line.contains([' ', '\t']) {
        assert_eq!(actual, [(line.into(), None)], "{source:?}");
    }
}

fn parser_contract(source: &str) {
    lexer_contract(source);
    // Losslessness, line nodes, ordered diagnostics at character boundaries,
    // and diagnostics that depend only on the text before them.
    round_trip::check(source.as_bytes());
    for options in &round_trip::OPTIONS {
        let parsed = parse_with(source, options);
        for error in parsed.errors() {
            diagnostic_contract(source, error.offset);
        }
        let file = File::cast(parsed.syntax()).unwrap();
        single_line_meaning(source, options, &file);
    }
}

#[test]
#[ignore = "bounded exhaustive suite; run in the extended-tests CI job"]
fn all_inputs_through_four_scalars() {
    const ALPHABET: [char; 14] = [
        'a', '=', ':', ' ', '\t', '[', ']', ';', '#', '\\', '\r', '\n', 'λ', '\u{FEFF}',
    ];
    let mut checked = 0;
    for length in 0..=4 {
        for mut code in 0..ALPHABET.len().pow(length) {
            let mut source = String::new();
            for _ in 0..length {
                source.push(ALPHABET[code % ALPHABET.len()]);
                code /= ALPHABET.len();
            }
            parser_contract(&source);
            checked += 1;
        }
    }
    assert_eq!(checked, 41_371);
}

/// SQLite's malformed-file tests corrupt a well-formed file one change at a
/// time. Here every character of every real-world fixture is deleted, replaced
/// by each other character of an alphabet, and has each alphabet character
/// inserted before it (and after the last one). Every corrupted document must
/// pass the checks that the `round_trip` fuzz oracle makes of every parse.
/// Diagnostics must also depend only on the text before them, on both sides
/// of the edit: those before the edited line are the diagnostics of parsing
/// just the unchanged lines before it, and dropping the lines after the edited
/// line keeps every diagnostic up to its end. Each shard asserts how many
/// documents it corrupted, and an ordinary test counts the edits of every
/// line to check those numbers, so no shard can skip a line unnoticed.
mod corruption {
    use std::fs;
    use std::ops::Range;
    use std::panic::{self, AssertUnwindSafe};
    use std::path::Path;

    use ini_edit::{Parse, parse_with};

    use crate::round_trip::{self, line_end, line_starts};

    /// A real-world file, and the number of unchanged lines that each
    /// corrupted document keeps on either side of its edited line.
    struct Fixture {
        name: &'static str,
        text: &'static str,
        context: usize,
    }

    /// Context that keeps the whole file in every corrupted document.
    const WHOLE_FILE: usize = usize::MAX;

    /// Every file in `tests/fixtures`, sorted by name. Corrupting all 129 KB
    /// of the Gitea example 3.7 million times would parse half a terabyte for
    /// each option set, so its documents keep four lines on either side of
    /// the edited line.
    const FIXTURES: [Fixture; 6] = [
        Fixture {
            name: "aws-config",
            text: include_str!("fixtures/aws-config"),
            context: WHOLE_FILE,
        },
        Fixture {
            name: "gitconfig",
            text: include_str!("fixtures/gitconfig"),
            context: WHOLE_FILE,
        },
        Fixture {
            name: "gitea-app.example.ini",
            text: include_str!("fixtures/gitea-app.example.ini"),
            context: 4,
        },
        Fixture {
            name: "my.cnf",
            text: include_str!("fixtures/my.cnf"),
            context: WHOLE_FILE,
        },
        Fixture {
            name: "php.ini",
            text: include_str!("fixtures/php.ini"),
            context: WHOLE_FILE,
        },
        Fixture {
            name: "systemd-unit.service",
            text: include_str!("fixtures/systemd-unit.service"),
            context: WHOLE_FILE,
        },
    ];

    /// Line breaks, every delimiter, blanks, a key character, and two- and
    /// three-byte characters. U+FEFF is a byte order mark at the start.
    const ALPHABET: [char; 14] = [
        '\r', '\n', '[', ']', '=', ':', ';', '#', '\\', ' ', '\t', 'k', 'é', '\u{FEFF}',
    ];

    /// A change of at most one character at byte `at` of a fixture.
    #[derive(Clone, Copy, Debug)]
    struct Edit {
        at: usize,
        removed: Option<char>,
        inserted: Option<char>,
    }

    impl Edit {
        /// Write `text[window]` with this edit applied to `document`.
        fn apply(self, text: &str, window: &Range<usize>, document: &mut String) {
            let end = self.at + self.removed.map_or(0, char::len_utf8);
            // The edit lies inside the window.
            assert!(window.start <= self.at, "{self:?}");
            assert!(end <= window.end, "{self:?}");
            if let Some(removed) = self.removed {
                assert!(text[self.at..].starts_with(removed), "{self:?}");
            }
            document.clear();
            document.push_str(&text[window.start..self.at]);
            document.extend(self.inserted);
            document.push_str(&text[end..window.end]);
        }
    }

    /// How many edits `line_edits` makes of `text[line]`, counted without
    /// making them: for each character, a deletion, a replacement by each
    /// other alphabet character, and an insertion of each alphabet character,
    /// and at the end of the text, an insertion of each alphabet character.
    fn edit_count(text: &str, line: Range<usize>, alphabet: &[char]) -> usize {
        let appended = if line.end == text.len() {
            alphabet.len()
        } else {
            0
        };
        let edited: usize = text[line]
            .chars()
            .map(|current| 2 * alphabet.len() + 1 - usize::from(alphabet.contains(&current)))
            .sum();
        edited + appended
    }

    /// Every edit of the characters in `text[line]`: each one is deleted,
    /// replaced by each other alphabet character, and has each alphabet
    /// character inserted before it. The last line also gets each alphabet
    /// character appended.
    fn line_edits(text: &str, line: Range<usize>, alphabet: &[char]) -> Vec<Edit> {
        let mut edits = Vec::new();
        for (offset, current) in text[line.clone()].char_indices() {
            let at = line.start + offset;
            edits.push(Edit {
                at,
                removed: Some(current),
                inserted: None,
            });
            for &new in alphabet {
                if new != current {
                    edits.push(Edit {
                        at,
                        removed: Some(current),
                        inserted: Some(new),
                    });
                }
                edits.push(Edit {
                    at,
                    removed: None,
                    inserted: Some(new),
                });
            }
        }
        if line.end == text.len() {
            for &new in alphabet {
                edits.push(Edit {
                    at: line.end,
                    removed: None,
                    inserted: Some(new),
                });
            }
        }
        // `every_line_is_corrupted` checks the number of documents that each
        // shard states with `edit_count`, which counts these edits.
        assert_eq!(edits.len(), edit_count(text, line, alphabet));
        edits
    }

    /// Check every edit of line `line` (0-based) of `fixture`, and return the
    /// number of corrupted documents.
    fn corrupt_line(fixture: &Fixture, starts: &[usize], line: usize, alphabet: &[char]) -> usize {
        let text = fixture.text;
        let end_of = |line: usize| starts.get(line + 1).copied().unwrap_or(text.len());
        let last = line.saturating_add(fixture.context).min(starts.len() - 1);
        let window = starts[line.saturating_sub(fixture.context)]..end_of(last);
        // Every document keeps the lines before the edited one, so its
        // diagnostics before that line are those of parsing just those lines.
        let boundary = starts[line] - window.start;
        let prefixes: Vec<Parse> = round_trip::OPTIONS
            .iter()
            .map(|options| parse_with(&text[window.start..starts[line]], options))
            .collect();
        let expected: Vec<_> = prefixes
            .iter()
            .map(|prefix| round_trip::errors_before(prefix, boundary))
            .collect();
        let edits = line_edits(text, starts[line]..end_of(line), alphabet);
        // The unchanged window must pass the same checks.
        let unchanged = Edit {
            at: starts[line],
            removed: None,
            inserted: None,
        };
        let mut document = String::with_capacity(window.len() + '\u{FEFF}'.len_utf8());
        for edit in std::iter::once(unchanged).chain(edits.iter().copied()) {
            let checked = panic::catch_unwind(AssertUnwindSafe(|| {
                edit.apply(text, &window, &mut document);
                // Only the edited line changes. In the document, it ends where
                // the first line break at or after the edit does.
                assert!(document.starts_with(&text[window.start..starts[line]]));
                let end = line_end(&document, edit.at - window.start);
                assert!(text[window.clone()].ends_with(&document[end..]));
                for (options, expected) in round_trip::OPTIONS.iter().zip(&expected) {
                    let parse = round_trip::check_parse(&document, options);
                    assert_eq!(
                        &round_trip::errors_before(&parse, boundary),
                        expected,
                        "{document:?}, {options:?}"
                    );
                    if end < document.len() {
                        let prefix = parse_with(&document[..end], options);
                        assert_eq!(
                            round_trip::errors_before(&parse, end),
                            round_trip::errors_before(&prefix, end),
                            "{document:?} without the lines after byte {end}, {options:?}"
                        );
                    }
                }
            }));
            if let Err(panic) = checked {
                eprintln!("{} line {}: {edit:?}", fixture.name, line + 1);
                panic::resume_unwind(panic);
            }
        }
        edits.len()
    }

    /// Corrupt lines `shard`, `shard + shards`, … of `fixture`, and return
    /// the number of corrupted documents.
    fn corrupt(fixture: &Fixture, alphabet: &[char], shard: usize, shards: usize) -> usize {
        assert!(shard < shards);
        // Each edit is made once.
        for (index, new) in alphabet.iter().enumerate() {
            assert!(!alphabet[..index].contains(new), "{new:?} is listed twice");
        }
        let starts = line_starts(fixture.text);
        (shard..starts.len())
            .step_by(shards)
            .map(|line| corrupt_line(fixture, &starts, line, alphabet))
            .sum()
    }

    /// A new fixture must be corrupted too.
    #[test]
    #[cfg_attr(miri, ignore = "reads files")]
    fn every_fixture_is_corrupted() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let mut names: Vec<String> = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let listed: Vec<&str> = FIXTURES.iter().map(|fixture| fixture.name).collect();
        assert_eq!(
            names, listed,
            "FIXTURES must list every file in tests/fixtures"
        );
        for fixture in &FIXTURES {
            let text = fs::read_to_string(directory.join(fixture.name)).unwrap();
            assert!(
                text == fixture.text,
                "{} has another file's text",
                fixture.name
            );
        }
    }

    /// The shards corrupt every line of every fixture once, each shard into
    /// as many documents as it states. This counts the edits without making
    /// them.
    #[test]
    #[cfg_attr(miri, ignore = "counts the edits of 139 KB of fixtures")]
    fn every_line_is_corrupted() {
        let mut documents = [0; SHARDS];
        let mut whole_files = 0;
        for fixture in &FIXTURES {
            let text = fixture.text;
            let starts = line_starts(text);
            for (line, &start) in starts.iter().enumerate() {
                let end = starts.get(line + 1).copied().unwrap_or(text.len());
                documents[line % SHARDS] += edit_count(text, start..end, &ALPHABET);
            }
            whole_files += edit_count(text, 0..text.len(), &ALPHABET);
        }
        // The lines hold every character of every fixture once.
        assert_eq!(documents.iter().sum::<usize>(), whole_files);
        assert_eq!(whole_files, 3_985_991);
        assert_eq!(documents, SHARD_DOCUMENTS);
    }

    /// A fast sample for every `cargo test`, well under two seconds in a
    /// debug build, keeping one line of context. It makes every deletion, and
    /// every inserted or substituted carriage return, in the fixture with a
    /// continued value. It also makes every deletion in the Git
    /// configuration, where deleting a `#` turns the comment above a section
    /// header into a key.
    #[test]
    #[cfg_attr(miri, ignore = "parses thousands of documents")]
    fn sample() {
        let systemd = Fixture {
            context: 1,
            ..FIXTURES[5]
        };
        assert_eq!(systemd.name, "systemd-unit.service");
        assert_eq!(corrupt(&systemd, &['\r'], 0, 1), 3_580);
        let git = Fixture {
            context: 1,
            ..FIXTURES[1]
        };
        assert_eq!(git.name, "gitconfig");
        assert_eq!(corrupt(&git, &[], 0, 1), 1_752);
    }

    /// Corrupt lines `shard`, `shard + SHARDS`, … of every fixture into the
    /// number of documents that the shard states.
    fn corrupt_fixtures(shard: usize) {
        let documents: usize = FIXTURES
            .iter()
            .map(|fixture| corrupt(fixture, &ALPHABET, shard, SHARDS))
            .sum();
        assert_eq!(documents, SHARD_DOCUMENTS[shard], "shard {shard}");
    }

    /// One test per shard, so that the test harness runs them in parallel.
    /// Shard `n` corrupts lines `n`, `n + SHARDS`, … of every fixture, which
    /// keeps the costs of the shards about equal. Each shard states the
    /// number of documents it corrupts.
    macro_rules! shards {
        ($($test:ident = $shard:literal => $documents:literal),* $(,)?) => {
            const SHARDS: usize = [$($shard),*].len();
            const SHARD_DOCUMENTS: [usize; SHARDS] = [$($documents),*];
            // Test `n` runs shard `n`, so that every shard runs once.
            const _: () = {
                let shards: [usize; SHARDS] = [$($shard),*];
                let mut index = 0;
                while index < SHARDS {
                    assert!(shards[index] == index, "shards are numbered 0, 1, 2, and so on");
                    index += 1;
                }
            };
            $(
                #[test]
                #[ignore = "corrupts every fixture; run in the extended-tests CI job"]
                fn $test() {
                    corrupt_fixtures($shard);
                }
            )*
        };
    }

    shards! {
        shard_0 = 0 => 472_081,
        shard_1 = 1 => 531_063,
        shard_2 = 2 => 514_685,
        shard_3 = 3 => 516_905,
        shard_4 = 4 => 478_968,
        shard_5 = 5 => 477_047,
        shard_6 = 6 => 497_726,
        shard_7 = 7 => 497_516,
    }
}
