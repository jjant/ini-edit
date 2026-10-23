# Standards

ini-edit follows two outside standards: [TigerStyle][tigerstyle], TigerBeetle's engineering style,
and [How SQLite Is Tested][sqlite], the testing practice behind SQLite. Both were written for
larger, stateful systems, and not every rule fits a small, pure, in-memory Rust library. This
document records how ini-edit applies them and, for each rule it deliberately does not follow, why.
TigerStyle asks for exactly that: "Always motivate, always say why."

Rules that this document does not mention apply as written. [TESTING.md](../TESTING.md) describes
the test layers and gates in detail, and [CONTRIBUTING.md](../CONTRIBUTING.md) lists what every
change needs.

## TigerStyle

TigerStyle ranks its design goals: safety, then performance, then developer experience.

### Safety

- **Assertions catch programmer errors.** A failed assertion means a bug. Wrong runtime data, such
  as malformed text or an invalid key, is an operating error, which TigerStyle says to handle.
  Malformed text still parses to a tree, with a `ParseError` and a byte offset for each problem,
  and the editor answers an edit that would not read back as written with an `EditError`, changing
  nothing.
- **Cheap checks run in every build.** O(1) checks use `assert!`. Checks that walk the whole
  document, such as the editor's comparison of its tree with a fresh parse after every edit, are
  compiled only under `#[cfg(all(debug_assertions, not(miri)))]`, so they run in debug builds: the
  debug test run, coverage and mutation runs, and every fuzz target, because `cargo fuzz` keeps
  debug assertions on.
- **One condition per assertion.** Write `assert!(a && b)` as `assert!(a); assert!(b);`, and an
  invariant `a || b` as the implication `if !a { assert!(b) }`. The coverage gate counts each
  operand of `&&` and `||` inside an assertion as a branch, and the failing side of an assertion
  never runs, so a compound condition fails the 100% branch gate.
- **Paired checks.** Edits are checked along two independent paths: the live tree after each edit,
  and a fresh parse of the saved text. The model-based tests described in TESTING.md require both to
  agree.
- **Limits.** No library function recurses, and every loop is bounded by the size of the text or
  tree it walks. Every CI job has a time limit, every fuzz input a 10-second timeout and a 1 GiB
  memory limit, and every mutant a 30-second timeout, so a hang fails instead of stalling.
- **The strictest warnings.** `clippy::all` and `clippy::pedantic` are on, and CI fails on any
  warning from rustc, clippy or rustdoc. `unsafe_code` is denied, and `missing_docs` warns.

### Performance

- **Costs are written down.** The `Editor` rustdoc states what an edit costs, and recommends
  building a large document as text and parsing it once, because adding its entries one at a time
  takes quadratic time. The superlinear-cost row below explains why.
- **Growth is measured.** The extended tests parse large inputs, and a separate check measures CPU
  time and peak memory as inputs grow, to catch superlinear regressions.

### Developer experience

- **Size limits.** Functions stay within 70 lines and lines within 100 columns, including tests, CI
  files and Markdown. rustfmt wraps code at 100 columns.
- **Names.** Full words (`source`, not `src`) in snake_case, with units in the name or in its
  documentation: `ParseError::offset` is documented as a byte offset.
- **Say why.** Comments explain the reasons behind the code. Pull requests are squash-merged with
  their title as the commit subject and their description as the commit body, so descriptions are
  written as commit messages.
- **Dependencies.** The library depends only on rowan; rowan and its four dependencies are all that
  users compile. Dev-dependencies serve the tests and benchmarks and never ship.

## How SQLite Is Tested

- **Independent harnesses (§2).** Unit tests check internals, and integration and snapshot tests
  use only the public API. Differential tests compare against rust-ini and configparser,
  independent models check edits, bounded exhaustive tests enumerate every short input over a
  small alphabet, and libFuzzer targets stress parsing and editing. Ordinary `cargo test` is the
  quick tier, like SQLite's "veryquick" tests; the exhaustive and large-input tests run in a
  separate CI job.
- **Anomalies and fuzzing (§3, §4).** For a parser, the anomaly that matters is malformed or
  hostile text, and fuzzing is how ini-edit tests it: each target fuzzes for 10 seconds on every
  pull request, 60 seconds on every push to `main`, and 15 minutes every Sunday. Like SQLite's
  fuzzcheck (§4.1.5), every `cargo test` replays the inputs committed under
  `tests/fuzz_regressions/`, and CI's fuzz job replays them under libFuzzer before it fuzzes. Like
  its malformed-database tests (§4.2), the extended tests corrupt every real-world fixture one
  character at a time.
- **Boundary values (§4.3).** Decision tables vary line endings, separators, whitespace, comments,
  and empty, unterminated and malformed input. Editor tests pass positions and ranges at the edge
  and past it, up to `usize::MAX`, and require an error past the edge, as SQLite's tests do beyond
  its limits.
- **Regression tests (§5).** A bug is fixed only when a test that fails without the fix is
  committed with it, and a fuzz finding only when its minimized input is committed too.
- **Coverage (§7).** CI requires 100% of functions, instantiations, lines, regions, branches and
  MC/DC conditions. Like SQLite, which measures its core library, it measures production code only:
  the bodies of unit tests are compiled with `coverage(off)`. Coverage is a test of the tests
  (§7.5), so the suite also runs as delivered: in debug and release builds, on the minimum
  supported Rust version, and on Linux, macOS and Windows.
- **Mutation testing (§7.6).** cargo-mutants changes `src/` one mutation at a time, and CI requires
  every mutant to be caught.
- **Dynamic analysis (§6, §8).** Debug-only assertions play the role of the asserts that SQLite
  enables only in debug builds (§8.1). Miri runs the library tests and reports undefined behavior
  and memory leaks. The lexer tests run with full borrow checking, and the rest with Stacked
  Borrows off because of an upstream rowan issue that TESTING.md describes. The fuzz targets run
  under AddressSanitizer.
- **Checklists (§10).** Every item of the release checklist in TESTING.md is a CI job. release-plz
  opens the release pull request, and the maintainer reviews its changelog and checks before
  merging it.
- **Static analysis (§11).** rustc, clippy and rustdoc run with warnings denied, and cargo-deny
  checks advisories, licenses, duplicate dependencies and sources.

## Deliberate deviations

Each row names a rule that ini-edit does not follow as written, says why, and says what it does
instead. Quoted rules are the standard's own words; the others paraphrase it.

| Source | Rule | Why ini-edit departs from it, and what it does instead |
|---|---|---|
| TigerStyle, Safety | "Do not use recursion to ensure that all executions that should be bounded are bounded." | The library does not recurse. Two test helpers that print a syntax tree, `cst` in `tests/real_world.rs` and `dump` in `tests/snapshots_test.rs`, recurse into child nodes. The parser nests nodes at most four deep (`ROOT`, `SECTION`, a line, then `KEY` or `VALUE`), so the recursion is bounded, and an explicit stack would only make the helpers longer. |
| TigerStyle, Safety | "Use explicitly-sized types like `u32` for everything, avoid architecture-specific `usize`." | Public offsets, positions and ranges, such as `ParseError::offset` and the editor's line positions, stay `usize`, the type Rust uses for lengths and indexing. Changing them would break every caller without removing a bug. The limit that matters is rowan's `u32` text size (4 GiB), which belongs where text enters the crate, not in every signature. |
| TigerStyle, Safety | "All memory must be statically allocated at startup." | Does not apply: ini-edit is a library built on rowan, whose trees allocate by design, and documents can have any size. |
| TigerStyle, Safety | "The assertion density of the code must average a minimum of two assertions per function." | The target applies to the lexer, parser and editor algorithms, where the invariants are. One-expression accessors are exempt: the AST accessors return `Option` because a tree may be malformed or built by the caller, and trivial getters have nothing to check. Padding them would add noise and catch nothing. |
| TigerStyle, Safety | "Assert all function arguments and return values, pre/postconditions and invariants." | Public functions do not assert what callers pass. Keys, values, section names, raw lines, positions and ranges often come from user data, so a bad one is an operating error, which TigerStyle says to handle: a panic would crash the caller's program over a bad value. The editor refuses it with an `EditError` and changes nothing, any text parses to a tree, and `ParseError::line_col` and `display` do not panic for any offset or source text. |
| TigerStyle, Safety | "Split compound conditions into simple conditions using nested `if/else` branches." | Production code keeps compound conditions where they read best. CI requires 100% MC/DC, which shows that each condition independently changes its decision, so nesting would add length without adding assurance. Assertions are the exception, because coverage counts each operand of an assertion's condition as a branch whose failing side never runs. |
| TigerStyle, Safety | Check every invariant in every build. | Checks that walk the whole document, such as the editor's reparse after every edit, are compiled only under `cfg(all(debug_assertions, not(miri)))`, as SQLite's production builds disable its asserts (§8.1): in release builds, every parse or edit would pay for a full pass over the document. The debug test run, coverage, mutation testing and every fuzz target still run them, because they build with debug assertions. Miri skips them: it looks for undefined behavior rather than logic errors, and it is far slower. O(1) checks stay on in every build. |
| TigerStyle, Safety | "Don't do things directly in reaction to external events." | Does not apply: ini-edit is a pure, synchronous library with no I/O and no events. |
| TigerStyle, Safety | "All compiler warnings at the compiler's strictest setting", read as every clippy lint. | CI fails on any warning from rustc, rustdoc, or clippy's `all` and `pedantic` groups, and lints from other groups are enabled one at a time. The next seven rows are the lints left off on purpose; the §11 row covers the `restriction` group as a whole. |
| TigerStyle, Safety | Strictest warnings: `shadow_reuse`, `shadow_same` and `shadow_unrelated`. | Rebinding a name to a refined value, such as a trimmed or narrowed copy, is idiomatic Rust and keeps one name for one thing; the lints cannot tell that from a misleading rebinding, which review catches. |
| TigerStyle, Safety | Strictest warnings: `missing_assert_message`. | A failed `assert!` already reports its condition and location, and `assert_eq!` also prints both operands, so a message would mostly repeat them. |
| TigerStyle, Safety | Strictest warnings: `missing_docs_in_private_items`. | It demands a doc comment on every private item, even a self-explanatory one, while TigerStyle asks for comments that explain why the code is written the way it is. |
| TigerStyle, Safety | Strictest warnings: `cognitive_complexity`. | clippy's documentation says that the lint does not measure how hard code is to understand, and recommends `too_many_lines`, which enforces TigerStyle's 70-line limit here. |
| TigerStyle, Safety | Strictest warnings: `unused_results` outside `src/`. | It is on for the library and its unit tests. In integration tests and benchmarks it fires on builder and helper calls whose results are not needed. |
| TigerStyle, Safety | Strictest warnings: the `nursery` group. | Its lints are still under development and change between toolchains, so useful ones, such as `use_self`, are enabled one at a time. |
| TigerStyle, Safety | Strictest warnings: `excessive_nesting`. | Its threshold is one setting for every target, and nested loops and closures are natural in table-driven tests, so it stays advisory. |
| TigerStyle, Performance | Design out superlinear costs. | On every edit, rowan 0.16 rebuilds each node from the edited line up to the document's root, copying each one's list of children. An edit therefore takes time proportional to the number of lines in its section plus the number of sections and of lines before the first section, and avoiding that needs a different tree library. The `Editor` rustdoc documents the cost and recommends building large documents as text and parsing them once. |
| TigerStyle, Naming | "Do not abbreviate variable names." | In `src/`, the library and its unit tests, clippy's `min_ident_chars` rejects one-letter names, and review catches other abbreviations. Integration tests and benchmarks keep short names where they are local and idiomatic, such as loop counters and closure parameters. |
| TigerStyle, Naming | "Use proper capitalization for acronyms", and put units in names. | Public names stay as they are, because renaming them breaks callers for little gain. Rust's API guidelines (C-CASE) write acronyms as words, `IniLang` rather than `INILang`, and SCREAMING_CASE `SyntaxKind` variants are the rowan convention. Units go in the documentation instead (`ParseError::offset` is a byte offset), and new APIs can add clearer names alongside old ones. |
| TigerStyle, Cache invalidation | Construct larger structs in place by passing an out pointer. | Rust has no stable in-place initialization, and the optimizer usually removes the copy of a returned value. TigerStyle's rule for arguments over 16 bytes is followed: `Copy` arguments of that size are passed by reference, Rust's form of `*const`. |
| TigerStyle, Style by the numbers | "Hard limit all line lengths, without exception, to at most 100 columns." | Exceptions: CHANGELOG.md, where release-plz writes each entry as one line (the pull request's title without its type, then a link to the pull request), so 9 of the 15 lines it wrote for 0.4.0 exceed 100 columns, and curated entries keep that form; `tests/fixtures/`, third-party files whose bytes are the test; `tests/snapshots/`, generated by insta; and Markdown table rows and link definitions, which cannot be wrapped. Prose and code are wrapped. |
| TigerStyle, Dependencies | A "zero dependencies" policy. | rowan is the crate's foundation; with its four dependencies, it is all that users compile. Dev-dependencies never reach users: rust-ini and configparser are the independent oracles of the differential tests, insta reviews snapshot changes, and the benchmarks run on criterion and compare against ini_core. Replacing criterion with a std-only benchmark harness is planned. |
| TigerStyle, Tooling | Write scripts in the primary language. | `.github/scripts/check_scaling.py` needs each child's CPU time and peak memory from `wait4`, which Rust's standard library does not expose: a port would need unsafe FFI, which the package denies, or procfs with 10 ms resolution. `.github/scripts/seed_fuzz_corpus.py` has no such need, and porting it to Rust is planned. |
| TigerStyle, Tooling | Write CI glue in the primary language. | The CI gates on coverage and mutation results are short jq programs over JSON output, and jq is preinstalled on the runners. In Rust, they would need a JSON parser or hand-written parsing. |
| SQLite §2 | Run the tests under every compile-time configuration. | ini-edit has no cargo features and no custom cfg other than `coverage_nightly`, so a feature matrix would test nothing new. CI varies what does change the build: debug and release, the minimum supported Rust version, Linux, macOS and Windows, a native 32-bit (i686) run of the whole suite, and Miri on a 32-bit and a big-endian target; see the second §8.6 row. |
| SQLite §3.1 | Out-of-memory tests that fail the Nth allocation. | A failed allocation aborts a Rust program (`handle_alloc_error`), and neither ini-edit nor rowan has a fallible-allocation API, so there is no recovery path to test. |
| SQLite §3.2 to §3.4 | I/O error, crash and compound-failure tests. | ini-edit performs no I/O and keeps no persistent state: the editor returns the new text as a `String`. There is no write to tear, no journal and no recovery to verify. |
| SQLite §4 | Fuzz at SQLite's scale, soak for hours before each release, and keep the whole learned corpus. | A pure in-memory INI library does not need that budget: each target fuzzes for 10 seconds per pull request, 60 seconds per push to `main` and 15 minutes every Sunday. `tests/fuzz_regressions/` holds a minimized copy of the learned corpus, made with `cargo fuzz cmin`, plus every crash or timeout reproducer, and every `cargo test` replays them, as SQLite's fuzzcheck replays its "interesting" cases. The full learned corpus, which changes with every campaign, stays in the CI cache. |
| SQLite §4.3, TigerStyle | Enumerate long hostile strings in the ordinary tests ("tests must test exhaustively"). | cargo-mutants reruns the ordinary tests for every mutant, so they stay small enough to finish in seconds. Longer enumerations belong in the ignored extended tests, which CI runs in release mode, next to the 41,371-string parser enumeration. |
| SQLite §7.2, §7.3 | The `ALWAYS()`, `NEVER()` and `testcase()` macros. | Rust's tooling already provides what they are for. llvm-cov does not count the panic arm of `assert!`, so defensive checks written as assertions do not lower coverage. 100% branch and MC/DC coverage, with no surviving mutant (cargo-mutants includes boundary mutants such as `<` into `<=`), forces the cases that `testcase()` exists to force, and Rust has no coverage hook to implement `testcase()` itself. Data-dependent limits get explicit boundary tests instead. |
| SQLite §7.6 | Mark branches that only affect speed (`OPTIMIZATION-IF-TRUE`) so that mutation testing skips them. | No mutant is skipped. A mutant that makes a loop run forever is caught by the 30-second timeout, and skipping such mutants would weaken the zero-survivor gate. |
| SQLite §8.2 to §8.5 | Valgrind, the memsys2 allocator, mutex asserts and journal tests. | Miri runs the library tests and reports undefined behavior and leaks, including in rowan's unsafe code, and the fuzz targets run under AddressSanitizer. There is no custom allocator to instrument, no threads to check (rowan's trees use non-atomic reference counts, so the editor is neither `Send` nor `Sync`), and no journal. |
| SQLite §8.6 | Rerun the tests with `-ftrapv`, `-fsanitize=undefined` and both `char` signednesses. | Safe Rust has no undefined signed overflow and no `char` signedness, and the package denies unsafe code. Debug tests check integer overflow, and Miri checks for undefined behavior in dependencies. |
| SQLite §8.6 | Run the whole suite on 32-bit and big-endian systems for every change. | Under Miri, that is too slow for every pull request: in one measurement, 12 editor tests took 469 seconds per target. Every pull request instead runs the whole suite natively on i686 and, under Miri, the lexer tests and the lexer and diagnostic contracts on i686 and s390x (big-endian). |
| SQLite §10 | Run the release checklist by hand, item by item. | Every item of the checklist in TESTING.md is an automated CI job, and for a single maintainer, re-running half-hour suites by hand would add toil and no signal. The human stays in the loop where SQLite puts them, reviewing results: release-plz opens the release pull request, and the maintainer reads its changelog and checks before merging it. |
| SQLite §11 | More static analyzers, or clippy's whole `restriction` group. | SQLite reports that static analysis found few bugs and that "more bugs have been introduced into SQLite while trying to get it to compile without warnings than have been found by static analysis". rustc, clippy and rustdoc with warnings denied, plus cargo-deny, are the static analysis here. |
| Process | Approval by a second person before every merge. | ini-edit has one maintainer, so the ruleset requires no approvals. The deliberate human checkpoint is the release pull request. |
| Process | Require branches to be up to date before merging, or use a merge queue. | With stacked pull requests and squash merges, re-running every pull request after each merge would serialize one maintainer's work. CI runs again on `main` after every merge, which catches semantic conflicts between merged changes. |
| Process | Pin the stable toolchain with `rust-toolchain.toml`. | Floating stable finds new rustc and clippy breakage early, in the Sunday runs, and every CI action is pinned to a commit. The minimum supported Rust version and the pinned nightlies are the reproducible baselines. A Sunday run can turn red because of a new lint; that is accepted. |

[tigerstyle]: https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md
[sqlite]: https://sqlite.org/testing.html
