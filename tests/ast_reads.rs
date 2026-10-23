//! The typed AST reads a node's children from its green node. Every accessor must return exactly
//! what a walk of rowan's cursors returns: on every parse of the fixtures, and on random trees that
//! no parser makes, with empty tokens and nodes and children in any order. The parser's oracles in
//! `tests/support/round_trip.rs` and the differential test run the same check on their inputs.

use std::fs;
use std::path::Path;

use ini_edit::ast::{AstNode, Entry, Section};
use ini_edit::{ParseOptions, SyntaxKind, SyntaxNode, parse_with};

#[path = "support/ast_reads.rs"]
mod ast_reads;

/// The kinds of the nodes of random trees: every node kind.
const NODE_KINDS: [SyntaxKind; 9] = [
    SyntaxKind::ROOT,
    SyntaxKind::SECTION,
    SyntaxKind::SECTION_HEADER,
    SyntaxKind::ENTRY,
    SyntaxKind::KEY,
    SyntaxKind::VALUE,
    SyntaxKind::COMMENT_LINE,
    SyntaxKind::BLANK_LINE,
    SyntaxKind::ERROR_LINE,
];

/// The kinds of the tokens of random trees: every token kind.
const TOKEN_KINDS: [SyntaxKind; 10] = [
    SyntaxKind::WHITESPACE,
    SyntaxKind::NEWLINE,
    SyntaxKind::COMMENT,
    SyntaxKind::L_BRACK,
    SyntaxKind::R_BRACK,
    SyntaxKind::EQ,
    SyntaxKind::COLON,
    SyntaxKind::IDENT,
    SyntaxKind::VALUE_TEXT,
    SyntaxKind::LEX_ERROR,
];

/// The texts of the tokens of random trees. An empty token, or a node with only empty tokens or
/// none, shares its range with its empty siblings.
const TEXTS: [&str; 3] = ["", "a", "; b"];

/// The most nodes open at once in a random tree, the root included.
const DEPTH_MAX: usize = 4;

/// The most building steps of a random tree after its root starts.
const STEPS_MAX: usize = 32;

/// The random trees checked.
const TREES: u64 = 100_000;

#[test]
#[cfg_attr(miri, ignore = "reads files")]
fn every_fixture_reads_as_cursors_read_it() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut fixtures = 0;
    for file in fs::read_dir(&directory).unwrap() {
        let source = fs::read_to_string(file.unwrap().path()).unwrap();
        for flags in 0..4 {
            let options = ParseOptions {
                allow_no_value: flags & 1 != 0,
                inline_comments: flags & 2 != 0,
            };
            ast_reads::check(&parse_with(&source, &options).syntax());
        }
        fixtures += 1;
    }
    // AWS, Git, Gitea, MySQL, PHP and systemd, at least.
    assert!(fixtures >= 6, "{fixtures} fixtures");
}

#[test]
fn random_trees_read_as_cursors_read_them() {
    let mut state = 0;
    let mut found = Found::default();
    for _ in 0..TREES {
        let root = random_tree(&mut state);
        ast_reads::check(&root);
        found.count(&root);
    }
    // The trees hold what the accessors look for, and empty siblings, often enough that the check
    // means something.
    assert!(found.section_names >= 1_000, "{found:?}");
    assert!(found.keys >= 1_000, "{found:?}");
    assert!(found.values >= 1_000, "{found:?}");
    assert!(found.empty_siblings >= 1_000, "{found:?}");
}

/// What the random trees hold, counted over all of them.
#[derive(Debug, Default)]
struct Found {
    /// Sections whose name is not `None`.
    section_names: usize,
    /// Entries whose key is not `None`.
    keys: usize,
    /// Entries whose value is not empty.
    values: usize,
    /// Pairs of consecutive child nodes that are both empty: unless a token with text lies between
    /// them, they share one range.
    empty_siblings: usize,
}

impl Found {
    fn count(&mut self, root: &SyntaxNode) {
        for node in root.descendants() {
            let section_name = Section::cast(node.clone()).and_then(|section| section.name());
            self.section_names += usize::from(section_name.is_some());
            if let Some(entry) = Entry::cast(node.clone()) {
                self.keys += usize::from(entry.key().is_some());
                self.values += usize::from(entry.value().is_some_and(|value| !value.is_empty()));
            }
            let empty = |child: &SyntaxNode| child.text_range().is_empty();
            let children: Vec<SyntaxNode> = node.children().collect();
            let pairs = children.windows(2);
            self.empty_siblings += pairs
                .filter(|pair| empty(&pair[0]) && empty(&pair[1]))
                .count();
        }
    }
}

/// A random tree of at most [`DEPTH_MAX`] levels of nodes, built without recursion. Half of the
/// children are of a kind that the parser puts in their parent, so that the accessors find what
/// they look for; the others are of any kind.
fn random_tree(state: &mut u64) -> SyntaxNode {
    let mut builder = rowan::GreenNodeBuilder::new();
    let mut open = vec![pick(state, &NODE_KINDS)];
    builder.start_node(open[0].into());
    for _ in 0..next_below(state, STEPS_MAX + 1) {
        let parent = open[open.len() - 1];
        match next_below(state, 3) {
            0 if open.len() < DEPTH_MAX => {
                let kind = pick_child(state, usual_nodes(parent), &NODE_KINDS);
                builder.start_node(kind.into());
                open.push(kind);
            }
            1 if open.len() > 1 => {
                builder.finish_node();
                open.pop();
            }
            _ => {
                let kind = pick_child(state, usual_tokens(parent), &TOKEN_KINDS);
                builder.token(kind.into(), TEXTS[next_below(state, TEXTS.len())]);
            }
        }
    }
    for _ in open {
        builder.finish_node();
    }
    SyntaxNode::new_root(builder.finish())
}

/// The kinds of the child nodes that the parser puts in a `parent` node.
fn usual_nodes(parent: SyntaxKind) -> &'static [SyntaxKind] {
    match parent {
        SyntaxKind::ROOT => &[SyntaxKind::SECTION, SyntaxKind::ENTRY],
        SyntaxKind::SECTION => &[
            SyntaxKind::SECTION_HEADER,
            SyntaxKind::ENTRY,
            SyntaxKind::COMMENT_LINE,
        ],
        SyntaxKind::ENTRY => &[SyntaxKind::KEY, SyntaxKind::VALUE],
        _ => &NODE_KINDS,
    }
}

/// The kinds of the tokens that the parser puts in a `parent` node.
fn usual_tokens(parent: SyntaxKind) -> &'static [SyntaxKind] {
    match parent {
        SyntaxKind::SECTION_HEADER | SyntaxKind::KEY => &[SyntaxKind::IDENT],
        SyntaxKind::ENTRY => &[SyntaxKind::EQ, SyntaxKind::COLON, SyntaxKind::COMMENT],
        SyntaxKind::VALUE => &[SyntaxKind::VALUE_TEXT],
        SyntaxKind::COMMENT_LINE => &[SyntaxKind::COMMENT],
        _ => &TOKEN_KINDS,
    }
}

/// A kind from `usual` or, as often, from `any`.
fn pick_child(state: &mut u64, usual: &[SyntaxKind], any: &[SyntaxKind]) -> SyntaxKind {
    if next_below(state, 2) == 0 {
        pick(state, usual)
    } else {
        pick(state, any)
    }
}

/// A random element of `kinds`.
fn pick(state: &mut u64, kinds: &[SyntaxKind]) -> SyntaxKind {
    kinds[next_below(state, kinds.len())]
}

/// The next number below `count` from the linear congruential generator `state`.
fn next_below(state: &mut u64, count: usize) -> usize {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let high = usize::try_from(*state >> 33).unwrap();
    high % count
}
