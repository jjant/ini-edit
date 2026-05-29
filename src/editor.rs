//! Mutation API for format-preserving INI editing.
//!
//! The [`Editor`] works on a line-based representation internally, using
//! the parsed tree to locate sections and entries by structure rather than
//! fragile string matching. Untouched lines are preserved byte-for-byte.
//!
//! # Example
//!
//! ```
//! use ini_edit::editor::Editor;
//!
//! let src = "[server]\nhost = 0.0.0.0\nport = 8080\n";
//! let mut editor = Editor::new(src);
//!
//! editor.section("server").set("port", "9090");
//! editor.section("server").append_entry("timeout", "30");
//!
//! let output = editor.finish();
//! assert!(output.contains("port = 9090"));
//! assert!(output.contains("timeout = 30"));
//! assert!(output.contains("host = 0.0.0.0")); // untouched
//! ```

use crate::ast::{AstNode, File};
use crate::parse;

/// A format-preserving editor for INI files.
///
/// Modifications are accumulated and applied when [`finish`](Self::finish)
/// is called, returning the new source string.
#[derive(Debug, Clone)]
pub struct Editor {
    lines: Vec<String>,
    /// Line ending detected from the source (used for new lines).
    line_ending: &'static str,
}

impl Editor {
    /// Create an editor from source text.
    #[must_use]
    pub fn new(src: &str) -> Self {
        let line_ending = if src.contains("\r\n") { "\r\n" } else { "\n" };
        let lines: Vec<String> = split_lines_preserving(src);
        Self { lines, line_ending }
    }

    /// Get a handle to a section. Creates the section at the end of the
    /// file if it doesn't exist.
    pub fn section(&mut self, name: &str) -> SectionEditor<'_> {
        // Find existing section range, or create it.
        let range = self.find_section_range(name).unwrap_or_else(|| {
            // Append new section at end.
            let start = self.lines.len();
            if start > 0 && !self.lines[start - 1].is_empty() {
                self.lines.push(String::new()); // blank line before new section
            }
            self.lines.push(format!("[{name}]{}", self.line_ending));
            let end = self.lines.len();
            start..end
        });

        SectionEditor {
            editor: self,
            section_name: name.to_string(),
            _range: range,
        }
    }

    /// Render the final output.
    #[must_use]
    pub fn finish(self) -> String {
        self.lines.join("")
    }

    /// Find the line range `[header_line, end_line)` for a named section.
    fn find_section_range(&self, name: &str) -> Option<std::ops::Range<usize>> {
        let src = self.lines.join("");
        let p = parse(&src);
        let file = File::cast(p.syntax())?;

        for section in file.sections() {
            if section.name().as_deref() == Some(name) {
                let start_offset = section.syntax().text_range().start();
                let end_offset = section.syntax().text_range().end();
                let start_line = offset_to_line(&self.lines, start_offset.into());
                let end_line = offset_to_line(&self.lines, end_offset.into());
                return Some(start_line..end_line);
            }
        }
        None
    }

    /// Find the line index and column info for a specific entry within a section.
    fn find_entry_line(&self, section_name: &str, key: &str) -> Option<usize> {
        let src = self.lines.join("");
        let p = parse(&src);
        let file = File::cast(p.syntax())?;

        for section in file.sections() {
            if section.name().as_deref() != Some(section_name) {
                continue;
            }
            for entry in section.entries() {
                if entry.key().as_deref() == Some(key) {
                    let offset: usize = entry.syntax().text_range().start().into();
                    return Some(offset_to_line(&self.lines, offset));
                }
            }
        }
        None
    }

    /// Find the line just past the last entry/content in a section (where
    /// new entries should be appended).
    fn section_append_line(&self, section_name: &str) -> usize {
        if let Some(range) = self.find_section_range(section_name) {
            range.end
        } else {
            self.lines.len()
        }
    }
}

/// Handle for editing a specific section.
pub struct SectionEditor<'a> {
    editor: &'a mut Editor,
    section_name: String,
    _range: std::ops::Range<usize>,
}

impl SectionEditor<'_> {
    /// Set a key's value. If the key exists, its value is updated in-place.
    /// If it doesn't exist, a new entry is appended.
    pub fn set(&mut self, key: &str, value: &str) {
        if let Some(line_idx) = self.editor.find_entry_line(&self.section_name, key) {
            // Replace the line, preserving the key's original formatting for
            // the key name but using canonical format for the value.
            let line = &self.editor.lines[line_idx];
            if let Some(new_line) = replace_value_in_line(line, value, self.editor.line_ending) {
                self.editor.lines[line_idx] = new_line;
            }
        } else {
            self.append_entry(key, value);
        }
    }

    /// Append a new entry at the end of this section (canonical format).
    pub fn append_entry(&mut self, key: &str, value: &str) {
        let line = format!("{key} = {value}{}", self.editor.line_ending);
        let insert_at = self.editor.section_append_line(&self.section_name);
        self.editor.lines.insert(insert_at, line);
    }

    /// Remove an entry by key name. Returns true if found and removed.
    pub fn remove_entry(&mut self, key: &str) -> bool {
        if let Some(line_idx) = self.editor.find_entry_line(&self.section_name, key) {
            self.editor.lines.remove(line_idx);
            true
        } else {
            false
        }
    }

    /// Rename a key (preserving its value and formatting).
    pub fn rename_key(&mut self, old_key: &str, new_key: &str) -> bool {
        if let Some(line_idx) = self.editor.find_entry_line(&self.section_name, old_key) {
            let line = &self.editor.lines[line_idx];
            if let Some(new_line) = replace_key_in_line(line, old_key, new_key) {
                self.editor.lines[line_idx] = new_line;
                return true;
            }
        }
        false
    }

    /// Remove this entire section (header + all entries). Comments above
    /// the section are NOT removed.
    pub fn remove(self) {
        if let Some(range) = self.editor.find_section_range(&self.section_name) {
            self.editor.lines.drain(range);
        }
    }

    /// Insert raw lines at the end of this section.
    pub fn insert_raw_lines(&mut self, lines: &[&str]) {
        let insert_at = self.editor.section_append_line(&self.section_name);
        for (i, line) in lines.iter().enumerate() {
            let formatted = if line.ends_with('\n') || line.ends_with('\r') {
                line.to_string()
            } else {
                format!("{line}{}", self.editor.line_ending)
            };
            self.editor.lines.insert(insert_at + i, formatted);
        }
    }

    /// Remove a range of lines (0-indexed relative to section start).
    pub fn remove_lines(&mut self, relative_range: std::ops::Range<usize>) {
        if let Some(section_range) = self.editor.find_section_range(&self.section_name) {
            let abs_start = section_range.start + relative_range.start;
            let abs_end = section_range.start + relative_range.end;
            let clamped_end = abs_end.min(self.editor.lines.len());
            if abs_start < clamped_end {
                self.editor.lines.drain(abs_start..clamped_end);
            }
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Split source into lines, keeping line endings attached to each line.
fn split_lines_preserving(src: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            lines.push(src[start..=i].to_string());
            start = i + 1;
        } else if bytes[i] == b'\r' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                lines.push(src[start..=i + 1].to_string());
                start = i + 2;
                i += 1;
            } else {
                lines.push(src[start..=i].to_string());
                start = i + 1;
            }
        }
        i += 1;
    }
    // Remaining content without trailing newline.
    if start < src.len() {
        lines.push(src[start..].to_string());
    }
    lines
}

/// Convert a byte offset to a line index.
fn offset_to_line(lines: &[String], offset: usize) -> usize {
    let mut acc = 0;
    for (i, line) in lines.iter().enumerate() {
        if acc + line.len() > offset {
            return i;
        }
        acc += line.len();
    }
    lines.len()
}

/// Replace the value portion of a `key = value` line, preserving the key
/// and separator formatting.
fn replace_value_in_line(line: &str, new_value: &str, line_ending: &str) -> Option<String> {
    // Find the separator (= or :)
    let sep_pos = line.find('=').or_else(|| line.find(':'))?;
    let after_sep = &line[sep_pos + 1..];
    // Find where the old value starts (skip whitespace after separator).
    let ws_after_sep = after_sep.len() - after_sep.trim_start().len();
    let prefix = &line[..sep_pos + 1 + ws_after_sep];
    Some(format!("{prefix}{new_value}{line_ending}"))
}

/// Replace the key portion of a line, preserving everything after it.
fn replace_key_in_line(line: &str, old_key: &str, new_key: &str) -> Option<String> {
    let trimmed_start = line.len() - line.trim_start().len();
    let key_start = line[trimmed_start..].find(old_key)?;
    let abs_start = trimmed_start + key_start;
    let abs_end = abs_start + old_key.len();
    Some(format!(
        "{}{new_key}{}",
        &line[..abs_start],
        &line[abs_end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_existing_value() {
        let src = "[server]\nhost = 0.0.0.0\nport = 8080\n";
        let mut ed = Editor::new(src);
        ed.section("server").set("port", "9090");
        let out = ed.finish();
        assert!(out.contains("port = 9090"));
        assert!(out.contains("host = 0.0.0.0"));
    }

    #[test]
    fn append_new_entry() {
        let src = "[server]\nhost = 0.0.0.0\n";
        let mut ed = Editor::new(src);
        ed.section("server").append_entry("port", "8080");
        let out = ed.finish();
        assert!(out.contains("port = 8080\n"));
        assert!(out.contains("host = 0.0.0.0\n"));
    }

    #[test]
    fn set_creates_if_missing() {
        let src = "[server]\nhost = 0.0.0.0\n";
        let mut ed = Editor::new(src);
        ed.section("server").set("port", "8080");
        let out = ed.finish();
        assert!(out.contains("port = 8080"));
    }

    #[test]
    fn auto_create_section() {
        let src = "[existing]\nk = v\n";
        let mut ed = Editor::new(src);
        ed.section("new").append_entry("key", "value");
        let out = ed.finish();
        assert!(out.contains("[new]"));
        assert!(out.contains("key = value"));
        assert!(out.contains("[existing]\nk = v")); // preserved
    }

    #[test]
    fn remove_entry() {
        let src = "[s]\na = 1\nb = 2\nc = 3\n";
        let mut ed = Editor::new(src);
        assert!(ed.section("s").remove_entry("b"));
        let out = ed.finish();
        assert!(!out.contains("b = 2"));
        assert!(out.contains("a = 1"));
        assert!(out.contains("c = 3"));
    }

    #[test]
    fn rename_key() {
        let src = "[s]\nold_name = value\n";
        let mut ed = Editor::new(src);
        ed.section("s").rename_key("old_name", "new_name");
        let out = ed.finish();
        assert!(out.contains("new_name = value"));
        assert!(!out.contains("old_name"));
    }

    #[test]
    fn remove_section() {
        let src = "; comment\n[a]\nx = 1\n[b]\ny = 2\n";
        let mut ed = Editor::new(src);
        ed.section("a").remove();
        let out = ed.finish();
        assert!(!out.contains("[a]"));
        assert!(!out.contains("x = 1"));
        assert!(out.contains("; comment")); // comment preserved
        assert!(out.contains("[b]\ny = 2"));
    }

    #[test]
    fn insert_raw_lines() {
        let src = "[s]\nk = v\n";
        let mut ed = Editor::new(src);
        ed.section("s").insert_raw_lines(&[
            "; marker start",
            "custom_key=custom_val",
            "; marker end",
        ]);
        let out = ed.finish();
        assert!(out.contains("; marker start\n"));
        assert!(out.contains("custom_key=custom_val\n"));
        assert!(out.contains("; marker end\n"));
    }

    #[test]
    fn preserves_formatting() {
        let src = "; top comment\n\n[server]\n  host=0.0.0.0\n  port=8080\n\n[other]\nk = v\n";
        let mut ed = Editor::new(src);
        ed.section("server").set("port", "9090");
        let out = ed.finish();
        // Comment and blank lines preserved.
        assert!(out.starts_with("; top comment\n\n"));
        // Other section untouched.
        assert!(out.contains("[other]\nk = v\n"));
        // Indentation of host preserved (we only touched port).
        assert!(out.contains("  host=0.0.0.0\n"));
    }
}
