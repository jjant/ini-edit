//! Snapshot tests for CST structure, ported from tree-sitter-ini's test corpus.
//!
//! Each test parses an input and snapshots the tree shape. The snapshots are
//! checked in and any structural change shows up as a diff in CI.
//!
//! Reference: <https://github.com/justinmk/tree-sitter-ini/blob/master/test/corpus/main.txt>

use std::fmt::Write as _;

use ini_edit::{SyntaxNode, parse};

fn dump(node: &SyntaxNode, indent: usize) -> String {
    let mut out = String::new();
    let pad = "  ".repeat(indent);
    let _ = writeln!(out, "{pad}{:?} {:?}", node.kind(), node.text_range());
    for child in node.children_with_tokens() {
        match child {
            rowan::NodeOrToken::Node(n) => out.push_str(&dump(&n, indent + 1)),
            rowan::NodeOrToken::Token(t) => {
                let child_pad = "  ".repeat(indent + 1);
                let _ = writeln!(
                    out,
                    "{child_pad}{:?} {:?} {:?}",
                    t.kind(),
                    t.text_range(),
                    t.text()
                );
            }
        }
    }
    out
}

fn tree(input: &str) -> String {
    let p = parse(input);
    // Always assert round-trip alongside structure.
    assert_eq!(p.syntax().text().to_string(), input);
    dump(&p.syntax(), 0)
}

// ─── Tests ported from tree-sitter-ini corpus ───────────────────────────────

#[test]
fn one_section() {
    insta::assert_snapshot!(tree("[a section title]\nfoo = bar\n"));
}

#[test]
fn setting_value_may_be_empty() {
    insta::assert_snapshot!(tree("[section 1]\nsetting 1 =\nsetting 2 = x\n"));
}

#[test]
fn many_sections_some_empty() {
    insta::assert_snapshot!(tree(
        "# comment\n[a section title]\nfoo = bar\n[section 2]\nfoo = bar\n[section 3 which is empty]\n[section4]\nfoo = bar\n"
    ));
}

#[test]
fn only_comments() {
    insta::assert_snapshot!(tree("# comment1\n; comment2\n# comment3\n"));
}

#[test]
fn mixed_comments() {
    insta::assert_snapshot!(tree(
        "; comment1\n[section1]\n; comment2\n# comment3\nkey1=val1\n"
    ));
}

#[test]
fn one_section_empty() {
    insta::assert_snapshot!(tree("[section with no content]\n"));
}

#[test]
fn not_a_comment_in_value() {
    // # and ; mid-value are NOT comments — they're part of the value.
    insta::assert_snapshot!(tree(
        "[foo]\nbar = baz # not-a-comment\nzim = -1>3 ; not-a-comment\n"
    ));
}

#[test]
fn aws_config() {
    insta::assert_snapshot!(tree(
        "# AWS config\n[default]\nregion = us-west-2\noutput = json\n[profile dev-user]\nregion = us-east-1\noutput = text\n"
    ));
}

#[test]
fn global_parameters() {
    // Entries before the first section (preamble).
    insta::assert_snapshot!(tree(
        "# leading comment\na=b\n# another comment\n[section]\nc=d\n"
    ));
}

// ─── Additional cases not in tree-sitter-ini ────────────────────────────────

#[test]
fn colon_separator() {
    insta::assert_snapshot!(tree("[s]\nkey: value\n"));
}

#[test]
fn duplicate_sections() {
    insta::assert_snapshot!(tree("[s]\na=1\n[s]\nb=2\n"));
}

#[test]
fn malformed_unclosed_section() {
    insta::assert_snapshot!(tree("[unclosed\nk=v\n"));
}

#[test]
fn malformed_empty_section_name() {
    insta::assert_snapshot!(tree("[]\nk=v\n"));
}

#[test]
fn unicode() {
    insta::assert_snapshot!(tree("[セクション]\nキー = 値\n"));
}

#[test]
fn crlf_line_endings() {
    insta::assert_snapshot!(tree("[s]\r\nk=v\r\n"));
}

#[test]
fn value_with_equals_and_brackets() {
    insta::assert_snapshot!(tree("[s]\nurl = https://x.com?a=1&b=[2]\n"));
}
