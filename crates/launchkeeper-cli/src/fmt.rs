//! Small formatting helpers for tables and timestamps. No extra dependency:
//! fixed-width columns computed by hand.

use chrono::{DateTime, Local, Utc};
use launchkeeper_core::Run;

/// Formats a UTC timestamp as local time, second resolution.
pub fn local_time(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// `finished_at - started_at` as `12s` / `3m12s`, or `运行中` while it has not
/// finished.
pub fn duration(run: &Run) -> String {
    match run.finished_at {
        None => "运行中".to_string(),
        Some(finished) => {
            let secs = (finished - run.started_at).num_seconds().max(0);
            if secs >= 60 {
                format!("{}m{}s", secs / 60, secs % 60)
            } else {
                format!("{secs}s")
            }
        }
    }
}

/// `0` / `-` / `signal` describing how a run ended.
pub fn exit_code(run: &Run) -> String {
    match (run.finished_at, run.exit_code) {
        (None, _) => "-".to_string(),
        (Some(_), Some(code)) => code.to_string(),
        (Some(_), None) => "signal".to_string(),
    }
}

/// Why a run ended, in Chinese, or `-` for a run that is still going or
/// predates schema v2.
pub fn stop_reason(run: &Run) -> String {
    run.stop_reason
        .map(|r| r.describe().to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// A service's uptime as `45s` / `12m03s` / `3h07m` / `2d04h`.
pub fn uptime(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else if s < 86400 {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d{:02}h", s / 86400, (s % 86400) / 3600)
    }
}

/// Renders a simple fixed-width table: `headers` plus `rows`, each row having
/// the same number of columns as `headers`. Columns are left-aligned and
/// padded to the widest cell (Chinese text may make alignment approximate,
/// which is acceptable for a temporary CLI).
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let ncols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(ncols) {
            widths[i] = widths[i].max(display_width(cell));
        }
    }
    let mut out = String::new();
    out.push_str(&render_row(headers.iter().map(|s| s.to_string()), &widths));
    out.push('\n');
    for row in rows {
        out.push_str(&render_row(row.iter().cloned(), &widths));
        out.push('\n');
    }
    out
}

fn render_row(cells: impl Iterator<Item = String>, widths: &[usize]) -> String {
    let parts: Vec<String> = cells
        .enumerate()
        .map(|(i, c)| pad(&c, widths.get(i).copied().unwrap_or(0)))
        .collect();
    parts.join("  ").trim_end().to_string()
}

/// East-Asian characters roughly take two terminal columns; good enough for
/// aligning a table that mixes ASCII and Chinese.
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

fn is_wide(c: char) -> bool {
    let cp = c as u32;
    (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
}

fn pad(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_pads_columns() {
        let out = table(
            &["name", "trigger"],
            &[
                vec!["a".into(), "每天 21:00".into()],
                vec!["long-name".into(), "登录时".into()],
            ],
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("name"));
    }

    #[test]
    fn uptime_picks_a_readable_unit() {
        assert_eq!(uptime(0), "0s");
        assert_eq!(uptime(-5), "0s");
        assert_eq!(uptime(45), "45s");
        assert_eq!(uptime(60), "1m00s");
        assert_eq!(uptime(723), "12m03s");
        assert_eq!(uptime(3600), "1h00m");
        assert_eq!(uptime(11_220), "3h07m");
        assert_eq!(uptime(187_200), "2d04h");
    }
}
