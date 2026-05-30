#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use ini_edit::editor::Editor;

#[derive(Debug, Arbitrary)]
enum Op<'a> {
    Set { section: &'a str, key: &'a str, value: &'a str },
    Append { section: &'a str, key: &'a str, value: &'a str },
    Remove { section: &'a str, key: &'a str },
    Rename { section: &'a str, old: &'a str, new: &'a str },
    RemoveSection { section: &'a str },
    AppendRaw { section: &'a str, line: &'a str },
    InsertRawAt { section: &'a str, index: u8, line: &'a str },
    RemoveLines { section: &'a str, start: u8, end: u8 },
}

#[derive(Debug, Arbitrary)]
struct FuzzInput<'a> {
    source: &'a str,
    ops: Vec<Op<'a>>,
}

fuzz_target!(|input: FuzzInput<'_>| {
    let ed = Editor::new(input.source);

    for op in &input.ops {
        match op {
            Op::Set { section, key, value } => {
                ed.section(section).set(key, value);
            }
            Op::Append { section, key, value } => {
                ed.section(section).append_entry(key, value);
            }
            Op::Remove { section, key } => {
                let _ = ed.section(section).remove_entry(key);
            }
            Op::Rename { section, old, new } => {
                let _ = ed.section(section).rename_key(old, new);
            }
            Op::RemoveSection { section } => {
                ed.section(section).remove();
            }
            Op::AppendRaw { section, line } => {
                ed.section(section).append_raw_lines(&[line]);
            }
            Op::InsertRawAt { section, index, line } => {
                ed.section(section).insert_raw_lines_at(*index as usize, &[line]);
            }
            Op::RemoveLines { section, start, end } => {
                let s = *start as usize;
                let e = (*end as usize).max(s);
                ed.section(section).remove_lines(s..e);
            }
        }
    }

    // The output must always be valid: re-parsing must round-trip.
    let output = ed.finish();
    let re_parsed = ini_edit::parse(&output);
    assert_eq!(
        re_parsed.syntax().text().to_string(),
        output,
        "Editor output does not round-trip"
    );
});
