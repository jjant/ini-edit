# ini-edit

[![crates.io][crates.io badge]][crates.io]
[![docs.rs][docs.rs badge]][docs.rs]
[![CI][CI badge]][CI]
[![codecov][codecov badge]][codecov]
[![License: MIT][license badge]](LICENSE)

[crates.io badge]: https://img.shields.io/crates/v/ini-edit.svg
[crates.io]: https://crates.io/crates/ini-edit
[docs.rs badge]: https://docs.rs/ini-edit/badge.svg
[docs.rs]: https://docs.rs/ini-edit
[CI badge]: https://github.com/jjant/ini-edit/actions/workflows/ci.yml/badge.svg
[CI]: https://github.com/jjant/ini-edit/actions
[codecov badge]: https://codecov.io/gh/jjant/ini-edit/graph/badge.svg
[codecov]: https://codecov.io/gh/jjant/ini-edit
[license badge]: https://img.shields.io/badge/license-MIT-blue.svg

**A lossless, format-preserving INI parser and editor for Rust.**

Parse, inspect, and edit INI files without losing comments, whitespace, or key ordering. Built
on [rowan](https://crates.io/crates/rowan).

## Features

- **Lossless** — `parse(text).syntax().text() == text` for every input of at most
  `MAX_INPUT_LEN` bytes (see the size limit below)
- **Format-preserving edits** — modify values, add entries, remove sections while keeping
  untouched lines byte-for-byte identical
- **Error-tolerant** — produces a valid tree for every input within the size limit, even for
  malformed input
- **Typed AST** — `File`, `Section`, `Entry`, `Key`, `Value` wrappers with ergonomic accessors
- **Full CST** — every whitespace character, comment, and line ending is in the tree
- **Zero unsafe** — `#![deny(unsafe_code)]`
- **Thread-safe documents** — `Parse` and `Editor` are `Send` and `Sync`: move one to another
  thread, or share it between threads that read it. Syntax-tree cursors (`SyntaxNode`, `File`,
  `Section`, ...) are neither, so each thread builds its own

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
use ini_edit::ParseOptions;
use ini_edit::editor::{EditError, EditOptions, Editor, SeparatorSpacing};

fn main() -> Result<(), EditError> {
    let source = "\
[server]
host = 0.0.0.0
port = 8080
";
    let parse_options = ParseOptions { allow_no_value: false, inline_comments: false };
    let edit_options = EditOptions { separator_spacing: SeparatorSpacing::Preserve };
    let mut editor = Editor::new(source, &parse_options, &edit_options)?;

    let mut server = editor.section_or_insert("server")?;
    server.set("port", "9090")?;
    server.append_entry("timeout", "30")?;

    assert_eq!(
        editor.finish(),
        "[server]\nhost = 0.0.0.0\nport = 9090\ntimeout = 30\n"
    );
    Ok(())
}
```

Every edit is checked against the parser before it is applied: an edit a reader would not read
back as written (for example a value with a line break) returns an `EditError` and leaves the
document unchanged.

Separator whitespace for edited entries is configurable. Existing formatting
is preserved by default; compact and exact policies normalize only entries
that are created or whose values are updated:

```rust
use ini_edit::ParseOptions;
use ini_edit::editor::{EditError, EditOptions, Editor, SeparatorSpacing};

fn main() -> Result<(), EditError> {
    let parse_options = ParseOptions { allow_no_value: false, inline_comments: false };
    let options = EditOptions {
        separator_spacing: SeparatorSpacing::Compact,
    };
    let mut editor = Editor::new("[service]\nendpoint = old\n", &parse_options, &options)?;

    editor
        .section_or_insert("service")?
        .set("endpoint", "unix:///tmp/service.sock")?;

    assert_eq!(
        editor.finish(),
        "[service]\nendpoint=unix:///tmp/service.sock\n"
    );
    Ok(())
}
```

## INI File Format Decisions

INI has no formal spec. `ini-edit` makes these choices:

| Feature | Behavior |
|---------|----------|
| Comment markers | `;` and `#` |
| Separators | `=` and `:` |
| Spaces in section names | Allowed: `[my section]` |
| Inline comments | Opt-in via `parse_with(.., &ParseOptions { inline_comments: true, ..Default::default() })`; a `;`/`#` preceded by whitespace starts a trailing comment (`key = value ; comment`, or `flag ; comment` for a bare key). Off by default, so the marker stays part of the value. |
| Backslash continuation | Supported: `key = long \`<br>`value`. A line ending in `\` always continues, so the editor refuses values whose last line ends in `\` (e.g. `C:\dir\`): drop the trailing backslash. |
| Empty values | `key =` is valid, value is `""` |
| Preamble entries | Keys before the first `[section]`: read with `File::preamble_entries()`, edit with `Editor::preamble()` |
| Line endings | `\n`, `\r\n`, `\r` all preserved. Lines added by the editor use the document's first line ending (`\n` if it has none), and CRLF where a line break would otherwise read together with a neighboring CR or LF. A removal between a CR and an LF blank line keeps every byte and drops that blank line, which now reads as part of one CRLF. |
| UTF-8 BOM | Handled (kept at the root, not part of the first line). An edit never makes U+FEFF the first character of a document without one. |
| Malformed input | Produces a tree for every input within the size limit; errors reported separately |
| Bare keys (no value) | Opt-in via `parse_with(.., &ParseOptions { allow_no_value: true })` |
| Size limit | Inputs up to `ini_edit::MAX_INPUT_LEN` bytes (`u32::MAX`, the range of rowan's text positions). `parse` and `parse_with` panic on a longer input, the same way in debug and release builds, with a message that names the limit. The editor holds one byte less and refuses larger documents and edits with `EditError::DocumentTooLarge` |

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

## Testing

The crate requires 100% function, instantiation, line, region, branch, and
MC/DC coverage, plus mutation testing, fuzzing, Miri, ThreadSanitizer,
debug/release, MSRV, and dependency-policy checks. Independent edit models
check meaning and unchanged bytes; exhaustive small inputs and large-input
scaling tests extend the boundary coverage. See [TESTING.md](TESTING.md),
including the current upstream limitation on full-suite Miri borrow checking.

## License

MIT
