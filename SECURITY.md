# Security policy

## Reporting a vulnerability

Report vulnerabilities privately, through GitHub's private vulnerability reporting:
[open a report][report], or select **Report a vulnerability** on the repository's **Security** tab.
Do not open a public issue, pull request or discussion about it, because that discloses the problem
before a fix is available. If you are not sure whether a bug is a vulnerability, report it
privately.

A useful report includes:

- the ini-edit version or commit, and the Rust version;
- a minimal reproducer: the input text and the calls that trigger the problem;
- what happens and what you expected, for example a panic, a hang, memory use far out of proportion
  to the input, or saved text that means something other than the documentation says;
- how untrusted data reaches the problem, for example through a file or a value that an attacker
  controls.

The report and its discussion stay private. The fix is prepared in the advisory's temporary private
fork and merged to `main` just before the release that contains it. The advisory is published with
that release, with credit to you if you want it.

## Supported versions

Security fixes are released as a new version. Only the latest release is supported: while ini-edit
is at 0.x, fixes are not backported to earlier releases.

[report]: https://github.com/jjant/ini-edit/security/advisories/new
