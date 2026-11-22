//! Human-readable tables, JSON/NDJSON serialization, and formatting helpers.

use std::io::{IsTerminal, Write};

use rig_api::LogLine;
use serde::Serialize;

use crate::error::CliResult;

pub fn print_json<T: Serialize>(value: &T) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn print_ndjson<T: Serialize>(value: &T) -> CliResult<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

pub fn print_log(data: &str, json: bool) {
    if json {
        match serde_json::from_str::<LogLine>(data) {
            Ok(line) => {
                let _ = print_ndjson(&line);
            }
            Err(_) => {
                let _ = print_ndjson(&serde_json::json!({ "line": data }));
            }
        }
        return;
    }
    match serde_json::from_str::<LogLine>(data) {
        Ok(line) if line.stream == "stderr" => eprintln!("{}", line.line),
        Ok(line) => println!("{}", line.line),
        Err(_) => println!("{data}"),
    }
}

/// The wire name of an enum value, e.g. `running`.
pub fn label<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// Last 8 characters of a UUIDv7 id (the random tail, unique in practice).
pub fn short(id: &str) -> String {
    let n = id.chars().count();
    id.chars().skip(n.saturating_sub(8)).collect()
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars == 1 {
        return "…".into();
    }
    let prefix: String = s.chars().take(max_chars - 1).collect();
    format!("{prefix}…")
}

/// Prints a plain aligned table, or `empty_hint` when `rows` is empty.
///
/// When writing to a narrow terminal and `wide` is false, long columns are
/// capped so rows stay on one line. Non-TTY output is never truncated.
pub fn table(headers: &[&str], rows: Vec<Vec<String>>, empty_hint: Option<&str>, wide: bool) {
    if rows.is_empty() {
        if let Some(hint) = empty_hint {
            println!("{hint}");
        }
        return;
    }
    let max_width = if !wide && std::io::stdout().is_terminal() {
        console::Term::stdout()
            .size_checked()
            .map(|(_, cols)| usize::from(cols))
    } else {
        None
    };

    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }

    // Shrink the widest columns proportionally only if the table exceeds terminal width.
    if let Some(term_w) = max_width {
        let gaps = widths.len().saturating_sub(1) * 2;
        while widths.iter().sum::<usize>() + gaps > term_w {
            let Some((idx, &w)) = widths
                .iter()
                .enumerate()
                .filter(|(i, w)| **w > headers[*i].chars().count().max(12))
                .max_by_key(|(_, w)| **w)
            else {
                break;
            };
            widths[idx] = w - 1;
        }
    }

    let print_row = |cells: &[String]| {
        let mut out = String::new();
        for (i, cell) in cells.iter().enumerate() {
            let fitted = if max_width.is_some() {
                truncate(cell, widths[i])
            } else {
                cell.clone()
            };
            if i + 1 == cells.len() {
                out.push_str(&fitted);
            } else {
                out.push_str(&format!("{fitted:<width$}  ", width = widths[i]));
            }
        }
        println!("{}", out.trim_end());
    };

    let header_row: Vec<String> = headers.iter().map(|h| (*h).to_string()).collect();
    print_row(&header_row);
    for row in &rows {
        print_row(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_api::DeploymentStatus;

    #[test]
    fn formats() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(short("0123456789abcdef"), "89abcdef");
        assert_eq!(short("abc"), "abc");
        assert_eq!(label(&DeploymentStatus::Running), "running");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hello", 5), "hello");
    }
}
