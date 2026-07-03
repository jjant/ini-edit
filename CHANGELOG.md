# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-07-03

### Added

- Add entry handles and guard rename_key against duplicates ([#35](https://github.com/jjant/ini-edit/pull/35))
- Separate auto-created sections with a blank line and never glue ([#37](https://github.com/jjant/ini-edit/pull/37))
- Add Editor::file() read view ([#36](https://github.com/jjant/ini-edit/pull/36))

## [0.2.1] - 2026-05-31

### Added

- Snapshot tests for error diagnostics ([#29](https://github.com/jjant/ini-edit/pull/29))
- Line/column display for `ParseError` ([#28](https://github.com/jjant/ini-edit/pull/28))
- Benchmark comparison against `rust-ini` ([#27](https://github.com/jjant/ini-edit/pull/27))
- `allow_no_value` parse option for bare keys ([#26](https://github.com/jjant/ini-edit/pull/26))
- Real-world INI fixture tests ([#25](https://github.com/jjant/ini-edit/pull/25))
- Editor fuzz target and edge case tests ([#24](https://github.com/jjant/ini-edit/pull/24))

### Fixed

- Out-of-bounds panic in editor surfaced by fuzzing ([#24](https://github.com/jjant/ini-edit/pull/24))

## [0.2.0] - 2026-05-29

### Added

- Rework raw lines API — verbatim insertion, positional support ([#22](https://github.com/jjant/ini-edit/pull/22))
- Add criterion benchmarks and coverage workflow ([#18](https://github.com/jjant/ini-edit/pull/18))

### Changed

- Use #[expect] at statement level for panic lints ([#21](https://github.com/jjant/ini-edit/pull/21))
- Remove stale lint allows, scope remaining ones ([#19](https://github.com/jjant/ini-edit/pull/19))

## [0.1.0] - 2026-05-29

### Added

- Lossless INI parser built on rowan — `parse(s).syntax().text() == s` for all inputs
- Line-aware lexer handling `;`/`#` comments, `=`/`:` separators, spaces in section names
- Backslash line continuation in values
- UTF-8 BOM handling
- CRLF, LF, and CR line ending preservation
- Error-tolerant parsing — always produces a valid tree
- Typed AST layer: `File`, `Section`, `Entry`, `Key`, `Value`
- Editor API with rowan tree surgery (`set`, `append_entry`, `remove_entry`, `rename_key`, `remove`)
- Low-level editor operations (`insert_raw_lines`, `remove_lines`)
- Fuzz testing with cargo-fuzz (round-trip invariant)
- Snapshot tests ported from tree-sitter-ini corpus
- Real-world fixture test (Gitea app.example.ini, 3011 lines)
- GitHub Actions CI (clippy, fmt, test, doc, MSRV 1.85, fuzz)
