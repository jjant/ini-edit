//! Real-world fixture tests — full CST snapshots.
//!
//! Each fixture is parsed and the entire concrete syntax tree is snapshotted,
//! including every whitespace token, comment, and newline. This proves the
//! lossless parser accounts for every byte.

use std::fmt::Write as _;

use ini_edit::{SyntaxNode, parse};

fn cst(node: &SyntaxNode, indent: usize) -> String {
    let mut out = String::new();
    let pad = "  ".repeat(indent);
    let _ = writeln!(out, "{pad}{:?}", node.kind());
    for child in node.children_with_tokens() {
        match child {
            rowan::NodeOrToken::Node(n) => out.push_str(&cst(&n, indent + 1)),
            rowan::NodeOrToken::Token(t) => {
                let child_pad = "  ".repeat(indent + 1);
                let _ = writeln!(out, "{child_pad}{:?} {:?}", t.kind(), t.text());
            }
        }
    }
    out
}

fn dump(input: &str) -> String {
    let p = parse(input);
    assert_eq!(p.syntax().text().to_string(), input, "round-trip failed");

    let mut out = String::new();
    if p.errors().is_empty() {
        let _ = writeln!(out, "errors: none");
    } else {
        let _ = writeln!(out, "errors:");
        for e in p.errors() {
            let _ = writeln!(out, "  - {e:?}");
        }
    }
    let _ = writeln!(out);
    out.push_str(&cst(&p.syntax(), 0));
    out
}

const GITCONFIG: &str = include_str!("fixtures/gitconfig");
const PHP_INI: &str = include_str!("fixtures/php.ini");
const AWS_CONFIG: &str = include_str!("fixtures/aws-config");
const SYSTEMD_UNIT: &str = include_str!("fixtures/systemd-unit.service");
const MY_CNF: &str = include_str!("fixtures/my.cnf");
const GITEA: &str = include_str!("fixtures/gitea-app.example.ini");

#[test]
fn gitea() {
    insta::assert_snapshot!(dump(GITEA));
}

#[test]
fn gitconfig() {
    insta::assert_snapshot!(dump(GITCONFIG));
}

#[test]
fn php_ini() {
    insta::assert_snapshot!(dump(PHP_INI));
}

#[test]
fn aws_config() {
    insta::assert_snapshot!(dump(AWS_CONFIG));
}

#[test]
fn systemd_unit() {
    insta::assert_snapshot!(dump(SYSTEMD_UNIT));
}

#[test]
fn my_cnf() {
    insta::assert_snapshot!(dump(MY_CNF));
}
