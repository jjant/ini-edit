# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-10-08

### Changed

- **Breaking:** every malformed line is now a single node in the syntax tree. A line that starts with an unexpected token is wrapped in the new `SyntaxKind::ERROR_LINE`, and text after a key without a separator (`key junk`) stays inside its `ENTRY`. Code that matches exhaustively on `SyntaxKind`, or walks the children of malformed lines, needs updating. Diagnostics are unchanged ([#60](https://github.com/jjant/ini-edit/pull/60))
- Lines added by the editor use the document's line ending instead of always `\n`. This is the first line break in the current text, or `\n` if there is none, so editing CRLF or CR files no longer mixes line endings ([#63](https://github.com/jjant/ini-edit/pull/63))
- Report unexpected text after a section header (`[s] extra`) as a parse error. It was previously accepted silently ([#52](https://github.com/jjant/ini-edit/pull/52))
- With both `allow_no_value` and `inline_comments` enabled, a bare key can carry an inline comment (`quick  # note`). This was previously a parse error, and assigning a value glued the comment onto it ([#62](https://github.com/jjant/ini-edit/pull/62))
- `Editor::file()` returns an immutable snapshot, as documented, instead of the editor's live tree, so a saved view no longer changes when the editor is edited ([#54](https://github.com/jjant/ini-edit/pull/54))

### Fixed

- Editing a key followed by junk (`key junk`, MySQL's `!includedir /path`) no longer glues the new value onto the junk, and removing that key no longer leaves the junk behind as a new key ([#60](https://github.com/jjant/ini-edit/pull/60))
- `remove_lines` and `insert_raw_lines_at` count each malformed line as one line, so later indices are no longer shifted and removing a line can no longer join two lines ([#60](https://github.com/jjant/ini-edit/pull/60))
- Keep a bare `\r` and a following `\n` as two line breaks when inserting lines, including between lines inserted together, so line indices match a reopened file ([#61](https://github.com/jjant/ini-edit/pull/61))
- Keep a bare `\r` and a following `\n` as two line breaks when deleting entries, sections, or lines in files with mixed line endings ([#58](https://github.com/jjant/ini-edit/pull/58))
- `insert_entry_at_line` with a `line` past the last content line inserts exactly where `append_entry` does, including after trailing malformed lines ([#66](https://github.com/jjant/ini-edit/pull/66))
- Don't count a leading UTF-8 BOM as a column in `ParseError::line_col` and `display` ([#59](https://github.com/jjant/ini-edit/pull/59))
- Parse a first line that is indented after a UTF-8 BOM, and keep the BOM when removing the first section or entry ([#45](https://github.com/jjant/ini-edit/pull/45))
- Fix a panic when adding to a retained section handle after removing all its lines, and keep insertions from joining lines after deletions ([#47](https://github.com/jjant/ini-edit/pull/47))
- Give the same results whether or not a file is saved and reopened between edits. Inserted line breaks belong to their line, so `remove_lines` can no longer join a header to a setting, and a new section after an empty, unterminated value no longer ends up inside that value ([#49](https://github.com/jjant/ini-edit/pull/49))
- Clearing a value no longer turns its inline comment into the value when the file is reopened ([#50](https://github.com/jjant/ini-edit/pull/50))
- Assigning a continued value that ends on a blank line no longer absorbs the inline comment, and clearing a value with `Compact` or `Exact` spacing no longer keeps the old surrounding spaces ([#53](https://github.com/jjant/ini-edit/pull/53))

### Performance

- Parse input with many malformed lines in linear time instead of quadratic time ([#46](https://github.com/jjant/ini-edit/pull/46))
- Repair line boundaries between inserted lines before attaching them to the document, about twice as fast for large raw-line batches that need repairs ([#64](https://github.com/jjant/ini-edit/pull/64))

### Documentation

- Explain that a value ending in a backslash continues on the next line when the file is reopened ([#67](https://github.com/jjant/ini-edit/pull/67))

## [0.3.2] - 2026-09-22

### Added

- Add configurable separator whitespace for entries created or value-updated through the editor ([#43](https://github.com/jjant/ini-edit/pull/43))

## [0.3.1] - 2026-09-21

### Fixed

- Add a canonical separator when assigning a value to a bare key, preventing invalid output such as `flagon` ([#39](https://github.com/jjant/ini-edit/pull/39))
- Preserve bare-CR line endings during verbatim raw-line insertion ([#39](https://github.com/jjant/ini-edit/pull/39))
- Report accurate parse-error locations and source excerpts for LF, CRLF, and bare-CR input ([#39](https://github.com/jjant/ini-edit/pull/39))
- Make empty and reversed `remove_lines` ranges safe no-ops instead of panicking ([#39](https://github.com/jjant/ini-edit/pull/39))

### Changed

- Require exhaustive source, branch, and MC/DC coverage alongside mutation testing, fuzzing, Miri, dependency auditing, and cross-platform checks ([#40](https://github.com/jjant/ini-edit/pull/40))
- Update vulnerable and yanked transitive development dependencies ([#40](https://github.com/jjant/ini-edit/pull/40))

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
