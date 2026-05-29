# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
