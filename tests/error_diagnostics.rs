//! Snapshot tests for error diagnostics.
//!
//! Locks down the exact error messages and positions so that error quality
//! doesn't regress silently.

use ini_edit::{Parse, ParseOptions, parse, parse_with};

fn errors(input: &str) -> String {
    render(input, &parse(input))
}

fn errors_with(input: &str, options: &ParseOptions) -> String {
    render(input, &parse_with(input, options))
}

fn render(input: &str, parsed: &Parse) -> String {
    assert_eq!(
        parsed.syntax().text().to_string(),
        input,
        "round-trip failed"
    );
    if parsed.errors().is_empty() {
        return "no errors".to_string();
    }
    parsed
        .errors()
        .iter()
        .map(|error| error.display(input))
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

#[test]
fn header_without_name_or_bracket() {
    insta::assert_snapshot!(errors("[\nk = v\n"));
}

#[test]
fn text_after_a_key() {
    insta::assert_snapshot!(errors("[s]\nkey junk\n"));
}

#[test]
fn text_after_a_bare_key_where_bare_keys_are_allowed() {
    let options = ParseOptions {
        allow_no_value: true,
        inline_comments: false,
    };
    insta::assert_snapshot!(errors_with(
        "[mysqld]\n!includedir /etc/mysql/conf.d/\n",
        &options
    ));
}

#[test]
fn lines_that_start_with_a_separator() {
    insta::assert_snapshot!(errors("=bad\n  :x\n"));
}

#[test]
fn tabs_before_the_column() {
    insta::assert_snapshot!(errors("[s]\n\tkey\tjunk\n"));
}

#[test]
fn four_digit_line_number() {
    insta::assert_snapshot!(errors(&format!("{}=bad\n", "\n".repeat(999))));
}
