//! Exhaust a bounded language, including malformed text and Unicode boundaries.

use ini_edit::ast::{AstNode, File};
use ini_edit::{ParseError, ParseOptions, SyntaxKind, lexer, parse_with};

#[path = "support/line_nodes.rs"]
mod line_nodes;

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
    for flags in 0..4 {
        let options = ParseOptions {
            allow_no_value: flags & 1 != 0,
            inline_comments: flags & 2 != 0,
        };
        let parsed = parse_with(source, &options);
        assert_eq!(parsed.syntax().text().to_string(), source);
        line_nodes::check(source, &parsed.syntax());
        let mut previous = 0;
        for error in parsed.errors() {
            assert!(
                error.offset >= previous && error.offset <= source.len(),
                "{source:?}"
            );
            assert!(source.is_char_boundary(error.offset), "{source:?}");
            diagnostic_contract(source, error.offset);
            previous = error.offset;
        }
        let file = File::cast(parsed.syntax()).unwrap();
        single_line_meaning(source, &options, &file);
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
