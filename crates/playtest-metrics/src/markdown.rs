//! Minimal append-only markdown builder for the Phase 1 reporter.
//!
//! The Unit 15 plan calls this out as "no templating engine; YAGNI".
//! This module is exactly the shape of that rule: a `String` inside,
//! one method per markdown primitive we actually emit, no state
//! machine, no block stack. Tables align on fixed column widths so
//! the output is both human-readable in a terminal and a valid GitHub
//! pipe-table.
//!
//! Non-goals (any of these belongs in a real markdown crate, not here):
//! * escape handling for backticks, pipes, or angle brackets in content
//! * nested lists, block quotes, footnotes
//! * streaming; the whole report fits comfortably in memory at Phase 1 scale

use core::fmt::Write;

/// Append-only markdown document.
#[derive(Debug, Default)]
pub struct MarkdownBuilder {
    buf: String,
}

impl MarkdownBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn h1(&mut self, text: &str) -> &mut Self {
        self.heading(1, text)
    }

    pub fn h2(&mut self, text: &str) -> &mut Self {
        self.heading(2, text)
    }

    pub fn h3(&mut self, text: &str) -> &mut Self {
        self.heading(3, text)
    }

    fn heading(&mut self, level: u8, text: &str) -> &mut Self {
        self.ensure_blank_line();
        for _ in 0..level {
            self.buf.push('#');
        }
        self.buf.push(' ');
        self.buf.push_str(text.trim());
        self.buf.push_str("\n\n");
        self
    }

    /// Append one line of text as its own paragraph (trailing blank line).
    pub fn paragraph(&mut self, text: &str) -> &mut Self {
        self.buf.push_str(text);
        if !text.ends_with('\n') {
            self.buf.push('\n');
        }
        self.buf.push('\n');
        self
    }

    /// Append a single line with no trailing blank line — useful inside
    /// a sequence of terse bullets or a small note under a heading.
    pub fn line(&mut self, text: &str) -> &mut Self {
        self.buf.push_str(text);
        if !text.ends_with('\n') {
            self.buf.push('\n');
        }
        self
    }

    /// Append a `- {text}` bullet.
    pub fn bullet(&mut self, text: &str) -> &mut Self {
        self.buf.push_str("- ");
        self.buf.push_str(text);
        if !text.ends_with('\n') {
            self.buf.push('\n');
        }
        self
    }

    /// End any open bullet block so the next paragraph is separated
    /// from it with a blank line.
    pub fn end_block(&mut self) -> &mut Self {
        self.ensure_blank_line();
        self
    }

    /// Emit a pipe-table with aligned columns. All cells are rendered
    /// as-is; callers are responsible for formatting numbers and
    /// escaping any `|` in text cells (which don't appear in this
    /// codebase's metric names).
    ///
    /// # Panics
    /// Panics if any row's length differs from `headers.len()` — the
    /// reporter builds rows programmatically, so this is a bug rather
    /// than a user-facing error.
    pub fn table(&mut self, headers: &[&str], rows: &[Vec<String>]) -> &mut Self {
        assert!(
            !headers.is_empty(),
            "markdown table needs at least one column"
        );
        for row in rows {
            assert_eq!(
                row.len(),
                headers.len(),
                "row length does not match header count"
            );
        }

        // Compute per-column widths (max of header + all cells).
        let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
        for row in rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.len());
            }
        }

        self.ensure_blank_line();
        write_row(&mut self.buf, headers, &widths);
        write_separator(&mut self.buf, &widths);
        for row in rows {
            let borrowed: Vec<&str> = row.iter().map(String::as_str).collect();
            write_row(&mut self.buf, &borrowed, &widths);
        }
        self.buf.push('\n');
        self
    }

    /// Finalise and return the accumulated markdown.
    #[must_use]
    pub fn into_string(self) -> String {
        self.buf
    }

    /// Ensure there's a single blank line between the previous block
    /// and whatever comes next. Avoids double-blank runs on repeated
    /// calls so the output stays compact and diff-friendly.
    fn ensure_blank_line(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        if !self.buf.ends_with('\n') {
            self.buf.push('\n');
        }
        if !self.buf.ends_with("\n\n") {
            self.buf.push('\n');
        }
    }
}

fn write_row(buf: &mut String, cells: &[&str], widths: &[usize]) {
    buf.push('|');
    for (cell, width) in cells.iter().zip(widths.iter()) {
        let _ = write!(buf, " {cell:<width$} |");
    }
    buf.push('\n');
}

fn write_separator(buf: &mut String, widths: &[usize]) {
    buf.push('|');
    for width in widths {
        buf.push(' ');
        for _ in 0..*width {
            buf.push('-');
        }
        buf.push(' ');
        buf.push('|');
    }
    buf.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_paragraphs_are_separated_by_a_blank_line() {
        let mut md = MarkdownBuilder::new();
        md.h1("Title").paragraph("Body text.").h2("Sub");
        let s = md.into_string();
        assert!(
            s.starts_with("# Title\n\nBody text.\n\n## Sub\n"),
            "got: {s}"
        );
    }

    #[test]
    fn bullets_stack_without_blank_lines_between_them() {
        let mut md = MarkdownBuilder::new();
        md.bullet("one").bullet("two").bullet("three");
        let s = md.into_string();
        assert_eq!(s, "- one\n- two\n- three\n");
    }

    #[test]
    fn table_aligns_columns_and_emits_separator() {
        let mut md = MarkdownBuilder::new();
        md.table(
            &["rank", "kept", "note"],
            &[
                vec!["5".into(), "91.3%".into(), "high-value".into()],
                vec!["2".into(), "40.0%".into(), "low-value".into()],
            ],
        );
        let s = md.into_string();
        assert!(
            s.contains("| rank | kept  | note       |\n"),
            "expected aligned header, got:\n{s}"
        );
        assert!(
            s.contains("| ---- | ----- | ---------- |\n"),
            "expected separator, got:\n{s}"
        );
        assert!(s.contains("| 5    | 91.3% | high-value |"), "got:\n{s}");
    }

    #[test]
    #[should_panic(expected = "row length does not match header count")]
    fn table_panics_on_jagged_rows() {
        let mut md = MarkdownBuilder::new();
        md.table(&["a", "b"], &[vec!["only-one".into()]]);
    }

    #[test]
    fn into_string_returns_the_full_document() {
        let mut md = MarkdownBuilder::new();
        md.h1("Report").paragraph("1 game.");
        let out = md.into_string();
        assert!(out.starts_with("# Report\n\n1 game.\n\n"));
    }
}
