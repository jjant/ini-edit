# ini-edit

[![crates.io](https://img.shields.io/crates/v/ini-edit.svg)](https://crates.io/crates/ini-edit)
[![docs.rs](https://docs.rs/ini-edit/badge.svg)](https://docs.rs/ini-edit)
[![CI](https://github.com/jjant/ini-edit/actions/workflows/ci.yml/badge.svg)](https://github.com/jjant/ini-edit/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**A lossless, format-preserving INI parser and editor for Rust.**

Parse, inspect, and edit INI files without losing comments, whitespace, or key ordering. Built on [rowan](https://crates.io/crates/rowan).

## Features

- **Lossless** — `parse(s).syntax().text() == s` for every input, always
- **Format-preserving edits** — modify values, add entries, remove sections while keeping untouched lines byte-for-byte identical
- **Error-tolerant** — always produces a valid tree, even for malformed input
- **Typed AST** — `File`, `Section`, `Entry`, `Key`, `Value` wrappers with ergonomic accessors
- **Full CST** — every whitespace character, comment, and line ending is in the tree
- **Zero unsafe** — `#![deny(unsafe_code)]`

## Quick start

```rust
use ini_edit::ast::{AstNode, File};

let input = "\
; Database settings
[database]
host = localhost
port = 5432
";

let parse = ini_edit::parse(input);
assert!(parse.errors().is_empty());

// Round-trip: tree text == original input
assert_eq!(parse.syntax().text().to_string(), input);

// Typed access
let file = File::cast(parse.syntax()).unwrap();
let db = file.sections().next().unwrap();
assert_eq!(db.name().as_deref(), Some("database"));

for entry in db.entries() {
    println!("{} = {}", entry.key().unwrap(), entry.value().unwrap());
}
```

## Editing

```rust
use ini_edit::editor::Editor;

let src = "\
[server]
host = 0.0.0.0
port = 8080
";
let ed = Editor::new(src);

ed.section("server").set("port", "9090");
ed.section("server").append_entry("timeout", "30");

let output = ed.finish();
// Output:
// [server]
// host = 0.0.0.0
// port = 9090
// timeout = 30
```

## INI File Format Decisions

INI has no formal spec. `ini-edit` makes these choices:

| Feature | Behavior |
|---------|----------|
| Comment markers | `;` and `#` |
| Separators | `=` and `:` |
| Spaces in section names | Allowed: `[my section]` |
| Inline comments | Not supported — `key = value ; this is part of the value` |
| Backslash continuation | Supported: `key = long \`<br>`value` |
| Empty values | `key =` is valid, value is `""` |
| Preamble entries | Keys before first `[section]` accessible via `File::preamble_entries()` |
| Line endings | `\n`, `\r\n`, `\r` all preserved |
| UTF-8 BOM | Handled (treated as whitespace) |
| Malformed input | Always produces a tree; errors reported separately |

## Comparison

| Crate | Approach | Lossless | Editable | Error-tolerant |
|-------|----------|----------|----------|----------------|
| **ini-edit** | Rowan CST + typed AST | ✅ | ✅ | ✅ |
| `ini-roundtrip` | Streaming iterator | ✅ | ❌ | Partial |
| `ini-preserve` | Line-based get/set | ✅ | ✅ | ❌ |
| `rust-ini` | HashMap | ❌ | ✅ | ❌ |
| `configparser` | HashMap | ❌ | ✅ | ❌ |

## MSRV

The minimum supported Rust version is **1.85.0**.

## License

MIT
