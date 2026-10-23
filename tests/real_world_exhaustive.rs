//! Detailed checks of the real-world fixtures.
//!
//! Each fixture has a table of what it must parse to: how many diagnostics and sections it has,
//! text that its comments contain, and the position, name, entries and comments of single
//! sections. `check` parses the fixture, confirms that the tree reproduces the text exactly, and
//! checks every row of the table. Each test then makes the checks that only its fixture needs.

use ini_edit::ast::{AstNode, File, Section};
use ini_edit::syntax_kind::SyntaxKind;
use ini_edit::{SyntaxNode, parse};

const GITCONFIG: &str = include_str!("fixtures/gitconfig");
const PHP_INI: &str = include_str!("fixtures/php.ini");
const AWS_CONFIG: &str = include_str!("fixtures/aws-config");
const SYSTEMD_UNIT: &str = include_str!("fixtures/systemd-unit.service");
const MY_CNF: &str = include_str!("fixtures/my.cnf");

/// What a fixture must parse to.
struct Fixture {
    source: &'static str,
    /// How many diagnostics `parse` reports.
    error_count: usize,
    section_count: usize,
    /// Text that some comment of the file contains.
    comments: &'static [&'static str],
    sections: &'static [ExpectedSection],
}

/// What one section of a fixture contains. It is the first section named `name`, and it sits at
/// `position` among the file's sections. Positions count from 0. Entries are `(key, value)`
/// pairs, and an entry without a value has the value "".
struct ExpectedSection {
    position: usize,
    name: &'static str,
    /// When set, the section has exactly these entries, in this order.
    entries_all: Option<&'static [(&'static str, &'static str)]>,
    /// Single entries, as `(position, key, value)`.
    entries_at: &'static [(usize, &'static str, &'static str)],
    /// Entries that appear somewhere in the section.
    entries_anywhere: &'static [(&'static str, &'static str)],
    /// Keys that no entry of the section has.
    keys_absent: &'static [&'static str],
    /// Text that some comment of the section contains.
    comments: &'static [&'static str],
}

impl ExpectedSection {
    /// The base of every table row. Each row sets `position` and `name`, so a row that sets
    /// nothing else checks only where the section is and what it is called.
    const NAME_ONLY: Self = Self {
        position: 0,
        name: "",
        entries_all: None,
        entries_at: &[],
        entries_anywhere: &[],
        keys_absent: &[],
        comments: &[],
    };
}

#[test]
fn aws_config_exhaustive() {
    check(&AWS_CONFIG_EXPECTED);
}

/// The AWS CLI configuration: every section, by position and name, and their entries.
const AWS_CONFIG_EXPECTED: Fixture = Fixture {
    source: AWS_CONFIG,
    error_count: 0,
    section_count: 8,
    comments: &["AWS CLI configuration file", "docs.aws.amazon.com"],
    sections: &[
        ExpectedSection {
            position: 0,
            name: "default",
            entries_all: Some(&[
                ("region", "us-east-1"),
                ("output", "json"),
                ("cli_pager", "less"),
                ("cli_auto_prompt", "on-partial"),
                ("retry_mode", "standard"),
                ("max_attempts", "3"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 1,
            name: "profile dev",
            entries_all: Some(&[
                ("region", "us-west-2"),
                ("output", "yaml"),
                ("role_arn", "arn:aws:iam::123456789012:role/DevRole"),
                ("source_profile", "default"),
                ("duration_seconds", "3600"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 2,
            name: "profile staging",
            entries_all: Some(&[
                ("region", "eu-west-1"),
                ("output", "json"),
                ("role_arn", "arn:aws:iam::987654321098:role/StagingRole"),
                ("source_profile", "default"),
                ("mfa_serial", "arn:aws:iam::123456789012:mfa/jane"),
                ("cli_pager", ""),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 3,
            name: "profile production",
            entries_all: Some(&[
                ("region", "us-east-1"),
                ("output", "json"),
                ("role_arn", "arn:aws:iam::111222333444:role/ProdReadOnly"),
                ("source_profile", "default"),
                ("mfa_serial", "arn:aws:iam::123456789012:mfa/jane"),
                ("duration_seconds", "900"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 4,
            name: "profile sso-dev",
            entries_all: Some(&[
                ("sso_start_url", "https://my-company.awsapps.com/start"),
                ("sso_region", "us-east-1"),
                ("sso_account_id", "123456789012"),
                ("sso_role_name", "DeveloperAccess"),
                ("region", "us-west-2"),
                ("output", "json"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 5,
            name: "profile cross-account",
            entries_all: Some(&[
                (
                    "role_arn",
                    "arn:aws:iam::555666777888:role/CrossAccountRole",
                ),
                ("credential_source", "Environment"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 6,
            name: "profile ec2-instance",
            entries_all: Some(&[
                ("credential_source", "Ec2InstanceMetadata"),
                ("region", "ap-southeast-1"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 7,
            name: "profile with-endpoint",
            // `s3 =` has an empty value, and the indented lines after it are entries of their own.
            entries_at: &[
                (0, "region", "us-east-1"),
                (1, "endpoint_url", "http://localhost:4566"),
                (2, "s3", ""),
                (3, "addressing_style", "path"),
                (4, "multipart_threshold", "64MB"),
            ],
            ..ExpectedSection::NAME_ONLY
        },
    ],
};

#[test]
fn gitconfig_exhaustive() {
    let file = check(&GITCONFIG_EXPECTED);
    // A table can only ask for text in some comment, and this text must be in the first one.
    assert!(comments(file.syntax())[0].contains("Git's per-user configuration file"));

    // The `lg` alias is long, so only its key and the start of its format string are checked.
    let (key, value) = entry_at(&section_at(&file, 14), 4);
    assert_eq!(key, "lg");
    assert!(value.contains("--pretty=format:'%Cred%h%Creset"));
}

/// A Git configuration with quoted subsection names.
const GITCONFIG_EXPECTED: Fixture = Fixture {
    source: GITCONFIG,
    error_count: 0,
    section_count: 23,
    comments: &[],
    sections: &[
        ExpectedSection {
            position: 0,
            name: "user",
            entries_all: Some(&[
                ("name", "Jane Developer"),
                ("email", "jane@example.com"),
                ("signingkey", "ABCDEF1234567890"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 1,
            name: "core",
            entries_at: &[
                (0, "editor", "nvim"),
                (1, "autocrlf", "input"),
                (
                    2,
                    "whitespace",
                    "fix,-indent-with-non-tab,trailing-space,cr-at-eol",
                ),
                (3, "pager", "delta"),
                (4, "excludesfile", "~/.gitignore_global"),
                (5, "attributesfile", "~/.gitattributes"),
            ],
            ..ExpectedSection::NAME_ONLY
        },
        // A subsection keeps its quotes in the section name.
        ExpectedSection {
            position: 4,
            name: "color \"branch\"",
            entries_all: Some(&[
                ("current", "yellow reverse"),
                ("local", "yellow"),
                ("remote", "green"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        // Values keep shell syntax such as `!`, `&&` and quotes.
        ExpectedSection {
            position: 14,
            name: "alias",
            entries_at: &[
                (0, "st", "status"),
                (8, "wip", "!git add -A && git commit -m \"WIP\""),
            ],
            ..ExpectedSection::NAME_ONLY
        },
        // A section name can contain `@` and `:`.
        ExpectedSection {
            position: 15,
            name: "url \"git@github.com:\"",
            entries_at: &[(0, "insteadOf", "https://github.com/")],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 16,
            name: "remote \"origin\"",
            entries_at: &[
                (0, "url", "git@github.com:user/repo.git"),
                (1, "fetch", "+refs/heads/*:refs/remotes/origin/*"),
            ],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 18,
            name: "filter \"lfs\"",
            entries_at: &[
                (0, "clean", "git-lfs clean -- %f"),
                (1, "smudge", "git-lfs smudge -- %f"),
                (2, "process", "git-lfs filter-process"),
                (3, "required", "true"),
            ],
            ..ExpectedSection::NAME_ONLY
        },
    ],
};

#[test]
fn systemd_unit_exhaustive() {
    let file = check(&SYSTEMD_UNIT_EXPECTED);

    // A backslash at the end of a line continues the value on the next line, so `ExecStart`
    // spans four lines.
    let (key, value) = entry_at(&section_at(&file, 1), 9);
    assert_eq!(key, "ExecStart");
    assert!(value.starts_with("/opt/myapp/bin/server \\"));
    assert!(value.contains("--config /etc/myapp/config.toml \\"));
    assert!(value.contains("--workers 4"));
}

/// A systemd service unit.
const SYSTEMD_UNIT_EXPECTED: Fixture = Fixture {
    source: SYSTEMD_UNIT,
    error_count: 0,
    section_count: 3,
    comments: &[],
    sections: &[
        ExpectedSection {
            position: 0,
            name: "Unit",
            entries_at: &[
                (0, "Description", "My Application Service"),
                (1, "Documentation", "https://example.com/docs"),
                (
                    2,
                    "After",
                    "network-online.target postgresql.service redis.service",
                ),
                (3, "Wants", "network-online.target"),
                (4, "Requires", "postgresql.service"),
            ],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 1,
            name: "Service",
            // `systemd_unit_exhaustive` checks entry 9, the continued `ExecStart`.
            entries_at: &[
                (0, "Type", "notify"),
                (1, "User", "appuser"),
                (2, "Group", "appgroup"),
                (3, "WorkingDirectory", "/opt/myapp"),
                // A key can repeat.
                (4, "Environment", "NODE_ENV=production"),
                (5, "Environment", "PORT=3000"),
                // `#` and `@` inside a value are part of it.
                (
                    6,
                    "Environment",
                    "DATABASE_URL=postgres://user:p@ss#word@localhost:5432/mydb?sslmode=require",
                ),
                (7, "EnvironmentFile", "-/etc/myapp/env"),
                (8, "ExecStartPre", "/opt/myapp/bin/migrate"),
                (10, "ExecReload", "/bin/kill -HUP $MAINPID"),
                (11, "ExecStop", "/bin/kill -TERM $MAINPID"),
                (12, "Restart", "on-failure"),
                (13, "RestartSec", "5"),
                (16, "WatchdogSec", "10"),
                (17, "NoNewPrivileges", "true"),
                (18, "ProtectSystem", "strict"),
                (25, "LimitNOFILE", "65536"),
                (26, "LimitNPROC", "4096"),
                (27, "MemoryMax", "2G"),
                (28, "CPUQuota", "200%"),
            ],
            // The comments that head the groups of entries belong to the section.
            comments: &["Security hardening", "Resource limits", "Logging"],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 2,
            name: "Install",
            entries_at: &[(0, "WantedBy", "multi-user.target")],
            ..ExpectedSection::NAME_ONLY
        },
    ],
};

#[test]
fn my_cnf_exhaustive() {
    check(&MY_CNF_EXPECTED);
}

/// A MySQL option file. It has five bare keys, which have no `=` or `:`: `syslog`,
/// `skip-external-locking`, `quick`, `quote-names` and `no-auto-rehash`. With the default
/// options each one is a diagnostic, and the parser keeps it as an entry without a value.
const MY_CNF_EXPECTED: Fixture = Fixture {
    source: MY_CNF,
    error_count: 5,
    section_count: 6,
    comments: &["MySQL configuration file"],
    sections: &[
        ExpectedSection {
            position: 0,
            name: "client",
            entries_all: Some(&[
                ("port", "3306"),
                ("socket", "/var/run/mysqld/mysqld.sock"),
                ("default-character-set", "utf8mb4"),
            ]),
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 1,
            name: "mysqld_safe",
            entries_at: &[
                (0, "socket", "/var/run/mysqld/mysqld.sock"),
                (1, "nice", "0"),
            ],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 2,
            name: "mysqld",
            entries_at: &[
                (0, "user", "mysql"),
                (1, "pid-file", "/var/run/mysqld/mysqld.pid"),
            ],
            entries_anywhere: &[
                ("innodb_buffer_pool_size", "1G"),
                ("innodb_flush_method", "O_DIRECT"),
                ("binlog_format", "ROW"),
            ],
            // `;relay-log = ...` is commented out, so it is a comment and not an entry.
            keys_absent: &["relay-log"],
            comments: &[
                "Basic Settings",
                "InnoDB",
                "Binary Logging",
                "Replication",
                "relay-log",
            ],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 3,
            name: "mysqldump",
            entries_anywhere: &[("max_allowed_packet", "64M")],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 4,
            name: "mysql",
            // The backslashes of the prompt are part of its value.
            entries_anywhere: &[
                ("prompt", "\\\\u@\\\\h [\\\\d]>\\\\_"),
                ("default-character-set", "utf8mb4"),
            ],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 5,
            name: "isamchk",
            entries_at: &[(0, "key_buffer", "16M")],
            ..ExpectedSection::NAME_ONLY
        },
    ],
};

#[test]
fn php_ini_exhaustive() {
    let file = check(&PHP_INI_EXPECTED);

    // Rows of semicolons frame the headings of the [PHP] section.
    let php_comments = comments(section_at(&file, 0).syntax());
    assert!(
        php_comments
            .iter()
            .any(|comment| comment.starts_with(";;;"))
    );
}

/// A PHP configuration.
const PHP_INI_EXPECTED: Fixture = Fixture {
    source: PHP_INI,
    error_count: 0,
    section_count: 16,
    comments: &[],
    sections: &[
        ExpectedSection {
            position: 0,
            name: "PHP",
            entries_at: &[
                (0, "engine", "On"),
                (1, "short_open_tag", "Off"),
                (2, "precision", "14"),
                (3, "output_buffering", "4096"),
                (4, "implicit_flush", "Off"),
            ],
            entries_anywhere: &[
                // `&` and `~` are part of the value.
                ("error_reporting", "E_ALL & ~E_DEPRECATED & ~E_STRICT"),
                // These three values are empty.
                ("auto_prepend_file", ""),
                ("auto_append_file", ""),
                ("doc_root", ""),
                // Quotes stay in the value.
                ("variables_order", "\"GPCS\""),
                ("default_charset", "\"UTF-8\""),
            ],
            comments: &[
                "About php.ini",
                "Resource Limits",
                "Error handling",
                // Entries that are commented out are comments.
                "error_log = syslog",
                "include_path",
            ],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 2,
            name: "Date",
            // The section has one entry, after the commented-out `;date.timezone =`.
            entries_all: Some(&[("date.timezone", "\"UTC\"")]),
            comments: &["Defines the default timezone", ";date.timezone"],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 10,
            name: "Session",
            entries_at: &[(0, "session.save_handler", "files")],
            entries_anywhere: &[
                ("session.name", "PHPSESSID"),
                ("session.cookie_path", "/"),
                ("session.cookie_domain", ""),
            ],
            // `;session.save_path = ...` is commented out, so it is a comment and not an entry.
            keys_absent: &["session.save_path"],
            comments: &["session.save_path"],
            ..ExpectedSection::NAME_ONLY
        },
        ExpectedSection {
            position: 15,
            name: "opcache",
            entries_at: &[(0, "opcache.enable", "1")],
            entries_anywhere: &[("opcache.jit", "1255"), ("opcache.jit_buffer_size", "100M")],
            ..ExpectedSection::NAME_ONLY
        },
    ],
};

/// Without the flag, bare keys produce parse errors but still round-trip.
#[test]
fn bare_keys_without_flag_produce_errors() {
    let src = "[mysqldump]\nquick\nquote-names\nmax_allowed_packet = 64M\n";
    let p = parse(src);

    // The tree reproduces the source exactly, despite the errors.
    assert_eq!(p.syntax().text().to_string(), src);

    // The two bare keys produce two errors.
    assert_eq!(p.errors().len(), 2, "errors: {:?}", p.errors());

    // The section keeps its three entries, despite the errors.
    let file = File::cast(p.syntax()).unwrap();
    let section = file.sections().next().unwrap();
    let entries: Vec<_> = section
        .entries()
        .map(|e| (e.key().unwrap_or_default(), e.value().unwrap_or_default()))
        .collect();

    // Each bare key stays an entry without a value, which `unwrap_or_default` turns into "".
    assert_eq!(entries[0], ("quick".into(), String::new()));
    assert_eq!(entries[1], ("quote-names".into(), String::new()));
    // The entry with `=` still has its value.
    assert_eq!(entries[2], ("max_allowed_packet".into(), "64M".into()));
}

/// With the flag enabled, bare keys parse without errors.
#[test]
fn bare_keys_with_flag_no_errors() {
    let src = "[mysqldump]\nquick\nquote-names\nmax_allowed_packet = 64M\n";

    let p = ini_edit::parse_with(
        src,
        &ini_edit::ParseOptions {
            allow_no_value: true,
            ..Default::default()
        },
    );

    assert_eq!(p.syntax().text().to_string(), src);
    assert!(p.errors().is_empty(), "errors: {:?}", p.errors());

    let file = File::cast(p.syntax()).unwrap();
    let section = file.sections().next().unwrap();
    let entries: Vec<_> = section
        .entries()
        .map(|e| (e.key().unwrap_or_default(), e.value()))
        .collect();

    // Each bare key has the value `None`, not `Some("")`.
    assert_eq!(entries[0], ("quick".into(), None));
    assert_eq!(entries[1], ("quote-names".into(), None));
    // The entry with `=` still has its value.
    assert_eq!(
        entries[2],
        ("max_allowed_packet".into(), Some("64M".into()))
    );
}

/// Parses `fixture.source` and checks it against every row of `fixture`. Returns the file for the
/// checks that only one fixture makes.
fn check(fixture: &Fixture) -> File {
    let parsed = parse(fixture.source);
    assert_eq!(parsed.syntax().text().to_string(), fixture.source);
    assert_eq!(
        parsed.errors().len(),
        fixture.error_count,
        "errors: {:?}",
        parsed.errors()
    );

    let file = File::cast(parsed.syntax()).unwrap();
    let file_comments = comments(file.syntax());
    for text in fixture.comments {
        assert!(
            has_comment(&file_comments, text),
            "no comment contains {text:?}"
        );
    }

    let sections: Vec<Section> = file.sections().collect();
    assert_eq!(sections.len(), fixture.section_count);
    for expected in fixture.sections {
        let name = Some(expected.name);
        let position = sections
            .iter()
            .position(|section| section.name().as_deref() == name);
        assert_eq!(
            position,
            Some(expected.position),
            "position of the first [{}]",
            expected.name
        );
        check_section(&sections[expected.position], expected);
    }
    file
}

/// Checks one section against its table row.
fn check_section(section: &Section, expected: &ExpectedSection) {
    let name = expected.name;
    let section_entries = entries(section);
    let pairs: Vec<(&str, &str)> = section_entries
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    if let Some(entries_all) = expected.entries_all {
        assert_eq!(pairs, entries_all, "[{name}]");
    }
    for &(position, key, value) in expected.entries_at {
        assert_eq!(
            pairs.get(position),
            Some(&(key, value)),
            "[{name}] entry {position}"
        );
    }
    for pair in expected.entries_anywhere {
        assert!(pairs.contains(pair), "[{name}] has no entry {pair:?}");
    }
    for &key in expected.keys_absent {
        assert!(
            !pairs.iter().any(|&(other, _)| other == key),
            "[{name}] has the key {key:?}"
        );
    }

    let section_comments = comments(section.syntax());
    for text in expected.comments {
        assert!(
            has_comment(&section_comments, text),
            "[{name}] has no comment with {text:?}"
        );
    }
}

/// The section at `position` among the file's sections.
fn section_at(file: &File, position: usize) -> Section {
    file.sections().nth(position).unwrap()
}

/// The entry at `position` in `section`.
fn entry_at(section: &Section, position: usize) -> (String, String) {
    entries(section).remove(position)
}

/// The `(key, value)` pair of every entry of `section`, with "" for a missing key or value.
fn entries(section: &Section) -> Vec<(String, String)> {
    section
        .entries()
        .map(|entry| {
            (
                entry.key().unwrap_or_default(),
                entry.value().unwrap_or_default(),
            )
        })
        .collect()
}

/// The text of every comment token under `node`.
fn comments(node: &SyntaxNode) -> Vec<String> {
    node.descendants_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .filter(|token| token.kind() == SyntaxKind::COMMENT)
        .map(|token| token.text().to_string())
        .collect()
}

/// Whether some comment in `comment_texts` contains `text`.
fn has_comment(comment_texts: &[String], text: &str) -> bool {
    comment_texts.iter().any(|comment| comment.contains(text))
}
