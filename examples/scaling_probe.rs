//! A bounded child process for CPU and peak-RSS scaling measurements.
//!
//! `scaling_probe <workload> <units> <parses>` builds one document and parses
//! it `parses` times. Several parses make CPU times long enough to measure;
//! one parse gives the peak memory of a single parse.

use std::fmt::Write;
use std::hint::black_box;

/// `=` lines per `error-lines` unit. The largest size that `check_scaling.py`
/// uses, 65,536 units, is then 2 MiB: one parse of it needs far more memory
/// than the process itself, so its peak RSS per input byte shows the parser's.
const ERROR_LINES_PER_UNIT: usize = 16;
/// `[]x` lines per `header-errors` unit. They are twice as long as `=` lines,
/// so 65,536 units are 2 MiB too.
const HEADER_LINES_PER_UNIT: usize = 8;
/// Blank lines per `newlines` unit, for the same reason: 65,536 units are 4 MiB.
const BLANK_LINES_PER_UNIT: usize = 64;
const UNITS_MAX: usize = 262_144;
const PARSES_MAX: usize = 8;

fn source(kind: &str, units: usize) -> Result<String, Box<dyn std::error::Error>> {
    let mut source = String::from("[s]\n");
    match kind {
        "entries" => {
            for index in 0..units {
                writeln!(source, "key{index}=value{index}")?;
            }
        }
        // A key followed by text instead of a separator. With the default
        // options (`errors`) that is the missing-separator error of
        // `parse_entry`; with `allow_no_value` (`bare-key-errors`, see
        // `options`) a bare key is valid, so the text after it is the error,
        // through `parse_entry`'s unexpected-text branch.
        "errors" | "bare-key-errors" => source.push_str(&"broken key\n".repeat(units)),
        // A line that starts with `=` is an error line (`parse_error_line`):
        // one error, with its own message string, every two bytes.
        "error-lines" => source.push_str(&"=\n".repeat(units * ERROR_LINES_PER_UNIT)),
        // A header without a name, then text after it (`parse_section_header`):
        // a section and two errors every four bytes, the densest input found.
        "header-errors" => source.push_str(&"[]x\n".repeat(units * HEADER_LINES_PER_UNIT)),
        "continuations" => {
            source.push_str("k=");
            source.push_str(&"chunk \\\r\n".repeat(units));
            source.push_str("last\n");
        }
        "long-value" => {
            source.push_str("k=");
            source.push_str(&"λ".repeat(units * 64));
            source.push('\n');
        }
        // A blank line is a whole tree node in one byte.
        "newlines" => source.push_str(&"\n".repeat(units * BLANK_LINES_PER_UNIT)),
        _ => return Err("unknown workload".into()),
    }
    Ok(source)
}

/// The parse options of a workload: only `bare-key-errors` accepts bare keys.
fn options(kind: &str) -> ini_edit::ParseOptions {
    ini_edit::ParseOptions {
        allow_no_value: kind == "bare-key-errors",
        inline_comments: false,
    }
}

/// The diagnostics that `source(kind, units)` must produce, so that every
/// line of an error workload takes its error path.
fn expected_errors(kind: &str, units: usize) -> usize {
    match kind {
        "errors" | "bare-key-errors" => units,
        "error-lines" => units * ERROR_LINES_PER_UNIT,
        "header-errors" => 2 * units * HEADER_LINES_PER_UNIT,
        _ => 0,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let kind = args.next().ok_or("expected workload name")?;
    let units: usize = args.next().ok_or("expected size")?.parse()?;
    let parse_count: usize = args.next().ok_or("expected parse count")?.parse()?;
    if !(1..=UNITS_MAX).contains(&units)
        || !(1..=PARSES_MAX).contains(&parse_count)
        || args.next().is_some()
    {
        let usage =
            format!("expected <workload> <units 1..={UNITS_MAX}> <parses 1..={PARSES_MAX}>");
        return Err(usage.into());
    }
    let source = source(&kind, units)?;
    let expected_errors = expected_errors(&kind, units);
    let options = options(&kind);
    for _ in 0..parse_count {
        let parsed = ini_edit::parse_with(black_box(&source), &options);
        assert_eq!(parsed.errors().len(), expected_errors);
        assert_eq!(parsed.syntax().text().to_string(), source);
        black_box(parsed.green());
    }
    println!(
        "{{\"bytes\":{},\"units\":{units},\"parses\":{parse_count}}}",
        source.len()
    );
    Ok(())
}
