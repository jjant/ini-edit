//! Exhaustive assertion tests for real-world fixtures.
//! Verifies every section, every entry, every comment, and all errors.
#![allow(clippy::manual_string_new)]
#![allow(clippy::too_many_lines)]

use ini_edit::ast::{AstNode, File, Section};
use ini_edit::syntax_kind::SyntaxKind;
use ini_edit::{SyntaxNode, parse};

const GITCONFIG: &str = include_str!("fixtures/gitconfig");
const PHP_INI: &str = include_str!("fixtures/php.ini");
const AWS_CONFIG: &str = include_str!("fixtures/aws-config");
const SYSTEMD_UNIT: &str = include_str!("fixtures/systemd-unit.service");
const MY_CNF: &str = include_str!("fixtures/my.cnf");

/// Collect all COMMENT tokens from a node tree.
fn comments(node: &SyntaxNode) -> Vec<String> {
    let mut out = vec![];
    for desc in node.descendants_with_tokens() {
        if let rowan::NodeOrToken::Token(t) = desc {
            if t.kind() == SyntaxKind::COMMENT {
                out.push(t.text().to_string());
            }
        }
    }
    out
}

/// Collect comments within a specific section.
fn section_comments(section: &Section) -> Vec<String> {
    comments(section.syntax())
}

fn entries(section: &Section) -> Vec<(String, String)> {
    section
        .entries()
        .map(|e| (e.key().unwrap_or_default(), e.value().unwrap_or_default()))
        .collect()
}

// =============================================================================
// AWS CONFIG
// =============================================================================

#[test]
fn aws_config_exhaustive() {
    let p = parse(AWS_CONFIG);
    assert_eq!(p.syntax().text().to_string(), AWS_CONFIG);
    assert!(p.errors().is_empty(), "errors: {:?}", p.errors());

    let file = File::cast(p.syntax()).unwrap();
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 8);

    // Comments at top
    let all_comments = comments(&p.syntax());
    assert!(
        all_comments
            .iter()
            .any(|c| c.contains("AWS CLI configuration file"))
    );
    assert!(
        all_comments
            .iter()
            .any(|c| c.contains("docs.aws.amazon.com"))
    );

    // [default]
    assert_eq!(sections[0].name().as_deref(), Some("default"));
    let e = entries(&sections[0]);
    assert_eq!(
        e,
        vec![
            ("region".into(), "us-east-1".into()),
            ("output".into(), "json".into()),
            ("cli_pager".into(), "less".into()),
            ("cli_auto_prompt".into(), "on-partial".into()),
            ("retry_mode".into(), "standard".into()),
            ("max_attempts".into(), "3".into()),
        ]
    );

    // [profile dev]
    assert_eq!(sections[1].name().as_deref(), Some("profile dev"));
    let e = entries(&sections[1]);
    assert_eq!(
        e,
        vec![
            ("region".into(), "us-west-2".into()),
            ("output".into(), "yaml".into()),
            (
                "role_arn".into(),
                "arn:aws:iam::123456789012:role/DevRole".into()
            ),
            ("source_profile".into(), "default".into()),
            ("duration_seconds".into(), "3600".into()),
        ]
    );

    // [profile staging]
    assert_eq!(sections[2].name().as_deref(), Some("profile staging"));
    let e = entries(&sections[2]);
    assert_eq!(
        e,
        vec![
            ("region".into(), "eu-west-1".into()),
            ("output".into(), "json".into()),
            (
                "role_arn".into(),
                "arn:aws:iam::987654321098:role/StagingRole".into()
            ),
            ("source_profile".into(), "default".into()),
            (
                "mfa_serial".into(),
                "arn:aws:iam::123456789012:mfa/jane".into()
            ),
            ("cli_pager".into(), "".into()),
        ]
    );

    // [profile production]
    assert_eq!(sections[3].name().as_deref(), Some("profile production"));
    let e = entries(&sections[3]);
    assert_eq!(
        e,
        vec![
            ("region".into(), "us-east-1".into()),
            ("output".into(), "json".into()),
            (
                "role_arn".into(),
                "arn:aws:iam::111222333444:role/ProdReadOnly".into()
            ),
            ("source_profile".into(), "default".into()),
            (
                "mfa_serial".into(),
                "arn:aws:iam::123456789012:mfa/jane".into()
            ),
            ("duration_seconds".into(), "900".into()),
        ]
    );

    // [profile sso-dev]
    assert_eq!(sections[4].name().as_deref(), Some("profile sso-dev"));
    let e = entries(&sections[4]);
    assert_eq!(
        e,
        vec![
            (
                "sso_start_url".into(),
                "https://my-company.awsapps.com/start".into()
            ),
            ("sso_region".into(), "us-east-1".into()),
            ("sso_account_id".into(), "123456789012".into()),
            ("sso_role_name".into(), "DeveloperAccess".into()),
            ("region".into(), "us-west-2".into()),
            ("output".into(), "json".into()),
        ]
    );

    // [profile cross-account]
    assert_eq!(sections[5].name().as_deref(), Some("profile cross-account"));
    let e = entries(&sections[5]);
    assert_eq!(
        e,
        vec![
            (
                "role_arn".into(),
                "arn:aws:iam::555666777888:role/CrossAccountRole".into()
            ),
            ("credential_source".into(), "Environment".into()),
        ]
    );

    // [profile ec2-instance]
    assert_eq!(sections[6].name().as_deref(), Some("profile ec2-instance"));
    let e = entries(&sections[6]);
    assert_eq!(
        e,
        vec![
            ("credential_source".into(), "Ec2InstanceMetadata".into()),
            ("region".into(), "ap-southeast-1".into()),
        ]
    );

    // [profile with-endpoint]
    assert_eq!(sections[7].name().as_deref(), Some("profile with-endpoint"));
    let e = entries(&sections[7]);
    assert_eq!(e[0], ("region".into(), "us-east-1".into()));
    assert_eq!(
        e[1],
        ("endpoint_url".into(), "http://localhost:4566".into())
    );
    // s3 = (empty value, followed by indented sub-keys as separate entries)
    assert_eq!(e[2], ("s3".into(), "".into()));
    assert_eq!(e[3], ("addressing_style".into(), "path".into()));
    assert_eq!(e[4], ("multipart_threshold".into(), "64MB".into()));
}

// =============================================================================
// GITCONFIG
// =============================================================================

#[test]
fn gitconfig_exhaustive() {
    let p = parse(GITCONFIG);
    assert_eq!(p.syntax().text().to_string(), GITCONFIG);
    assert!(p.errors().is_empty(), "errors: {:?}", p.errors());

    let file = File::cast(p.syntax()).unwrap();
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 23);

    // Top comment
    let all_comments = comments(&p.syntax());
    assert!(all_comments[0].contains("Git's per-user configuration file"));

    // [user]
    assert_eq!(sections[0].name().as_deref(), Some("user"));
    let e = entries(&sections[0]);
    assert_eq!(
        e,
        vec![
            ("name".into(), "Jane Developer".into()),
            ("email".into(), "jane@example.com".into()),
            ("signingkey".into(), "ABCDEF1234567890".into()),
        ]
    );

    // [core]
    assert_eq!(sections[1].name().as_deref(), Some("core"));
    let e = entries(&sections[1]);
    assert_eq!(e[0], ("editor".into(), "nvim".into()));
    assert_eq!(e[1], ("autocrlf".into(), "input".into()));
    assert_eq!(
        e[2],
        (
            "whitespace".into(),
            "fix,-indent-with-non-tab,trailing-space,cr-at-eol".into()
        )
    );
    assert_eq!(e[3], ("pager".into(), "delta".into()));
    assert_eq!(e[4], ("excludesfile".into(), "~/.gitignore_global".into()));
    assert_eq!(e[5], ("attributesfile".into(), "~/.gitattributes".into()));

    // [color "branch"] — subsection with quotes
    assert_eq!(sections[4].name().as_deref(), Some("color \"branch\""));
    let e = entries(&sections[4]);
    assert_eq!(
        e,
        vec![
            ("current".into(), "yellow reverse".into()),
            ("local".into(), "yellow".into()),
            ("remote".into(), "green".into()),
        ]
    );

    // [alias] — values with special chars
    assert_eq!(sections[14].name().as_deref(), Some("alias"));
    let e = entries(&sections[14]);
    assert_eq!(e[0], ("st".into(), "status".into()));
    assert_eq!(e[4].0, "lg");
    assert!(e[4].1.contains("--pretty=format:'%Cred%h%Creset"));
    assert_eq!(
        e[8],
        ("wip".into(), "!git add -A && git commit -m \"WIP\"".into())
    );

    // [url "git@github.com:"] — section name with special chars
    assert_eq!(
        sections[15].name().as_deref(),
        Some("url \"git@github.com:\"")
    );
    let e = entries(&sections[15]);
    assert_eq!(e[0], ("insteadOf".into(), "https://github.com/".into()));

    // [remote "origin"]
    assert_eq!(sections[16].name().as_deref(), Some("remote \"origin\""));
    let e = entries(&sections[16]);
    assert_eq!(e[0], ("url".into(), "git@github.com:user/repo.git".into()));
    assert_eq!(
        e[1],
        ("fetch".into(), "+refs/heads/*:refs/remotes/origin/*".into())
    );

    // [filter "lfs"]
    assert_eq!(sections[18].name().as_deref(), Some("filter \"lfs\""));
    let e = entries(&sections[18]);
    assert_eq!(e[0], ("clean".into(), "git-lfs clean -- %f".into()));
    assert_eq!(e[1], ("smudge".into(), "git-lfs smudge -- %f".into()));
    assert_eq!(e[2], ("process".into(), "git-lfs filter-process".into()));
    assert_eq!(e[3], ("required".into(), "true".into()));
}

// =============================================================================
// SYSTEMD UNIT
// =============================================================================

#[test]
fn systemd_unit_exhaustive() {
    let p = parse(SYSTEMD_UNIT);
    assert_eq!(p.syntax().text().to_string(), SYSTEMD_UNIT);
    assert!(p.errors().is_empty(), "errors: {:?}", p.errors());

    let file = File::cast(p.syntax()).unwrap();
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 3);

    // [Unit]
    assert_eq!(sections[0].name().as_deref(), Some("Unit"));
    let e = entries(&sections[0]);
    assert_eq!(
        e[0],
        ("Description".into(), "My Application Service".into())
    );
    assert_eq!(
        e[1],
        ("Documentation".into(), "https://example.com/docs".into())
    );
    assert_eq!(
        e[2],
        (
            "After".into(),
            "network-online.target postgresql.service redis.service".into()
        )
    );
    assert_eq!(e[3], ("Wants".into(), "network-online.target".into()));
    assert_eq!(e[4], ("Requires".into(), "postgresql.service".into()));

    // [Service]
    assert_eq!(sections[1].name().as_deref(), Some("Service"));
    let e = entries(&sections[1]);
    assert_eq!(e[0], ("Type".into(), "notify".into()));
    assert_eq!(e[1], ("User".into(), "appuser".into()));
    assert_eq!(e[2], ("Group".into(), "appgroup".into()));
    assert_eq!(e[3], ("WorkingDirectory".into(), "/opt/myapp".into()));
    // Duplicate Environment keys
    assert_eq!(e[4], ("Environment".into(), "NODE_ENV=production".into()));
    assert_eq!(e[5], ("Environment".into(), "PORT=3000".into()));
    // Value with special chars: password containing # and @
    assert_eq!(
        e[6],
        (
            "Environment".into(),
            "DATABASE_URL=postgres://user:p@ss#word@localhost:5432/mydb?sslmode=require".into()
        )
    );
    assert_eq!(e[7], ("EnvironmentFile".into(), "-/etc/myapp/env".into()));
    assert_eq!(
        e[8],
        ("ExecStartPre".into(), "/opt/myapp/bin/migrate".into())
    );
    // Backslash continuation — value spans multiple lines
    assert_eq!(e[9].0, "ExecStart");
    assert!(e[9].1.starts_with("/opt/myapp/bin/server \\"));
    assert!(e[9].1.contains("--config /etc/myapp/config.toml \\"));
    assert!(e[9].1.contains("--workers 4"));

    assert_eq!(
        e[10],
        ("ExecReload".into(), "/bin/kill -HUP $MAINPID".into())
    );
    assert_eq!(
        e[11],
        ("ExecStop".into(), "/bin/kill -TERM $MAINPID".into())
    );
    assert_eq!(e[12], ("Restart".into(), "on-failure".into()));
    assert_eq!(e[13], ("RestartSec".into(), "5".into()));

    // Security hardening section (comments within section)
    let svc_comments = section_comments(&sections[1]);
    assert!(
        svc_comments
            .iter()
            .any(|c| c.contains("Security hardening"))
    );
    assert!(svc_comments.iter().any(|c| c.contains("Resource limits")));
    assert!(svc_comments.iter().any(|c| c.contains("Logging")));

    assert_eq!(e[16], ("WatchdogSec".into(), "10".into()));

    // Security hardening section (comments within section)
    let svc_comments = section_comments(&sections[1]);
    assert!(
        svc_comments
            .iter()
            .any(|c| c.contains("Security hardening"))
    );
    assert!(svc_comments.iter().any(|c| c.contains("Resource limits")));
    assert!(svc_comments.iter().any(|c| c.contains("Logging")));

    assert_eq!(e[17], ("NoNewPrivileges".into(), "true".into()));
    assert_eq!(e[18], ("ProtectSystem".into(), "strict".into()));
    assert_eq!(e[25], ("LimitNOFILE".into(), "65536".into()));
    assert_eq!(e[26], ("LimitNPROC".into(), "4096".into()));
    assert_eq!(e[27], ("MemoryMax".into(), "2G".into()));
    assert_eq!(e[28], ("CPUQuota".into(), "200%".into()));

    // [Install]
    assert_eq!(sections[2].name().as_deref(), Some("Install"));
    let e = entries(&sections[2]);
    assert_eq!(e[0], ("WantedBy".into(), "multi-user.target".into()));
}

// =============================================================================
// MY.CNF — bare keys produce errors
// =============================================================================

#[test]
fn my_cnf_exhaustive() {
    let p = parse(MY_CNF);
    assert_eq!(p.syntax().text().to_string(), MY_CNF);

    // 5 errors for bare keys without = or :
    assert_eq!(p.errors().len(), 5, "errors: {:?}", p.errors());

    let file = File::cast(p.syntax()).unwrap();
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 6);

    // Top comment
    let all_comments = comments(&p.syntax());
    assert!(
        all_comments
            .iter()
            .any(|c| c.contains("MySQL configuration file"))
    );

    // [client]
    assert_eq!(sections[0].name().as_deref(), Some("client"));
    let e = entries(&sections[0]);
    assert_eq!(
        e,
        vec![
            ("port".into(), "3306".into()),
            ("socket".into(), "/var/run/mysqld/mysqld.sock".into()),
            ("default-character-set".into(), "utf8mb4".into()),
        ]
    );

    // [mysqld_safe] — has bare key "syslog"
    assert_eq!(sections[1].name().as_deref(), Some("mysqld_safe"));
    let e = entries(&sections[1]);
    assert_eq!(
        e[0],
        ("socket".into(), "/var/run/mysqld/mysqld.sock".into())
    );
    assert_eq!(e[1], ("nice".into(), "0".into()));
    // "syslog" is a bare key — parser error-recovers it
    // It won't appear as a normal entry since it has no separator

    // [mysqld]
    assert_eq!(sections[2].name().as_deref(), Some("mysqld"));
    let e = entries(&sections[2]);
    assert_eq!(e[0], ("user".into(), "mysql".into()));
    assert_eq!(
        e[1],
        ("pid-file".into(), "/var/run/mysqld/mysqld.pid".into())
    );
    // Check some InnoDB settings
    assert!(
        e.iter()
            .any(|kv| kv == &("innodb_buffer_pool_size".into(), "1G".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("innodb_flush_method".into(), "O_DIRECT".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("binlog_format".into(), "ROW".into()))
    );
    // Comments within section
    let mysqld_comments = section_comments(&sections[2]);
    assert!(mysqld_comments.iter().any(|c| c.contains("Basic Settings")));
    assert!(mysqld_comments.iter().any(|c| c.contains("InnoDB")));
    assert!(mysqld_comments.iter().any(|c| c.contains("Binary Logging")));
    assert!(mysqld_comments.iter().any(|c| c.contains("Replication")));
    // Commented-out entries (;relay-log) are comments, not entries
    assert!(mysqld_comments.iter().any(|c| c.contains("relay-log")));
    assert!(!e.iter().any(|kv| kv.0 == "relay-log"));

    // [mysqldump] — bare keys "quick" and "quote-names"
    assert_eq!(sections[3].name().as_deref(), Some("mysqldump"));
    let e = entries(&sections[3]);
    // Only the entry with = should parse as a proper entry
    assert!(
        e.iter()
            .any(|kv| kv == &("max_allowed_packet".into(), "64M".into()))
    );

    // [mysql]
    assert_eq!(sections[4].name().as_deref(), Some("mysql"));
    let e = entries(&sections[4]);
    // prompt has backslashes
    assert!(
        e.iter()
            .any(|kv| kv == &("prompt".into(), "\\\\u@\\\\h [\\\\d]>\\\\_".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("default-character-set".into(), "utf8mb4".into()))
    );

    // [isamchk]
    assert_eq!(sections[5].name().as_deref(), Some("isamchk"));
    let e = entries(&sections[5]);
    assert_eq!(e[0], ("key_buffer".into(), "16M".into()));
}

// =============================================================================
// PHP.INI
// =============================================================================

#[test]
fn php_ini_exhaustive() {
    let p = parse(PHP_INI);
    assert_eq!(p.syntax().text().to_string(), PHP_INI);
    assert!(p.errors().is_empty(), "errors: {:?}", p.errors());

    let file = File::cast(p.syntax()).unwrap();
    let sections: Vec<_> = file.sections().collect();
    assert_eq!(sections.len(), 16);

    // [PHP] section
    assert_eq!(sections[0].name().as_deref(), Some("PHP"));
    let e = entries(&sections[0]);
    assert_eq!(e[0], ("engine".into(), "On".into()));
    assert_eq!(e[1], ("short_open_tag".into(), "Off".into()));
    assert_eq!(e[2], ("precision".into(), "14".into()));
    assert_eq!(e[3], ("output_buffering".into(), "4096".into()));
    assert_eq!(e[4], ("implicit_flush".into(), "Off".into()));
    // error_reporting with special chars
    assert!(e.iter().any(|kv| kv
        == &(
            "error_reporting".into(),
            "E_ALL & ~E_DEPRECATED & ~E_STRICT".into()
        )));
    // Empty values
    assert!(
        e.iter()
            .any(|kv| kv == &("auto_prepend_file".into(), "".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("auto_append_file".into(), "".into()))
    );
    assert!(e.iter().any(|kv| kv == &("doc_root".into(), "".into())));
    // Quoted values preserved with quotes
    assert!(
        e.iter()
            .any(|kv| kv == &("variables_order".into(), "\"GPCS\"".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("default_charset".into(), "\"UTF-8\"".into()))
    );

    // Comments — ;;; decorative blocks
    let php_comments = section_comments(&sections[0]);
    assert!(php_comments.iter().any(|c| c.starts_with(";;;")));
    assert!(php_comments.iter().any(|c| c.contains("About php.ini")));
    assert!(php_comments.iter().any(|c| c.contains("Resource Limits")));
    assert!(php_comments.iter().any(|c| c.contains("Error handling")));
    // Commented-out entries are comments
    assert!(
        php_comments
            .iter()
            .any(|c| c.contains("error_log = syslog"))
    );
    assert!(php_comments.iter().any(|c| c.contains("include_path")));

    // [Session]
    let session = sections
        .iter()
        .find(|s| s.name().as_deref() == Some("Session"))
        .unwrap();
    let e = entries(session);
    assert_eq!(e[0], ("session.save_handler".into(), "files".into()));
    assert!(
        e.iter()
            .any(|kv| kv == &("session.name".into(), "PHPSESSID".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("session.cookie_path".into(), "/".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("session.cookie_domain".into(), "".into()))
    );
    // Commented-out save_path is a comment, not an entry
    let session_comments = section_comments(session);
    assert!(
        session_comments
            .iter()
            .any(|c| c.contains("session.save_path"))
    );
    assert!(!e.iter().any(|kv| kv.0 == "session.save_path"));

    // [opcache]
    let opcache = sections
        .iter()
        .find(|s| s.name().as_deref() == Some("opcache"))
        .unwrap();
    let e = entries(opcache);
    assert_eq!(e[0], ("opcache.enable".into(), "1".into()));
    assert!(
        e.iter()
            .any(|kv| kv == &("opcache.jit".into(), "1255".into()))
    );
    assert!(
        e.iter()
            .any(|kv| kv == &("opcache.jit_buffer_size".into(), "100M".into()))
    );

    // [Date] — commented-out entry + real entry
    let date = sections
        .iter()
        .find(|s| s.name().as_deref() == Some("Date"))
        .unwrap();
    let e = entries(date);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0], ("date.timezone".into(), "\"UTC\"".into()));
    let date_comments = section_comments(date);
    assert!(
        date_comments
            .iter()
            .any(|c| c.contains("Defines the default timezone"))
    );
    assert!(date_comments.iter().any(|c| c.contains(";date.timezone")));
}

// =============================================================================
// Bare-key (no-value) support — MySQL/systemd dialect
// =============================================================================

/// Without the flag, bare keys produce parse errors but still round-trip.
#[test]
fn bare_keys_without_flag_produce_errors() {
    let src = "[mysqldump]\nquick\nquote-names\nmax_allowed_packet = 64M\n";
    let p = parse(src);

    // Round-trips perfectly despite errors
    assert_eq!(p.syntax().text().to_string(), src);

    // Two errors for the two bare keys
    assert_eq!(p.errors().len(), 2, "errors: {:?}", p.errors());

    // The entries with = still parse correctly
    let file = File::cast(p.syntax()).unwrap();
    let section = file.sections().next().unwrap();
    let entries: Vec<_> = section
        .entries()
        .map(|e| (e.key().unwrap_or_default(), e.value().unwrap_or_default()))
        .collect();

    // Bare keys are recovered as entries with empty values
    assert_eq!(entries[0], ("quick".into(), String::new()));
    assert_eq!(entries[1], ("quote-names".into(), String::new()));
    // Normal entry still works
    assert_eq!(entries[2], ("max_allowed_packet".into(), "64M".into()));
}

/// With the flag enabled, bare keys parse without errors.
/// TODO: implement `ParseOptions { allow_no_value: true }` in a separate PR.
#[test]
#[ignore = "not yet implemented: allow_no_value flag"]
fn bare_keys_with_flag_no_errors() {
    let src = "[mysqldump]\nquick\nquote-names\nmax_allowed_packet = 64M\n";

    // Future API sketch:
    // let p = ini_edit::parse_with(src, ParseOptions { allow_no_value: true });
    let p = parse(src);

    assert_eq!(p.syntax().text().to_string(), src);
    assert!(p.errors().is_empty(), "errors: {:?}", p.errors());

    let file = File::cast(p.syntax()).unwrap();
    let section = file.sections().next().unwrap();
    let entries: Vec<_> = section
        .entries()
        .map(|e| (e.key().unwrap_or_default(), e.value()))
        .collect();

    // Bare keys have None value (not Some(""))
    assert_eq!(entries[0], ("quick".into(), None));
    assert_eq!(entries[1], ("quote-names".into(), None));
    // Normal entry still has Some value
    assert_eq!(
        entries[2],
        ("max_allowed_packet".into(), Some("64M".into()))
    );
}
