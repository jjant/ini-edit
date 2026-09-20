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

Current results:

| Metric | Covered |
|---|---:|
| Functions | 137 / 137 (100%) |
| Instantiations | 153 / 153 (100%) |
| Lines | 862 / 862 (100%) |
| Regions | 919 / 919 (100%) |
| Branches | 174 / 174 (100%) |
| MC/DC conditions | 46 / 46 (100%) |

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
- Real-world fixtures cover AWS, Git, Gitea, MySQL, PHP, and systemd syntax.
- Boundary decision tables vary line endings, separators, whitespace, comments,
  malformed input, empty input, unterminated input, and editor indices.
- Regression tests preserve every previously discovered bug.
- Mutation testing requires zero surviving source mutations. Mutations that
  destroy lexer/parser progress are detected by a strict timeout.
- Two structure-aware libFuzzer targets independently stress round trips and
  arbitrary editor operation sequences.
- Stable tests run on Linux, macOS, and Windows.

## As-delivered and dynamic checks

Coverage is a meta-test of the test suite, not a substitute for testing the
shipping configuration. CI separately runs the complete suite on stable Rust
in debug and release modes and on the minimum supported Rust version.

Miri checks the library test suite for undefined behavior. Its Stacked Borrows
provenance model is temporarily disabled because `rowan 0.16.1` triggers the
known upstream [rowan issue #163](https://github.com/rust-analyzer/rowan/issues/163);
all other Miri checks remain enabled. Clippy, rustdoc, `cargo audit`, and
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
4. Miri.
5. Dependency audit and policy checks.
6. Full mutation test.
7. Both fuzz targets.
