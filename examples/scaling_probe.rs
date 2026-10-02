//! A bounded child process for CPU and peak-RSS scaling measurements.

use std::fmt::Write;
use std::hint::black_box;

fn source(kind: &str, units: usize) -> Result<String, Box<dyn std::error::Error>> {
    let mut source = String::from("[s]\n");
    match kind {
        "entries" => {
            for index in 0..units {
                writeln!(source, "key{index}=value{index}")?;
            }
        }
        "errors" => source.push_str(&"broken key\n".repeat(units)),
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
        _ => return Err("unknown workload".into()),
    }
    Ok(source)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let kind = args.next().ok_or("expected workload name")?;
    let units: usize = args.next().ok_or("expected size")?.parse()?;
    if !(1..=262_144).contains(&units) || args.next().is_some() {
        return Err("size must be 1..=262144; expected exactly two arguments".into());
    }
    let source = source(&kind, units)?;
    for _ in 0..8 {
        let parsed = ini_edit::parse(black_box(&source));
        assert_eq!(
            parsed.errors().len(),
            if kind == "errors" { 2 * units } else { 0 }
        );
        assert_eq!(parsed.syntax().text().to_string(), source);
        black_box(parsed.green());
    }
    println!(
        "{{\"bytes\":{},\"units\":{units},\"iterations\":8}}",
        source.len()
    );
    Ok(())
}
