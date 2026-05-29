//! Real-world fixture tests — large INI files from open source projects.

use ini_edit::ast::{AstNode, File};
use ini_edit::parse;

/// Gitea's app.example.ini (~3000 lines, MIT licensed).
/// Source: <https://github.com/go-gitea/gitea/blob/main/custom/conf/app.example.ini>
const GITEA: &str = include_str!("fixtures/gitea-app.example.ini");

#[test]
fn gitea_round_trips() {
    let p = parse(GITEA);
    assert_eq!(p.syntax().text().to_string(), GITEA);
}

#[test]
fn gitea_no_errors() {
    let p = parse(GITEA);
    assert!(
        p.errors().is_empty(),
        "expected no errors, got: {:?}",
        p.errors()
    );
}

#[test]
fn gitea_ast_traversal() {
    let p = parse(GITEA);
    let file = File::cast(p.syntax()).unwrap();

    let sections: Vec<_> = file.sections().collect();
    assert!(
        sections.len() >= 5,
        "expected several sections, got {}",
        sections.len()
    );

    // Spot-check known sections exist.
    let names: Vec<_> = sections
        .iter()
        .filter_map(ini_edit::ast::Section::name)
        .collect();
    assert!(names.contains(&"server".to_string()));
    assert!(names.contains(&"database".to_string()));

    // Verify AST traversal doesn't panic on any section/entry.
    let total_entries: usize = sections.iter().map(|s| s.entries().count()).sum();
    let _ = total_entries;
}
