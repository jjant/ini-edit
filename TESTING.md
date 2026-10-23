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

Before switching between the two nightlies, delete `target/llvm-cov-target`
or run `cargo llvm-cov clean` without `--workspace`. The two nightlies' cargo
put the instrumented test binary in different directories, so
`cargo llvm-cov clean --workspace`, and the partial clean that every
`cargo llvm-cov` run starts with, leave the other nightly's binary in place.
The report then also counts that binary's copy of the code, which never ran,
and falls below 100%. CI is not affected, because each job starts from a clean
checkout.

## Editor guarantees

Every editor test layer checks the guarantees the `editor` module documents.
After every call, successful or not, the editor's tree equals a parse of its
text with its parser options (C1); a call that returns `Err` leaves the text
and the tree unchanged (C2); a successful call changes only the bytes it
documents, so an insertion adds one contiguous run of bytes and a removal
deletes one (C3); and no caller input makes the editor panic (C4). Debug builds
also check inside the editor, after every edit, that C1 holds and that the edit
added or removed exactly the body lines it documents, so every debug test,
coverage run, mutation run, and fuzz build checks them.

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
- As in SQLite's malformed-database tests, the fixtures are also corrupted one
  change at a time. Every character of every fixture is deleted, replaced by
  each other character of a 14-character alphabet (CR, LF, every delimiter,
  space, tab, a key character, `é`, and U+FEFF), and has each alphabet
  character inserted before it (and after the last one). Each of the
  3,985,991 corrupted documents is parsed under all four parser-option
  combinations. It must be lossless, keep one node per line (an entry whose
  value continues over several physical lines is one line), and report
  diagnostics in order at character boundaries. Its diagnostics must
  also depend only on the text before them, on both sides of the edit: those
  before the edited line are exactly the diagnostics of parsing the unchanged
  lines before it, and dropping the lines after the edited line keeps every
  diagnostic up to its end. The five small fixtures are corrupted whole. The
  129 KB Gitea example keeps four lines on either side of each edit, because
  corrupting all of it would parse half a terabyte per option set. Most edits
  land in comments, values, or whitespace: 442,328 of the documents (11%)
  report other diagnostic messages than the unchanged text under some option
  set. Ordinary tests run a sample of 5,332 documents, with one line of
  context: every deletion, and every inserted or substituted CR, in the
  systemd unit, which has a continued value, and every deletion in the Git
  configuration, where deleting a `#` turns the comment above a section
  header into a key.
- Boundary decision tables vary line endings, separators, whitespace, comments,
  malformed input, empty input, unterminated input, and editor indices.
- Regression tests preserve every previously discovered bug.
- An independent ordered-entry model exhausts short edit sequences and checks
  both the live AST and the reparsed output after every operation. Following
  [TigerStyle's paired assertions](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md),
  successful serialization alone is not evidence that edits preserved meaning.
- Mutation testing requires zero surviving source mutations. A mutation that
  stops a lexer or parser loop from advancing fails that loop's progress
  assertion at once (see [Assertions](#assertions)), so a timeout in the lexer
  or parser points to a loop without one and is worth investigating.
  CI prints caught mutations as well as timeouts and publishes a count table
  in the job summary. Without `--caught`, cargo-mutants hides ordinary test
  failures, which can make a healthy run appear to consist only of timeouts.
- Five libFuzzer targets independently stress round trips and
  arbitrary editor operation sequences: every public method of `Editor`,
  `SectionEditor`, and `EntryEditor` with arbitrary strings, positions, and
  ranges, each call checked for C1, C2, C3, and the
  same result, text, and tree on an editor reopened before it, plus the
  identity property of empty insertions.
  `cargo fuzz` builds them with AddressSanitizer, debug assertions, and
  overflow checks, and libFuzzer checks for leaks with LeakSanitizer.
  The round-trip target also requires every line, including malformed ones,
  to be exactly one node that owns only its own terminator, because editor
  positions address those nodes; line breaks may occur only in line break
  tokens and, after a continuation backslash, in values. Its diagnostics must
  depend only on the text before them: parsing the input without its last
  line reports exactly the diagnostics that precede that line. A final line
  break belongs to the last line, so a file that ends with one still loses a
  line.
  The reload target applies the same edits with and without reopening
  the document between operations, comparing results, output, and complete
  syntax trees. Its shared oracle also runs every pair of its 25 operations on
  12 documents (7,500 sequences) in normal tests, malformed, CR-only, and
  mixed CR/LF documents included; an `#[ignore]` test runs every triple
  (see below).
  Every target is a thin wrapper around a `check(&[u8])` oracle in
  `tests/support/`, so ordinary tests compile, lint, and run the same code,
  and every committed fuzz input is replayed in each `cargo test` (see
  [Fuzz corpus and reproducibility](#fuzz-corpus-and-reproducibility)).
- An independent identity-based model checks up to 128 edits per fuzz input.
  It tracks duplicate keys, continued values, bare keys, entries addressed by
  position and by key, and immutable file snapshots across deletion and
  recreation; hostile keys and values must be refused with the model
  unchanged. Every step checks live values, reparsed values, the complete
  syntax tree, and saved views. Ordinary tests also run 24 deterministic
  sequences of 32 edits across parsing and spacing options.
- A bidirectional hostile-input oracle renders hostile strings at ten sites
  (keys, values, section names, raw lines) of hostile documents by string
  concatenation, and decides from a full parse of that text whether a reader
  would read the edit back. The editor must accept exactly those edits and
  write exactly the reference's bytes. Normal tests run 8,736 cases: every
  string of up to two of ten pieces, in five documents, under two option sets.
  `#[ignore]` tests run every string of up to two, and of up to three, of all
  16 pieces, in 14 documents, under every option set and spacing (see below).
  A random walk applies hostile operations to hostile documents with the same
  per-call checks as the editor fuzz target.
- A generated-document model constructs source text and expected meaning
  together, without parsing to obtain the expected result. It varies section
  and key names, duplicates, Unicode, BOMs, separators, indentation, comments,
  line endings, continuations, and missing final newlines. Nine edit operations
  check the entire output byte for byte, including all untouched text, and
  compare live/reparsed values and syntax trees. Appended entries must use the
  first line break in the document, predicted from the model's own text.
  Normal tests run 216 cases
  covering all operations, spacing policies, and parser options; the
  `generated_edits` fuzz target explores more documents using the same oracle.
  Deleting a line between a CR line break and an LF blank line keeps every
  byte; the editor's tree then reads the two as one CRLF, as a reader does.
- Bounded exhaustive tests enumerate all 41,371 strings of zero through four
  scalars from a 14-character alphabet containing delimiters, whitespace,
  Unicode, and a BOM. Every input runs through all four parser-option
  combinations and through the round-trip fuzz oracle. Checks include
  losslessness, token partitioning, one node per line (a continued entry is
  one line), diagnostic boundaries/locations (a leading BOM is not a
  column), diagnostics that do not change when the last line is removed, and
  an independent semantic expectation for a restricted single-line subset.
- Large-input tests exercise a 4 MiB value, large Unicode identifiers,
  32,768 continuations, and 65,536 malformed lines. A separate Linux child
  process measures CPU time and peak RSS for growing entry lists, errors,
  continuations, and values. Broad growth bounds catch gross superlinear
  regressions; they are not a proof of complexity or an absolute latency SLA.
- Stable tests and doctests run on Linux, macOS, and Windows, and on 32-bit
  Linux.

The bounded exhaustive test, the corruption shards, and the large-input tests
are ignored in ordinary test runs and mutation testing; the corruption sample
runs in both. The `extended-tests` CI job runs them explicitly in release mode
and saves the CPU/memory measurements as an artifact. The tests keep the
checks of a debug build, which a plain release build compiles out:
`debug_assert!`, code under `cfg(debug_assertions)`, and arithmetic overflow
checks. A failed check panics and fails its test. The scaling probe is built
with the release profile as it ships, so that its timings measure the shipped
code:

```sh
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
  cargo test --release --test exhaustive_parser --test stress -- --include-ignored
cargo build --release --example scaling_probe
python3 .github/scripts/check_scaling.py
```

The editor's extended oracles in `tests/fuzz_regressions.rs` are ignored too:
the reload triples, the full two- and three-piece hostile-input products, and
a random walk over 60,000 seeds. No CI job runs them yet, so run them by hand
before changing the editor; together they take under two minutes:

```sh
cargo test --release --test fuzz_regressions -- --ignored
```

The corruption test runs as eight tests, `corruption::shard_0` through
`corruption::shard_7`. Each one corrupts every eighth line of every fixture, so
the shards cost about the same and the test harness runs them in parallel. With
four test threads they take about a minute in release mode. Each shard asserts
how many documents it corrupted, and the ordinary test
`corruption::every_line_is_corrupted` counts the edits of every line, without
making them, to check those numbers and their total of 3,985,991. A shard that
skipped a line would therefore fail.

The scaling script uses three fresh processes at each size, compares median
CPU/RSS measurements, and terminates an individual probe after 45 seconds.
It uses per-child resource accounting so time waiting for another process to
use the CPU is excluded.

A growth ratio cannot see a parser that needs twice the memory at every size,
so three dense workloads also have an absolute bound on one parse of their
largest input: 4 MiB of blank lines, and 2 MiB each of lines holding only `=`
(an error with its own message every two bytes) and only `[` (two errors every
two bytes). The check uses one parse in a fresh process because parsing again
in the same process reuses a fragmented heap: at some sizes, eight parses of
`=` lines peak 50% higher than one.

Measured that way on Linux x86-64, with the input string included, one parse
peaks at 66 bytes of RSS per input byte for blank lines, 85 for `=` lines, and
110 for `[` lines. No line of up to three characters drawn from `[]=:;#\`, a
letter, space, tab, and CR needs more, with any parse options. Short valid
lines such as `a=` or `[a]` need about 70, `k=v` lines 59, and entries such as
`key1=value1` 25. The bounds are 128 bytes per input byte for `=` and `[`
lines, the sizing rule below, and 96 for blank lines, each plus 4 MiB, twice
what the probe needs for a one-unit input. A regression that doubles the
memory of a line takes the three workloads to 130, 169, and 218, at least 30%
above their bounds, so the check fails.

`parse` frees the lexer's tokens and the tree builder's buffers before it
returns, but the returned `Parse` keeps the tree and every diagnostic, each
with its own message string. A `Parse` that stays alive holds 16 bytes per
input byte for blank lines (a quarter of the peak), 28 for `k=v` lines, 48 for
`=` lines, and 72 for `[` lines (two thirds of the peak). To parse untrusted
input, allow about 128 bytes of memory per input byte while `parse` runs, and
up to 1.5 times that in a process that parses many documents. A `Parse` that
the caller keeps holds up to 76 bytes per input byte, the most found (for
lines such as `[]x`), for as long as it lives.

Distinct-entry measurements use 65,536, 131,072, and 262,144 entries. Smaller
16K–64K windows showed steep growth in the unchanged parser that varied across
CI runners and produced a false alarm on the same source tree. The larger
window checks sustained growth with the same ratio bound; other workload
families use 16,384, 32,768, and 65,536 units.

## Assertions

The lexer and the parser check their own invariants, following TigerStyle
("assert all function arguments and return values, pre/postconditions and
invariants") and SQLite's use of `assert()` for "function preconditions and
postconditions and loop invariants". A broken invariant panics where it is
detected; it never gets a silent fallback. These assertions play the role of
SQLite's `ALWAYS()` and `NEVER()`, on top of what each test layer's own oracle
compares.

Which of them a run checks depends on its build; the tiers are listed below.
Every build checks the always-on `assert!`s. Debug test runs, coverage,
cargo-mutants and the fuzz targets also check the `debug_assert!`s and the
O(document) checks, and Miri checks all but the O(document) ones. The
`extended-tests` CI job runs the bounded exhaustive inputs, the corruption
shards and the large-input tests in plain `--release`, so there they check
only the always-on tier. The last item below shows how to run them with the
others.

`parse_with` pairs its postconditions with the lexer's checks: every token is
consumed once, and the offsets and the tree cover exactly the input. In debug
builds it also runs `debug_assert_line_nodes`, which checks that every
physical line is one node owning only its own line break; `Editor::finish`
and `Editor::file` check the same invariant on the edited tree. Between
those, the stateful and generated-edit models compare the live green tree
with a fresh parse after every edit, which implies the invariant for every
edit they make.

- O(1) checks use `assert!` and stay on in release builds: loop progress,
  non-empty tokens, the shape of tokens and lines, and the postconditions of
  `parse_with`.
- On the per-line or per-token hot path, a check that only restates what is
  already established uses `debug_assert!`: the first character a line was
  dispatched on, whitespace that was just eaten, or, in `Parser::bump`, the
  non-empty token that the lexer's always-on check guarantees. As `assert!`,
  the lexer's five such checks cost about 15% of release lexing time on short
  lines, and the parser's token check 2% to 9% of parse time.
- Checks that read the document again, such as summing the token lengths,
  scanning each token for line breaks, or comparing the tree's text with the
  input, are compiled only under `#[cfg(all(debug_assertions, not(miri)))]`.
  Debug tests, coverage, cargo-mutants and cargo-fuzz (which enables debug
  assertions by default) run them. Miri looks for undefined behavior, not
  logic errors, and skipping them keeps its run time from multiplying.
- Every loop that consumes input has a fixed upper bound or asserts that each
  iteration advances, so a step that stops advancing panics at once instead of
  hanging.
- Assertion conditions contain no `&&` or `||`. Coverage counts each operand
  as a branch, and the failing side never runs. Split `assert!(a && b)` into
  `assert!(a); assert!(b);`, and write an implication as
  `if !a { assert!(b); }`, whose arms ordinary inputs both exercise.
- Runtime assertions carry no message. The MC/DC toolchain counts a message,
  even a string literal, as a region that runs only on failure. A comment
  above the assertion explains it; `assert!` reports its condition and
  `assert_eq!` both values. A message is fine where a `should_panic` test trips
  the assertion, as in `kind_from_raw`.
- Assertions stay inline, never in a helper fn. The helper's "replace with
  ()" mutant would delete the check, and a helper that wraps a loop step would
  turn that mutant into a hang.
- A compile-time proof keeps its whole computation inside `assert!`, because
  cargo-mutants does not mutate macro arguments: a mutated loop in a `const`
  block can make the proof vacuous and still compile.
- A checker fn or a bound counter gets a `#[should_panic(expected = ...)]`
  test that trips it; otherwise its mutants survive. So does a precondition
  that a test can break by calling the function directly, so that deleting the
  assertion fails a test. A test of a debug-only check carries the same `cfg`:
  `cargo test --release` drops the check, so the test would fail, or would not
  compile if it calls a debug-only fn.
- A `should_panic` test that reaches its assertion through a generic fn also
  runs that fn's other paths. The MC/DC job's llvm-cov reports a generic fn by
  its best single instantiation, not by the union of its instantiations.
- A `pub` fn whose assertions check internal invariants carries
  `#[expect(clippy::missing_panics_doc, reason = ...)]`. A panic that a caller
  can cause gets a `# Panics` section instead.
- A predicate that lexes caller text lexes it in a mid-document position, so
  that the leading-BOM rule cannot apply to it.
- `--release` compiles out the debug-only checks. To run the exhaustive,
  corruption and large-input suites with them at release speed:

  ```sh
  CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true \
  CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
    cargo test --release --test exhaustive_parser --test stress -- --include-ignored
  ```

## Fuzz corpus and reproducibility

`tests/fuzz_regressions/<target>/` holds committed fuzz inputs: the
`cargo fuzz cmin` minimization of the learned corpus plus every crash and
timeout reproducer. Like SQLite's fuzzcheck, `tests/fuzz_regressions.rs` runs
each file through its target's oracle in every `cargo test`, so past findings
are checked on every platform and on the MSRV without nightly or libFuzzer.
An input fails if it panics or runs for more than 10 seconds, the timeout CI
fuzzes with, and the failure names the input file. The current 3,732 inputs
(155 KiB) replay in a few seconds in a debug build. The test also fails if a
fuzz target lacks committed inputs or a replay test. The same test binary runs
the fixed sweeps of the shared oracles, because cargo-mutants rebuilds every
test binary for every mutant. The published package excludes the inputs, this
test, and the oracles that only it and the fuzz targets use.

A fuzz-found bug is fixed only when its reproducer is committed:

1. Minimize the crash or timeout input with the limits CI fuzzes with.
   `cargo fuzz tmin` sets no timeout of its own, and libFuzzer's default of
   1,200 seconds would never count a slow input as a failure:

   ```sh
   cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/<file> -- \
     -timeout=10 -rss_limit_mb=1024
   ```

2. In the pull request that fixes the bug, commit the minimized file to
   `tests/fuzz_regressions/<target>/` under a descriptive name such as
   `crash-empty-section-insert`, and add a named regression test that states
   the bug through the public API.
3. Check that the replay fails without the fix and passes with it.

Committed bytes mean whatever the target's decoder makes of them. When a
change to a target's input layout alters that meaning, refresh the minimized
inputs and re-create any reproducer that no longer reaches its bug. A libFuzzer
merge names its output files by their SHA-1, so a refresh replaces only the
40-hex-digit names and keeps every reproducer. Keep the directory under about
2 MB. Run this from the repository root; it also merges what local fuzzing
learned in `fuzz/corpus/<target>`, if anything. `cargo fuzz cmin` exits with
status 0 even when its merge fails, so the snippet runs the merge itself and
stops before deleting anything if the merge fails or keeps no input:

```sh
(
  set -eu
  target=editor_stateful max_len=515 # 4096 for generated_edits, else 16384
  committed=tests/fuzz_regressions/$target learned=fuzz/corpus/$target
  inputs=$(mktemp -d) merged=$(mktemp -d)
  trap 'rm -r "$inputs" "$merged"' EXIT
  # True for a merged input's name: 40 lowercase hex digits.
  is_hash() { case $1 in *[!0-9a-f]*) return 1 ;; esac; [ ${#1} -eq 40 ]; }
  for file in "$committed"/*; do
    if is_hash "${file##*/}"; then cp "$file" "$inputs/"; fi
  done
  mkdir -p "$learned"
  # libFuzzer merges every later directory into the first one.
  cargo +nightly fuzz run "$target" "$merged" "$inputs" "$learned" -- \
    -merge=1 -max_len="$max_len" -timeout=10 -rss_limit_mb=1024
  [ -n "$(ls "$merged")" ] # the merge kept at least one input
  for file in "$committed"/*; do
    if is_hash "${file##*/}"; then rm "$file"; fi
  done
  cp "$merged"/* "$committed/"
)
```

`.gitattributes` marks these files `binary`. Under the repository's
`text=auto eol=lf` default, Git would rewrite CRLF line endings in 17 of the
current inputs.

CI restores the learned corpus, then adds deterministic boundary cases,
real-world fixtures, and generated model seeds without deleting existing
inputs. Successful and failing campaigns on `main` save their corpus for
future runs; pull requests consume that cache but do not update the main
corpus. The cache key depends on no file, so dependency and lockfile updates
keep the learned corpus. GitHub may evict caches, so the reproducible seeds
remain useful.
Crash and timeout inputs are uploaded separately and retained for 90 days,
which is still finite: that is why reproducers must be committed.

Each fuzz input has a 10-second timeout and each target has a 1 GiB RSS limit.
Input lengths are bounded independently: 16,384 bytes for the original three
targets, 515 bytes for the stateful model, and 4,096 bytes for the
generated-document model. Local reproduction, also starting from the committed
inputs (libFuzzer reads every corpus directory but adds inputs to the first):

```sh
python3 .github/scripts/seed_fuzz_corpus.py
cargo +nightly-2026-09-17 fuzz run generated_edits fuzz/corpus/generated_edits \
  tests/fuzz_regressions/generated_edits -- \
  -max_len=4096 -max_total_time=120 -timeout=10 -rss_limit_mb=1024
# Replay a downloaded crash/timeout artifact:
cargo +nightly-2026-09-17 fuzz run generated_edits path/to/reproducer
```

## As-delivered and dynamic checks

Coverage is a meta-test of the test suite, not a substitute for testing the
shipping configuration. CI separately runs the complete suite on stable Rust
in debug and release modes, on the minimum supported Rust version, and on
macOS and Windows. `cargo test --all-targets` runs no doctests, so each of
these jobs also runs `cargo test --doc`.

The `i686` job runs the complete suite and the doctests natively on 32-bit x86
Linux, where `usize` and pointers are 32 bits wide. To run it on Debian or
Ubuntu:

```sh
sudo apt-get install gcc-multilib
rustup target add i686-unknown-linux-gnu
cargo test --target i686-unknown-linux-gnu --all-targets --all-features
cargo test --target i686-unknown-linux-gnu --doc --all-features
```

The `package` job tests the crate that `cargo publish` uploads rather than the
repository. First it compares the crate's file list with the repository: the
crate must contain every tracked file except the ones that only this
repository uses, and none of those. `package.exclude` in `Cargo.toml` leaves
them out, and the job keeps its own list of them, so a test or the data it
reads can leave the crate only through a change to both lists. Then the job
builds the `.crate` file with `cargo package`, extracts it outside the
checkout, and runs the shipped tests and doctests there, so a test fails if it
reads a file the crate leaves out, as the snapshot tests of 0.4.0 did. The
tests must build in an empty target directory: cargo dates every file in a
`.crate` 2006-07-24, so it treats an earlier build of the same version as up
to date and reruns its old test binaries. To build and test the packaged crate
locally:

```sh
dir=$(mktemp -d)
cargo package --locked --target-dir "$dir/target"
tar -xzf "$dir/target/package/ini-edit-<version>.crate" -C "$dir"
cd "$dir/ini-edit-<version>"
cargo test --locked --target-dir target --all-targets --all-features
cargo test --locked --target-dir target --doc --all-features
```

The `sanitizers` job runs the complete suite and the doctests under
AddressSanitizer and LeakSanitizer (`-Zsanitizer=address` on
`nightly-2026-09-17`). AddressSanitizer stops a test binary at the first
out-of-bounds or use-after-free access in the instrumented code, `rowan`'s
unsafe code included. LeakSanitizer fails a test binary that exits with memory
nothing can reach any more, such as a `rowan` tree that a forgotten handle
keeps alive. The fuzz targets already run under both, but only with the
oracles in `tests/support/`; this job adds every other test, among them the
unit, snapshot, real-world, differential, and editor tests.

Neither job rebuilds the standard library with the sanitizer (`-Zbuild-std`).
Generic code such as `Vec<T>` is compiled with the crate and checked; the
precompiled rest is checked only through the calls that AddressSanitizer
intercepts, such as `malloc`, `free`, and `memcpy`. AddressSanitizer checks no
aliasing rules either, so it does not replace the Stacked Borrows checks that
are disabled below.

To run the job locally, pass `--target`: with it, `RUSTFLAGS` reach only the
code under test and not the proc macros of the dev-dependencies, which `rustc`
refuses to build with a sanitizer. `allow_addr2line=1` lets a report name
source lines through binutils when `llvm-symbolizer` is not installed:

```sh
(
  export RUSTFLAGS=-Zsanitizer=address RUSTDOCFLAGS=-Zsanitizer=address
  export ASAN_OPTIONS=detect_leaks=1:allow_addr2line=1
  cargo +nightly-2026-09-17 test --target x86_64-unknown-linux-gnu --all-targets --all-features
  cargo +nightly-2026-09-17 test --target x86_64-unknown-linux-gnu --doc --all-features
)
```

Miri runs all lexer unit tests and independent lexer/diagnostic contracts with
borrow checking enabled. These tests do not construct `rowan` trees. They run
on x86_64, on 32-bit `i686-unknown-linux-gnu`, and on big-endian
`s390x-unknown-linux-gnu`: Miri runs the code as compiled for the target it is
given, so neither target needs its hardware. Running the whole library on each
of them would repeat the full-library Miri run below once per target, so the
native `i686` job checks 32-bit for the whole suite, and big-endian runs only
the lexer and its contracts.

The full library suite also runs under Miri, with its Stacked Borrows
provenance model temporarily disabled because `rowan 0.16.1` triggers the
known upstream [rowan issue #192](https://github.com/rust-analyzer/rowan/issues/192).
Other default Miri checks remain enabled. A compatibility investigation on
2026-10-02 reproduced failures with both Stacked Borrows and Tree Borrows using
`nightly-2026-09-17`. `rowan 0.17.0` is not a drop-in fix: it removes
`clone_for_update`, `detach`, and `splice_children`, and upgrading would change
the `rowan` types exposed by this crate's public API. Full borrow checking
therefore remains an explicit limitation, not a passing guarantee.

Recheck the strict paths locally without `MIRIFLAGS` overrides. Miri builds
each target's standard library itself, so the targets need no `rustup target
add`:

```sh
for target in x86_64-unknown-linux-gnu i686-unknown-linux-gnu s390x-unknown-linux-gnu; do
  cargo +nightly-2026-09-17 miri test --target "$target" --lib lexer::tests::
  cargo +nightly-2026-09-17 miri test --target "$target" --test exhaustive_parser \
    lexer_and_diagnostic_contracts -- --exact
done
```

Clippy, rustdoc, and `cargo deny` cover static analysis, advisories
(including unsound and unmaintained crates), yanked packages, licenses,
duplicate dependencies, and unexpected dependency sources.

The full matrix also runs every Sunday. Scheduled fuzzing gives each target 15
minutes instead of the short pull-request smoke-test budget, and so does every
push to `main` whose version is not released yet (see the release checklist).

Allocation-failure, filesystem I/O-failure, crash-recovery, and concurrency
tests from SQLite's strategy do not map directly to this crate: `ini-edit`
performs no persistent I/O, owns no allocator abstraction, and exposes no
shared mutable state. Fuzzing malformed text and mutation sequences is the
relevant anomaly-testing layer here.

## Release checklist

CI publishes a version to crates.io only from a commit on which it ran every
item below and each one passed. Merging the release pull request pushes the
version bump to `main`, and the CI run for that push runs every job below on
the merged commit. Its `required` job waits for all of them, however they end,
and fails unless each one succeeded. Only after `required` succeeds, and only
on a push to `main`, does the `release` job run `release-plz release`. That
publishes the version in `Cargo.toml` unless its `v<version>` tag exists or
crates.io already has it, and creates the tag after the upload.

Until that tag exists, every push to `main` fuzzes each target for 15 minutes,
as on Sundays, instead of 60 seconds. The 15-minute campaigns of item 7
therefore run on the commit that `release` publishes, and the release waits
for them: it starts about 80 minutes after the merge, when `fuzz` ends.

A failed or cancelled job stops the release. If a job failed for a reason
outside the commit, such as a flaky runner, re-running the failed jobs of that
run retries the release; otherwise the push of the fix publishes the version,
after its own 15-minute campaigns.

`required` also checks the wiring that makes `release` wait for every gate. It
fails if a job in `ci.yml` other than `release` is missing from its `needs`,
if a job other than `release` needs it, if the `if:` of `release` is not the
one it expects, or if another workflow runs on pushes, where `release` cannot
wait for it. On a pull request, `required` is the one check that stands for
all the others. No branch rule requires it yet, so wait for it to pass before
merging.

1. Stable debug, stable release, doctests, rustdoc, Clippy, and formatting
   (`check`), the stable tests and doctests on macOS and Windows (`macos-26`
   and `windows-2025`), and on 32-bit Linux (`i686`).
2. MSRV tests and doctests (`msrv`).
3. 100% functions, instantiations, lines, regions, branches, and MC/DC
   (`coverage / branches (100%)` and `coverage / MC/DC (100%)`).
4. Full-suite Miri with the documented upstream workaround (`miri (1/4)`
   through `miri (4/4)`), strict Miri for lexer and diagnostic contracts on
   x86_64, 32-bit i686, and big-endian s390x (`miri-strict`), and the complete
   suite under AddressSanitizer and LeakSanitizer (`sanitizers`).
5. Dependency audit and policy checks (`dependencies`).
6. Full mutation test (`mutation (1/4)` through `mutation (4/4)`).
7. All five fuzz targets (`fuzz`), for 15 minutes each on the published
   commit, as on Sundays.
8. Bounded exhaustive, fixture corruption, and large-input tests with debug
   assertions and overflow checks, and CPU/memory scaling tests
   (`extended-tests`).
9. A `fuzz/Cargo.lock` that matches the release version (`check`). `fuzz/` is
   a separate workspace, so the version bump does not reach its lockfile. The
   Release PR workflow commits
   `cargo update --manifest-path fuzz/Cargo.toml --package ini-edit` to the
   release pull request every time it opens or rewrites it. If that step
   fails, run the command and commit `fuzz/Cargo.lock` by hand.
10. The package check (`package`): the crate ships every tracked file except
    the repository-only ones, and its tests and doctests pass when run from
    the extracted `.crate` file.
