//! Human-readable tables, status colors, relative timestamps, JSON/YAML/name
//! output formats, and destructive confirmation prompts.

use std::io::{IsTerminal, Write};

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use console::Style;
use rig_api::LogLine;
use serde::Serialize;
use serde_json::Value;

use crate::error::{CliError, CliResult};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Yaml,
    Name,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct OutputOpts {
    pub format: OutputFormat,
    pub wide: bool,
    pub yes: bool,
}

impl OutputOpts {
    pub fn is_json(self) -> bool {
        self.format == OutputFormat::Json
    }

    pub fn is_structured(self) -> bool {
        matches!(self.format, OutputFormat::Json | OutputFormat::Yaml)
    }

    pub fn is_name(self) -> bool {
        self.format == OutputFormat::Name
    }
}

/// True when stdout is an interactive terminal and `NO_COLOR` is unset.
pub fn color_enabled() -> bool {
    std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && console::colors_enabled()
}

/// True when stderr is an interactive terminal and `NO_COLOR` is unset.
pub fn stderr_color_enabled() -> bool {
    std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && console::colors_enabled_stderr()
}

/// Styles a status word (`running`, `failed`, `deploying`, etc.) when stdout is a colored TTY.
pub fn style_status(status: &str) -> String {
    if !color_enabled() {
        return status.to_string();
    }
    let lower = status.to_ascii_lowercase();
    let style = match lower.as_str() {
        "running" | "ok" | "ready" | "enabled" | "active" => Style::new().green(),
        "queued" | "building" | "deploying" | "creating" | "idle" | "stopped" => {
            Style::new().yellow()
        }
        "failed" | "error" | "not reachable" => Style::new().red(),
        "superseded" | "cancelled" | "disabled" => Style::new().dim(),
        _ => return status.to_string(),
    };
    style.apply_to(status).to_string()
}

/// Formats a domain into an `https://` URL (or `http://` when `tls_off`).
pub fn domain_url(host: &str, tls_off: bool) -> String {
    if host.starts_with("http://") || host.starts_with("https://") {
        host.to_string()
    } else if tls_off {
        format!("http://{host}")
    } else {
        format!("https://{host}")
    }
}

/// Prints structured output (`--json` or `-o yaml`).
pub fn print_structured<T: Serialize>(value: &T, format: OutputFormat) -> CliResult<()> {
    match format {
        OutputFormat::Yaml => {
            let v = serde_json::to_value(value)?;
            println!("{}", to_yaml(&v));
            Ok(())
        }
        _ => print_json(value),
    }
}

pub fn print_json<T: Serialize>(value: &T) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn print_names(names: &[String]) {
    for n in names {
        println!("{n}");
    }
}

pub fn print_ndjson<T: Serialize>(value: &T) -> CliResult<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

pub fn print_log(data: &str, json: bool) {
    print_log_opts(data, json, false);
}

pub fn print_log_opts(data: &str, json: bool, show_timestamps: bool) {
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
        Ok(line) => {
            let prefix = if show_timestamps && !line.ts.is_empty() {
                format!("{} ", line.ts)
            } else {
                String::new()
            };
            if line.stream == "stderr" {
                eprintln!("{prefix}{}", line.line);
            } else {
                println!("{prefix}{}", line.line);
            }
        }
        Err(_) => println!("{data}"),
    }
}

/// Prompts the user to confirm a destructive action unless `yes` is true.
///
/// Refuses without a terminal so automated scripts must pass `--yes`.
pub fn confirm(prompt: &str, yes: bool) -> CliResult<()> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::message(
            "refusing to run destructive action without a terminal; pass --yes to confirm",
        ));
    }
    eprint!("{prompt} [y/N]: ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let trimmed = answer.trim().to_ascii_lowercase();
    if matches!(trimmed.as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::message("cancelled"))
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

/// Formats an RFC 3339 timestamp relative to now (`3m ago`, `2h ago`, `5d ago`).
/// Falls back to the raw string if it cannot be parsed.
pub fn relative_time(rfc3339: &str) -> String {
    relative_time_from(rfc3339, Utc::now())
}

pub fn relative_time_from(rfc3339: &str, now: DateTime<Utc>) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(rfc3339) else {
        return rfc3339.to_string();
    };
    let delta = now.signed_duration_since(parsed.with_timezone(&Utc));
    let secs = delta.num_seconds();
    if secs < 0 {
        return rfc3339.chars().take(10).collect();
    }
    if secs < 10 {
        "just now".into()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 30 * 86_400 {
        format!("{}d ago", secs / 86_400)
    } else {
        parsed.format("%Y-%m-%d").to_string()
    }
}

/// Formats a `std::time::Duration` concisely (`420ms`, `12.4s`, `2m 15s`).
pub fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", d.as_secs_f64())
    } else {
        let secs = d.as_secs();
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

/// Parses a duration flag like `30s`, `5m`, `2h`, or bare seconds `120`.
pub fn parse_duration_secs(raw: &str) -> CliResult<std::time::Duration> {
    let raw = raw.trim();
    if let Ok(secs) = raw.parse::<u64>()
        && secs > 0
    {
        return Ok(std::time::Duration::from_secs(secs));
    }
    if raw.len() >= 2 {
        let (num, unit) = raw.split_at(raw.len() - 1);
        if let Ok(n) = num.parse::<u64>()
            && n > 0
        {
            let secs = match unit {
                "s" | "S" => Some(n),
                "m" | "M" => n.checked_mul(60),
                "h" | "H" => n.checked_mul(3600),
                "d" | "D" => n.checked_mul(86400),
                _ => None,
            };
            if let Some(s) = secs {
                return Ok(std::time::Duration::from_secs(s));
            }
        }
    }
    Err(CliError::message(format!(
        "invalid duration `{raw}`; use e.g. 30s, 5m, 1h, or seconds"
    )))
}

fn visible_len(s: &str) -> usize {
    console::strip_ansi_codes(s).chars().count()
}

fn truncate(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let plain = console::strip_ansi_codes(s);
    let count = plain.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars == 1 {
        return "…".into();
    }
    let prefix: String = plain.chars().take(max_chars - 1).collect();
    format!("{prefix}…")
}

/// Prints a plain aligned table, or `empty_hint` when `rows` is empty.
///
/// Handles ANSI-styled cells accurately and truncates long columns on narrow TTYs
/// unless `wide` is true. Non-TTY output is never truncated.
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
                widths[i] = widths[i].max(visible_len(cell));
            }
        }
    }

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
                let pad = widths[i].saturating_sub(visible_len(&fitted));
                out.push_str(&fitted);
                for _ in 0..pad + 2 {
                    out.push(' ');
                }
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

/// Renders a compact ASCII/Unicode sparkline for a slice of numeric values.
pub fn sparkline(values: &[f64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if values.is_empty() {
        return String::new();
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    values
        .iter()
        .map(|&v| {
            let idx = if span <= f64::EPSILON {
                if v > 0.0 { 3 } else { 0 }
            } else {
                (((v - min) / span) * 7.0).round().clamp(0.0, 7.0) as usize
            };
            BARS[idx]
        })
        .collect()
}

/// Lightweight YAML serializer over `serde_json::Value` (avoids pulling a heavy YAML crate).
pub fn to_yaml(value: &Value) -> String {
    let mut out = String::new();
    write_yaml(&mut out, value, 0);
    out.trim_end().to_string()
}

fn yaml_scalar(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.starts_with([' ', '-', '?', ':', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`'])
        || s.ends_with(' ')
        || s.contains([':', '\n', '\r', '\t', '#'])
        || matches!(s, "true" | "false" | "null" | "~" | "yes" | "no")
        || s.parse::<f64>().is_ok();
    if needs_quote {
        serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
    } else {
        s.to_string()
    }
}

fn write_yaml(out: &mut String, value: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => out.push_str(&yaml_scalar(s)),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                    out.push_str(&pad);
                }
                match item {
                    Value::Object(map) if !map.is_empty() => {
                        out.push_str("- ");
                        for (j, (k, v)) in map.iter().enumerate() {
                            if j > 0 {
                                out.push('\n');
                                out.push_str(&pad);
                                out.push_str("  ");
                            }
                            out.push_str(&yaml_scalar(k));
                            out.push(':');
                            match v {
                                Value::Object(m) if !m.is_empty() => {
                                    out.push('\n');
                                    out.push_str(&" ".repeat(indent + 4));
                                    write_yaml(out, v, indent + 4);
                                }
                                Value::Array(a) if !a.is_empty() => {
                                    out.push('\n');
                                    out.push_str(&" ".repeat(indent + 4));
                                    write_yaml(out, v, indent + 4);
                                }
                                _ => {
                                    out.push(' ');
                                    write_yaml(out, v, indent + 4);
                                }
                            }
                        }
                    }
                    Value::Array(a) if !a.is_empty() => {
                        out.push_str("-\n");
                        out.push_str(&" ".repeat(indent + 2));
                        write_yaml(out, item, indent + 2);
                    }
                    _ => {
                        out.push_str("- ");
                        write_yaml(out, item, indent + 2);
                    }
                }
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                    out.push_str(&pad);
                }
                out.push_str(&yaml_scalar(k));
                out.push(':');
                match v {
                    Value::Object(m) if !m.is_empty() => {
                        out.push('\n');
                        out.push_str(&" ".repeat(indent + 2));
                        write_yaml(out, v, indent + 2);
                    }
                    Value::Array(a) if !a.is_empty() => {
                        out.push('\n');
                        out.push_str(&" ".repeat(indent + 2));
                        write_yaml(out, v, indent + 2);
                    }
                    _ => {
                        out.push(' ');
                        write_yaml(out, v, indent + 2);
                    }
                }
            }
        }
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
        assert_eq!(domain_url("web.example.com", false), "https://web.example.com");
        assert_eq!(domain_url("web.example.com", true), "http://web.example.com");
    }

    #[test]
    fn relative_timestamps_and_durations() {
        let now = DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(relative_time_from("2026-11-22T11:59:55Z", now), "just now");
        assert_eq!(relative_time_from("2026-11-22T11:59:20Z", now), "40s ago");
        assert_eq!(relative_time_from("2026-11-22T11:55:00Z", now), "5m ago");
        assert_eq!(relative_time_from("2026-11-22T09:00:00Z", now), "3h ago");
        assert_eq!(relative_time_from("2026-11-18T12:00:00Z", now), "4d ago");
        assert_eq!(relative_time_from("2025-01-05T12:00:00Z", now), "2025-01-05");
        assert_eq!(parse_duration_secs("5m").unwrap().as_secs(), 300);
        assert_eq!(parse_duration_secs("90").unwrap().as_secs(), 90);
        assert!(parse_duration_secs("0s").is_err());
    }

    #[test]
    fn yaml_and_sparkline() {
        let yaml = to_yaml(&serde_json::json!({
            "name": "web",
            "port": 80,
            "domains": ["web.example.com"],
        }));
        assert!(yaml.contains("name: web"));
        assert!(yaml.contains("port: 80"));
        assert!(yaml.contains("- web.example.com"));
        assert_eq!(sparkline(&[0.0, 50.0, 100.0]).chars().count(), 3);
    }
}
