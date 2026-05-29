//! Integration tests asserting the lossless round-trip invariant and
//! end-to-end typed API usage.

use ini_edit::ast::{AstNode, File};
use ini_edit::{SyntaxKind, parse};

fn assert_round_trip(s: &str) {
    let p = parse(s);
    assert_eq!(p.syntax().text().to_string(), s, "round-trip failed");
}

#[test]
fn round_trip_minimal() {
    assert_round_trip("");
    assert_round_trip("\n");
    assert_round_trip("\r\n");
    assert_round_trip("k=v\n");
    assert_round_trip("[s]\n");
}

#[test]
fn round_trip_comments() {
    assert_round_trip("; leading\n# also\nk=v\n");
}

#[test]
fn round_trip_complex() {
    assert_round_trip(
        "\
; Database settings
[database]
host = localhost
port = 5432
user = admin
password =

# Cache config
[cache]
backend: redis
ttl: 3600

[empty_section]

[trailing_no_newline]
k = v",
    );
}

#[test]
fn round_trip_special_chars_in_values() {
    assert_round_trip("[s]\nurl = https://example.com/path?a=1&b=[2]&c=3:4\n");
    assert_round_trip("[s]\narray = [1, 2, 3]\n");
}

#[test]
fn round_trip_unicode() {
    assert_round_trip("[セクション]\nキー = 値\n");
}

#[test]
fn round_trip_crlf() {
    assert_round_trip("[s]\r\nk=v\r\n\r\n[t]\r\nx=y\r\n");
}

#[test]
fn round_trip_spaces_in_section_name() {
    assert_round_trip("[my section]\nk = v\n");
}

#[test]
fn malformed_still_round_trips() {
    assert_round_trip("[unclosed\n");
    assert_round_trip("no_separator\n");
    assert_round_trip("[]\n");
    assert_round_trip("[a] junk after\n");
}

#[test]
fn malformed_records_errors() {
    let p = parse("[unclosed\n");
    assert!(!p.errors().is_empty());
    assert_eq!(p.syntax().text().to_string(), "[unclosed\n");
}

#[test]
fn typed_api_end_to_end() {
    let src = "; comment\n[server]\nhost = 0.0.0.0\nport = 8080\n[client]\ntimeout: 30\n";
    let p = parse(src);
    assert!(p.errors().is_empty());
    let file = File::cast(p.syntax()).unwrap();

    let names: Vec<_> = file.sections().filter_map(|s| s.name()).collect();
    assert_eq!(names, vec!["server", "client"]);

    let server = file.sections().next().unwrap();
    let kvs: Vec<_> = server
        .entries()
        .filter_map(|e| Some((e.key()?, e.value()?)))
        .collect();
    assert_eq!(
        kvs,
        vec![
            ("host".into(), "0.0.0.0".into()),
            ("port".into(), "8080".into()),
        ]
    );

    let client = file.sections().nth(1).unwrap();
    assert!(client.entries().next().unwrap().uses_colon());
}

#[test]
fn root_kind() {
    assert_eq!(parse("[s]\n").syntax().kind(), SyntaxKind::ROOT);
}
