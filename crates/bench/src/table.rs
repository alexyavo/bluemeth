//! Markdown table rendering.

use std::fmt::Write as _;

/// A simple markdown table with left-aligned first column and right-aligned
/// numeric columns.
#[derive(Debug, Default)]
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new<S: Into<String>>(headers: impl IntoIterator<Item = S>) -> Self {
        Self {
            headers: headers.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row<S: Into<String>>(&mut self, cells: impl IntoIterator<Item = S>) -> &mut Self {
        self.rows.push(cells.into_iter().map(Into::into).collect());
        self
    }

    pub fn render(&self) -> String {
        let cols = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate().take(cols) {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let fmt_row = |cells: &[String]| {
            let mut out = String::from("|");
            for (i, w) in widths.iter().copied().enumerate() {
                let cell = cells.get(i).map_or("", String::as_str);
                if i == 0 {
                    let _ = write!(out, " {cell:<w$} |");
                } else {
                    let _ = write!(out, " {cell:>w$} |");
                }
            }
            out
        };
        let mut out = fmt_row(&self.headers);
        out.push('\n');
        out.push('|');
        for (i, w) in widths.iter().enumerate() {
            if i == 0 {
                let _ = write!(out, " {} |", "-".repeat(*w));
            } else {
                let _ = write!(out, " {}: |", "-".repeat(w.saturating_sub(1)));
            }
        }
        for row in &self.rows {
            out.push('\n');
            out.push_str(&fmt_row(row));
        }
        out
    }
}
