//! Snapshot tests for error diagnostics.
//!
//! Locks down the exact error messages and positions so that error quality
//! doesn't regress silently.

use ini_edit::parse;

fn errors(input: &str) -> String {
    let p = parse(input);
    assert_eq!(p.syntax().text().to_string(), input, "round-trip failed");
    if p.errors().is_empty() {
        return "no errors".to_string();
    }
    p.errors()
        .iter()
        .map(|e| e.display(input))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn unclosed_section() {
    insta::assert_snapshot!(errors("[unclosed\nk = v\n"));
}

#[test]
fn empty_section_name() {
    insta::assert_snapshot!(errors("[]\nk = v\n"));
}

#[test]
fn missing_separator() {
    insta::assert_snapshot!(errors("[s]\nbare_key\n"));
}

#[test]
fn multiple_errors() {
    insta::assert_snapshot!(errors("[unclosed\nbare_key\n"));
}

#[test]
fn stray_bracket_at_root() {
    insta::assert_snapshot!(errors("]\n[s]\nk=v\n"));
}

#[test]
fn bare_keys_mysql_style() {
    insta::assert_snapshot!(errors("[mysqldump]\nquick\nquote-names\nmax = 64M\n"));
}

#[test]
fn valid_input_no_errors() {
    insta::assert_snapshot!(errors("[s]\nk = v\n"));
}
