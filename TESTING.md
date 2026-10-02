# Testing strategy

`ini-edit` follows the layered testing ideas described in
[How SQLite Is Tested](https://sqlite.org/testing.html), adapted to a small,
pure, in-memory Rust library.

## Required coverage

Production code is measured separately from test implementation code. Unit
tests still run and exercise private invariants, but their bodies use
`#[coverage(off)]` while coverage instrumentation is active.

The coverage jobs execute one instrumented library test binary. Running every
integration-test binary in one LLVM report would compile the library once per
binary and count duplicate, unexecuted monomorphizations rather than additional
production behavior.

CI requires these thresholds; exact counts grow as production code changes:

| Metric | Covered |
|---|---:|
| Functions | 100% |
| Instantiations | 100% |
| Lines | 100% |
| Regions | 100% |
| Branches | 100% |
| MC/DC conditions | 100% |

LLVM's MC/DC instrumentation is currently tied to
`nightly-2025-06-01`. The other source and branch measurements use
`nightly-2026-09-17`. Compiler versions produce slightly different region
counts, so CI requires 100% independently in each report.

Run the same measurements locally:

```sh
cargo +nightly-2026-09-17 llvm-cov --lib --all-features
cargo +nightly-2026-09-17 llvm-cov --lib --all-features --branch
RUSTFLAGS='--cfg no_zerocopy_simd_x86_avx12_1_89_0' \
  cargo +nightly-2025-06-01 llvm-cov --lib --all-features --mcdc
```

## Independent test layers

- Unit tests cover internal lexer, parser, AST, and editor invariants.
- Integration and snapshot tests exercise only the public API.
- Differential tests generate 1,024 documents in the shared INI subset and
  compare their values and sections against a separate model, `rust-ini`, and
  `configparser`, including all four `ini-edit` parser-option combinations.
  Dialect-specific quotes, escapes, duplicate keys, continuations, and inline
  comments stay outside that comparison so expected dialect differences do
  not become false bug reports.
- Real-world fixtures cover AWS, Git, Gitea, MySQL, PHP, and systemd syntax.
- Boundary decision tables vary line endings, separators, whitespace, comments,
  malformed input, empty input, unterminated input, and editor indices.
- Regression tests preserve every previously discovered bug.
- An independent ordered-entry model exhausts short edit sequences and checks
  both the live AST and the reparsed output after every operation. Following
  [TigerStyle's paired assertions](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md),
  successful serialization alone is not evidence that edits preserved meaning.
- Mutation testing requires zero surviving source mutations. Mutations that
  destroy lexer/parser progress are detected by a strict timeout.
  CI prints caught mutations as well as timeouts and publishes a count table
  in the job summary. Without `--caught`, cargo-mutants hides ordinary test
  failures, which can make a healthy run appear to consist only of timeouts.
- Five libFuzzer targets independently stress round trips and
  arbitrary editor operation sequences, including reuse of section handles
  after deleting their headers and the identity property of empty insertions.
  The reload target applies the same valid edits with and without reopening
  the document between operations, comparing both output and complete syntax
  trees. Its shared oracle also exhausts 3,072 short sequences in normal tests.
- An independent identity-based model checks up to 128 edits per fuzz input.
  It tracks duplicate keys, continued values, bare keys, retained entry and
  section handles, detached objects, and immutable file snapshots across
  deletion and recreation. Every step checks live values, reparsed values,
  the complete syntax tree, and saved handles/views. Ordinary tests also run
  24 deterministic sequences of 32 edits across parsing and spacing options.
- A generated-document model constructs source text and expected meaning
  together, without parsing to obtain the expected result. It varies section
  and key names, duplicates, Unicode, BOMs, separators, indentation, comments,
  line endings, continuations, and missing final newlines. Eight edit operations
  check the entire output byte for byte, including all untouched text, and
  compare live/reparsed values and syntax trees. Normal tests run 192 cases
  covering all operations, spacing policies, and parser options; the
  `generated_edits` fuzz target explores more documents using the same oracle.
- Bounded exhaustive tests enumerate all 41,371 strings of zero through four
  scalars from a 14-character alphabet containing delimiters, whitespace,
  Unicode, and a BOM. Every input runs through all four parser-option
  combinations. Checks include losslessness, token partitioning, diagnostic
  boundaries/locations, and an independent semantic expectation for a
  restricted single-line subset.
- Large-input tests exercise a 4 MiB value, large Unicode identifiers,
  32,768 continuations, and 65,536 malformed lines. A separate Linux child
  process measures CPU time and peak RSS for growing entry lists, errors,
  continuations, and values. Broad growth bounds catch gross superlinear
  regressions; they are not a proof of complexity or an absolute latency SLA.
- Stable tests run on Linux, macOS, and Windows.

The exhaustive and large-input tests are ignored in ordinary test runs and
mutation testing. The `extended-tests` CI job runs them explicitly in release
mode and saves the CPU/memory measurements as an artifact:

```sh
cargo test --release --test exhaustive_parser --test stress -- --include-ignored
cargo build --release --example scaling_probe
python3 .github/scripts/check_scaling.py
```

The scaling script uses three fresh processes at each size, compares median
CPU/RSS measurements, and terminates an individual probe after 45 seconds.
It uses per-child resource accounting so time waiting for another process to
use the CPU is excluded.

## Fuzz corpus and reproducibility

CI restores the learned corpus, then adds deterministic boundary cases,
real-world fixtures, and generated model seeds without deleting existing
inputs. Successful and failing campaigns on `main` save their corpus for
future runs; pull requests consume that cache but do not update the main
corpus. GitHub may evict caches, so the reproducible seeds remain useful.
Crash and timeout inputs are uploaded separately and retained for 14 days.

Each fuzz input has a 10-second timeout and each target has a 1 GiB RSS limit.
Input lengths are bounded independently: 16,384 bytes for the original three
targets, 515 bytes for the stateful model, and 4,096 bytes for the
generated-document model. Local reproduction:

```sh
python3 .github/scripts/seed_fuzz_corpus.py
cargo +nightly fuzz run generated_edits -- \
  -max_len=4096 -max_total_time=120 -timeout=10 -rss_limit_mb=1024
# Replay a downloaded crash/timeout artifact:
cargo +nightly fuzz run generated_edits path/to/reproducer
```

## As-delivered and dynamic checks

Coverage is a meta-test of the test suite, not a substitute for testing the
shipping configuration. CI separately runs the complete suite on stable Rust
in debug and release modes and on the minimum supported Rust version.

Miri runs all lexer unit tests and independent lexer/diagnostic contracts with
borrow checking enabled. These tests do not construct `rowan` trees.

The full library suite also runs under Miri, with its Stacked Borrows
provenance model temporarily disabled because `rowan 0.16.1` triggers the
known upstream [rowan issue #192](https://github.com/rust-analyzer/rowan/issues/192).
Other default Miri checks remain enabled. A compatibility investigation on
2026-10-02 reproduced failures with both Stacked Borrows and Tree Borrows using
`nightly-2026-09-17`. `rowan 0.17.0` is not a drop-in fix: it removes
`clone_for_update`, `detach`, and `splice_children`, and upgrading would change
the `rowan` types exposed by this crate's public API. Full borrow checking
therefore remains an explicit limitation, not a passing guarantee.

Recheck the strict paths locally without `MIRIFLAGS` overrides:

```sh
cargo +nightly-2026-09-17 miri test --lib lexer::tests::
cargo +nightly-2026-09-17 miri test --test exhaustive_parser \
  lexer_and_diagnostic_contracts -- --exact
```

Clippy, rustdoc, `cargo audit`, and
`cargo deny` cover static analysis, advisories, yanked packages, licenses,
duplicate dependencies, and unexpected dependency sources.

The full matrix also runs every Sunday. Scheduled fuzzing gives each target 15
minutes instead of the short pull-request smoke-test budget.

Allocation-failure, filesystem I/O-failure, crash-recovery, and concurrency
tests from SQLite's strategy do not map directly to this crate: `ini-edit`
performs no persistent I/O, owns no allocator abstraction, and exposes no
shared mutable state. Fuzzing malformed text and mutation sequences is the
relevant anomaly-testing layer here.

## Release checklist

Before release, all required GitHub checks must be green:

1. Stable debug, stable release, doctests, rustdoc, Clippy, and formatting.
2. MSRV.
3. 100% functions, instantiations, lines, regions, branches, and MC/DC.
4. Full-suite Miri with the documented upstream workaround, plus strict Miri
   for lexer and diagnostic contracts.
5. Dependency audit and policy checks.
6. Full mutation test.
7. All five fuzz targets.
8. Bounded exhaustive, large-input, and CPU/memory scaling tests.
