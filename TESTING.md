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
- As in SQLite's malformed-database tests, the fixtures are also corrupted one
  change at a time. Every character of every fixture is deleted, replaced by
  each other character of a 14-character alphabet (CR, LF, every delimiter,
  space, tab, a key character, `é`, and U+FEFF), and has each alphabet
  character inserted before it (and after the last one). Each of the
  3,985,991 corrupted documents is parsed under all four parser-option
  combinations. It must be lossless, keep one node per physical line, and
  report diagnostics in order at character boundaries. Its diagnostics must
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
- Mutation testing requires zero surviving source mutations. Mutations that
  destroy lexer/parser progress are detected by a strict timeout.
  CI prints caught mutations as well as timeouts and publishes a count table
  in the job summary. Without `--caught`, cargo-mutants hides ordinary test
  failures, which can make a healthy run appear to consist only of timeouts.
- Five libFuzzer targets independently stress round trips and
  arbitrary editor operation sequences, including reuse of section handles
  after deleting their headers and the identity property of empty insertions.
  The round-trip target also requires every physical line, including
  malformed ones, to be exactly one node that owns only its own terminator,
  because editor line indices address those nodes. Its diagnostics must
  depend only on the text before them: parsing the input without its last
  line reports exactly the diagnostics that precede that line. A final line
  break belongs to the last line, so a file that ends with one still loses a
  line.
  The reload target applies the same valid edits with and without reopening
  the document between operations, comparing both output and complete syntax
  trees. Its shared oracle also exhausts 5,103 short sequences in normal tests,
  including a CR-only document and a raw line ending in CR that is inserted
  before a blank separator line.
  Every target is a thin wrapper around a `check(&[u8])` oracle in
  `tests/support/`, so ordinary tests compile, lint, and run the same code,
  and every committed fuzz input is replayed in each `cargo test` (see
  [Fuzz corpus and reproducibility](#fuzz-corpus-and-reproducibility)).
- An independent identity-based model checks up to 128 edits per fuzz input.
  It tracks duplicate keys, continued values, bare keys, retained entry and
  section handles, detached objects, and immutable file snapshots across
  deletion and recreation. Every step checks live values, reparsed values,
  the complete syntax tree, and saved handles/views. Ordinary tests also run
  24 deterministic sequences of 32 edits across parsing and spacing options.
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
  Unindented blank lines stay separate when deleting their neighbor brings a
  CR and LF together; the model records any required CRLF ending so subsequent
  edits preserve it.
- Bounded exhaustive tests enumerate all 41,371 strings of zero through four
  scalars from a 14-character alphabet containing delimiters, whitespace,
  Unicode, and a BOM. Every input runs through all four parser-option
  combinations and through the round-trip fuzz oracle. Checks include
  losslessness, token partitioning, one node per physical line, diagnostic
  boundaries/locations (a leading BOM is not a column), diagnostics that do
  not change when the last line is removed, and an independent semantic
  expectation for a restricted single-line subset.
- Large-input tests exercise a 4 MiB value, large Unicode identifiers,
  32,768 continuations, and 65,536 malformed lines. A separate Linux child
  process measures CPU time and peak RSS for growing entry lists, errors,
  continuations, and values. Broad growth bounds catch gross superlinear
  regressions; they are not a proof of complexity or an absolute latency SLA.
- Stable tests run on Linux, macOS, and Windows.

The bounded exhaustive test, the corruption shards, and the large-input tests
are ignored in ordinary test runs and mutation testing; the corruption sample
runs in both. The `extended-tests` CI job runs them explicitly in release mode
and saves the CPU/memory measurements as an artifact:

```sh
cargo test --release --test exhaustive_parser --test stress -- --include-ignored
cargo build --release --example scaling_probe
python3 .github/scripts/check_scaling.py
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

Distinct-entry measurements use 65,536, 131,072, and 262,144 entries. Smaller
16K–64K windows showed steep growth in the unchanged parser that varied across
CI runners and produced a false alarm on the same source tree. The larger
window checks sustained growth with the same ratio bound; other workload
families use 16,384, 32,768, and 65,536 units.

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
in debug and release modes and on the minimum supported Rust version.

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
   (`check`), and the stable tests on macOS and Windows (`macos-26` and
   `windows-2025`).
2. MSRV (`msrv`).
3. 100% functions, instantiations, lines, regions, branches, and MC/DC
   (`coverage / branches (100%)` and `coverage / MC/DC (100%)`).
4. Full-suite Miri with the documented upstream workaround (`miri (1/4)`
   through `miri (4/4)`), plus strict Miri for lexer and diagnostic contracts
   (`miri-strict`).
5. Dependency audit and policy checks (`dependencies`).
6. Full mutation test (`mutation (1/4)` through `mutation (4/4)`).
7. All five fuzz targets (`fuzz`), for 15 minutes each on the published
   commit, as on Sundays.
8. Bounded exhaustive, fixture corruption, large-input, and CPU/memory scaling
   tests (`extended-tests`).
9. A `fuzz/Cargo.lock` that matches the release version (`check`). `fuzz/` is
   a separate workspace, so the version bump does not reach its lockfile. The
   Release PR workflow commits
   `cargo update --manifest-path fuzz/Cargo.toml --package ini-edit` to the
   release pull request every time it opens or rewrites it. If that step
   fails, run the command and commit `fuzz/Cargo.lock` by hand.
10. The package check (`package`): the crate ships every tracked file except
    the repository-only ones, and its tests and doctests pass when run from
    the extracted `.crate` file.
