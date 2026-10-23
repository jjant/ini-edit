//! Unit tests of the editor. They reach private items (`with_len_max`, the tree) and count toward
//! `--lib` coverage, so every production decision is tested here.

mod behaviors;
mod examples;
mod fragment;
mod insertion;
mod lookups;
mod options;
mod precedence;
mod regressions;
mod removal;
mod rewrite;
mod sections;
mod support;
