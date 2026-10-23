# Contributing to ini-edit

Thank you for helping to improve ini-edit. This guide lists what a change needs before it is
merged: the branch, the pull request title and description, the tests, and the checks to run
locally.

- [TESTING.md](TESTING.md) describes the test layers and the CI gates in detail.
- [docs/standards.md](docs/standards.md) explains how the project applies TigerStyle and "How
  SQLite Is Tested", and why it departs from some of their rules.
- Report security vulnerabilities privately, as [SECURITY.md](SECURITY.md) explains, never in a
  public issue.

## Reporting bugs

Open an issue with a minimal reproducer: the input text, the calls you made, the output you got,
the output you expected, and the ini-edit version.

## Pull requests

Branch from the latest `main` and open the pull request against `main`, the only long-lived
branch. Keep each pull request to one change. For a larger change, open an issue first to agree on
the approach.

Pull requests are squash-merged, and GitHub uses the pull request's title as the commit subject and
its description as the commit body. Write both as a commit message: `git log` and `git blame` show
them long after the pull request page is forgotten.

### Title

Use a [Conventional Commits][conventional] subject, `type: summary`, in the imperative mood and at
most 72 characters long, such as `fix: accept inline comments after bare keys`. release-plz builds
CHANGELOG.md from these subjects, so a title without one of the types below never reaches the
changelog. The groups are configured in [.release-plz.toml](.release-plz.toml):

| Type | Use it for | Changelog group |
|---|---|---|
| `feat` | New public API or behavior | Added |
| `fix` | A bug fix | Fixed |
| `perf` | Faster or smaller, with the same behavior | Performance |
| `docs` | Documentation only | Documentation |
| `refactor` | Restructured code with the same behavior | Changed |
| `test`, `ci`, `chore`, `style` | Tests, CI, tooling and formatting | Not listed |

A change that breaks the public API puts `!` after the type, as in `feat!: ...`, and its
description has a "Breaking changes" section with a migration example. While ini-edit is at 0.x,
release-plz then bumps the minor version instead of the patch version.

### Description

Say what changed and why. For a bug fix, include a minimal reproducer (Rust code or input text) and
the exact behavior before and after the fix. Then say how you verified the change. Delete any
pre-filled template text that does not apply, because it would become part of the commit.

## What every change needs

- **A regression test for every bug fix.** Write the test first, watch it fail on `main`, then fix
  the bug, and say in the pull request that you saw the test fail. Name the test after the behavior
  it pins. As How SQLite Is Tested (§5) puts it, a bug "is not considered fixed until new test
  cases that would exhibit the bug have been added". That test is what keeps the bug from coming
  back.
- **A committed reproducer for every fuzz finding.** A crash, timeout or out-of-memory input that
  a fuzz target finds, locally (`fuzz/artifacts/<target>/`) or in CI (the run's `fuzz-failures-*`
  artifact), is fixed only when its input is committed. TESTING.md describes the
  [committed inputs](TESTING.md#fuzz-corpus-and-reproducibility).
  1. Minimize the input with the limits CI fuzzes with. `cargo fuzz tmin` sets no timeout of its
     own, so without `-timeout=10` a slow input would not count as a failure. The result is
     written to `fuzz/artifacts/<target>/minimized-from-*`:

     ```sh
     cargo +nightly fuzz tmin <target> <input> -- -timeout=10 -rss_limit_mb=1024
     ```

  2. In the pull request that fixes the bug, commit the minimized file under
     `tests/fuzz_regressions/<target>/` with a descriptive name such as
     `crash-empty-section-insert`, not a 40-hex-digit hash: refreshing the minimized corpus
     replaces the hash names. Add a named regression test that states the bug through the public
     API.
  3. Check that the committed input fails without the fix and passes with it, in both replays.
     `cargo test` replays every committed input on every platform, with a 10-second limit per
     input. libFuzzer replays it with all of CI's limits, so it also catches an input that only
     exceeds the 1 GiB memory limit; without the limits, libFuzzer would allow 20 minutes and
     2 GiB:

     ```sh
     cargo test --test fuzz_regressions
     cargo +nightly fuzz run <target> tests/fuzz_regressions/<target>/<name> -- \
       -runs=0 -timeout=10 -rss_limit_mb=1024
     ```
- **Tests for every new behavior.** CI requires 100% function, instantiation, line, region and
  branch coverage of the library, 100% MC/DC, and no surviving mutant, so each new branch needs a
  test that notices when it changes.

## Code style

- `cargo fmt` formats Rust code, and clippy's `all` and `pedantic` groups must report no warnings.
- Functions stay within 70 lines, and every line within 100 columns, in tests, CI files and
  Markdown too. [docs/standards.md](docs/standards.md) lists the few exceptions.
- Names are full words: `source`, not `src`.
- Comments are sentences that explain why the code is written the way it is.
- Assertions catch programmer errors, never bad user data.
  [docs/standards.md](docs/standards.md#safety) explains which checks run in every build and which
  only in debug builds.

## Checks to run locally

CI runs each of these checks on every pull request ([ci.yml](.github/workflows/ci.yml) and
[coverage.yml](.github/workflows/coverage.yml)), with the same toolchains. Run the first group
before every push, and the others when your change can affect what they check.

Besides stable Rust, the commands use the rustup toolchains 1.85.0 (the `rust-version` in
Cargo.toml), nightly (with the `miri`, `rust-src` and `llvm-tools-preview` components) and
nightly-2025-08-08 (with `llvm-tools-preview`); the cargo-llvm-cov, cargo-fuzz, cargo-mutants,
cargo-nextest and cargo-deny tools; jq; and, for the scaling check, Python 3 on Linux. CI installs
the newest stable, nightly and tools on every run, so update yours (`rustup update stable nightly`)
when a check passes locally but fails in CI. nightly-2025-08-08 is the one nightly that CI pins:
rustc removed its MC/DC instrumentation that day, so no later nightly can measure MC/DC.

Every change. ci.yml denies warnings in every build, so a warning that only a release or MSRV
build emits fails CI. The first line does the same locally (rustdoc reads `RUSTDOCFLAGS` instead):

```sh
export RUSTFLAGS='--deny warnings'
cargo fmt --all --check
cargo clippy --all-targets --all-features -- --deny warnings
cargo test --all-targets --all-features
cargo test --release --all-targets --all-features
cargo test --doc --all-features
RUSTDOCFLAGS='--deny warnings' cargo doc --no-deps --all-features
```

Any change to Rust code, on the minimum supported Rust version, with the same `RUSTFLAGS`:

```sh
cargo +1.85.0 test --all-targets --all-features
```

Any change to `src/`: coverage. CI requires 100% of branches, functions, instantiations, lines and
regions. The table that `cargo llvm-cov` prints by default has no instantiations column, so check
the JSON summary as CI does. The `jq --exit-status` command prints `true` only when those five
totals are all 100%, and `jq '.data[0].totals'` shows the counts (this report's `mcdc` total is
empty, so it is not checked here):

```sh
CARGO_TARGET_DIR=target/coverage-branch cargo +nightly llvm-cov --lib \
  --all-features --branch --json --summary-only --output-path /tmp/ini-edit-coverage.json
jq --exit-status '.data[0].totals | [.branches, .functions, .instantiations, .lines, .regions]
  | all(.percent == 100)' /tmp/ini-edit-coverage.json
```

When you add or change a condition, MC/DC too. This report needs its own pinned nightly, and CI
requires 100% of MC/DC conditions as well as the totals above:

```sh
CARGO_TARGET_DIR=target/coverage-mcdc cargo +nightly-2025-08-08 llvm-cov --lib --all-features \
  --mcdc --json --summary-only --output-path /tmp/ini-edit-mcdc.json
jq --exit-status '.data[0].totals
  | [.mcdc, .branches, .functions, .instantiations, .lines, .regions]
  | all(.percent == 100)' /tmp/ini-edit-mcdc.json
```

Keep the two target directories apart. The two nightlies put their test binaries in different
places, and cargo-llvm-cov reports every test binary it finds, so a binary that the other toolchain
left behind shows up as uncovered code: twice as many instantiations, half of them missed. CI does
not see this, because every job starts from a fresh checkout.

Lexer changes, under Miri with full borrow checking:

```sh
cargo +nightly miri test --lib lexer::tests::
cargo +nightly miri test --test exhaustive_parser \
  lexer_and_diagnostic_contracts -- --exact
```

The whole library suite under Miri takes about half an hour, so CI splits it into four parallel
jobs. TESTING.md explains why that run disables Stacked Borrows. To run it anyway:

```sh
MIRIFLAGS=-Zmiri-disable-stacked-borrows cargo +nightly miri test --lib
```

Changes to the lexer, the parser, or code whose cost grows with the input: the exhaustive and
large-input tests, which ordinary runs skip, and the check of CPU time and peak memory as inputs
grow, which runs only on Linux:

```sh
cargo test --release --test exhaustive_parser --test stress -- --include-ignored
cargo build --release --example scaling_probe
python3 .github/scripts/check_scaling.py
```

Changes to `Cargo.toml` or `Cargo.lock`: the dependency policy in [deny.toml](deny.toml), which
covers advisories, yanked crates, licenses, duplicate dependencies and sources:

```sh
cargo deny --locked check advisories bans licenses sources
```

Fuzzing: TESTING.md explains how to seed each target's corpus, how to start from the committed
inputs, and what each target's input limits are. A two-minute campaign looks like this:

```sh
cargo +nightly fuzz run generated_edits -- \
  -max_len=4096 -max_total_time=120 -timeout=10 -rss_limit_mb=1024
```

### Mutation testing

CI mutates all of `src/`, one change at a time, and requires every mutant to be caught. Each
mutant rebuilds and reruns the tests, so CI splits the full run into four parallel jobs. Locally,
test only the mutants in your own change, at low priority and with two jobs, so that the machine
stays usable. Commit first, because the diff below covers committed changes only (`origin` is the
remote for jjant/ini-edit):

```sh
git diff origin/main...HEAD > /tmp/ini-edit.diff
nice -n 19 cargo mutants --in-diff /tmp/ini-edit.diff --all-features --test-tool nextest \
  --jobs 2 --timeout 30 --cap-lints true --output /tmp/ini-edit-mutants
```

To test one file instead, replace `--in-diff ...` with `--file src/<file>.rs`. Survivors are
listed in `/tmp/ini-edit-mutants/mutants.out/missed.txt`, which must be empty. A timeout counts as
caught: it is a mutant that made the tests hang.

## License

ini-edit is released under the [MIT License](LICENSE). By contributing, you agree that your
contribution is licensed under the same terms.

[conventional]: https://www.conventionalcommits.org/en/v1.0.0/
