//! `bluemeth status`: a compact view of `pmset -g assertions`.

use std::error::Error;
use std::ffi::CStr;
use std::fmt::Write as _;

use bluemeth_core::macos::inspect::{self, AssertionInfo};

/// Print every assertion (or only `pid`'s) as an aligned table.
pub fn print(pid: Option<i32>) -> Result<(), Box<dyn Error>> {
    let mut rows = match pid {
        Some(pid) => inspect::assertions_for_pid(pid)?,
        None => inspect::all_assertions()?,
    };
    rows.sort_by(|a, b| (a.pid, &a.kind, &a.name).cmp(&(b.pid, &b.kind, &b.name)));

    if rows.is_empty() {
        match pid {
            Some(pid) => println!("no power assertions held by pid {pid}"),
            None => println!("no power assertions held"),
        }
        return Ok(());
    }

    let table: Vec<[String; 6]> = rows
        .iter()
        .map(|row| {
            [
                row.pid.to_string(),
                process_name(row.pid),
                row.kind.clone(),
                row.name.clone(),
                timeout_column(row),
                row.details.clone().unwrap_or_default(),
            ]
        })
        .collect();
    let headers = ["PID", "PROCESS", "TYPE", "NAME", "TIMEOUT", "DETAILS"];
    let mut widths = headers.map(str::len);
    for row in &table {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |cells: &[&str]| {
        let mut out = String::new();
        for (i, (cell, w)) in cells.iter().zip(widths).enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            if i == cells.len() - 1 {
                out.push_str(cell);
            } else {
                let _ = write!(out, "{cell:<w$}");
            }
        }
        out.trim_end().to_owned()
    };
    println!("{}", line(&headers));
    for row in &table {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        println!("{}", line(&cells));
    }
    Ok(())
}

fn timeout_column(row: &AssertionInfo) -> String {
    match row.timeout_secs {
        Some(secs) => format!("{secs:.0}s"),
        None => "-".to_owned(),
    }
}

fn process_name(pid: i32) -> String {
    // PROC_PIDPATHINFO_MAXSIZE is 4*MAXPATHLEN; names are far shorter.
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is a valid writable buffer of the stated size; proc_name
    // writes at most `buffersize` bytes including a NUL terminator.
    let len = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if len <= 0 {
        return "?".to_owned();
    }
    CStr::from_bytes_until_nul(&buf)
        .map_or_else(|_| "?".to_owned(), |s| s.to_string_lossy().into_owned())
}
