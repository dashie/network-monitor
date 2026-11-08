//! Rendering of query results: an aligned table for people, JSON for scripts.

use crate::cli::Format;
use crate::store::{EventRow, TargetStateRow};
use crate::time::format_rfc3339;

pub fn states(rows: &[TargetStateRow], format: Format) -> String {
    match format {
        Format::Table => {
            let body = rows
                .iter()
                .map(|row| {
                    vec![
                        row.target.clone(),
                        row.interface.clone().unwrap_or_else(|| "-".to_string()),
                        row.label.clone().unwrap_or_else(|| "-".to_string()),
                        row.state.as_str().to_string(),
                        format_rfc3339(row.last_change_at_ms),
                        millis(row.last_rtt_us),
                    ]
                })
                .collect();
            table(
                &["TARGET", "INTERFACE", "LABEL", "STATE", "SINCE", "RTT"],
                body,
            )
        }
        Format::Json => {
            let items: Vec<String> = rows
                .iter()
                .map(|row| {
                    format!(
                        "  {{\"target\":{}, \"interface\":{}, \"label\":{}, \"state\":{}, \
                         \"since\":{}, \"sinceMs\":{}, \"rttUs\":{}}}",
                        json_string(&row.target),
                        json_option(&row.interface),
                        json_option(&row.label),
                        json_string(row.state.as_str()),
                        json_string(&format_rfc3339(row.last_change_at_ms)),
                        row.last_change_at_ms,
                        json_number(row.last_rtt_us),
                    )
                })
                .collect();
            array(items)
        }
    }
}

pub fn events(rows: &[EventRow], format: Format) -> String {
    match format {
        Format::Table => {
            let body = rows
                .iter()
                .map(|row| {
                    vec![
                        format_rfc3339(row.occurred_at_ms),
                        row.target.clone(),
                        row.interface.clone().unwrap_or_else(|| "-".to_string()),
                        format!("{} -> {}", row.previous_state.as_str(), row.state.as_str()),
                        millis(row.rtt_us),
                    ]
                })
                .collect();
            table(
                &["OCCURRED AT", "TARGET", "INTERFACE", "TRANSITION", "RTT"],
                body,
            )
        }
        Format::Json => {
            let items: Vec<String> = rows
                .iter()
                .map(|row| {
                    format!(
                        "  {{\"id\":{}, \"target\":{}, \"interface\":{}, \"previousState\":{}, \
                         \"state\":{}, \"occurredAt\":{}, \"occurredAtMs\":{}, \"rttUs\":{}}}",
                        row.id,
                        json_string(&row.target),
                        json_option(&row.interface),
                        json_string(row.previous_state.as_str()),
                        json_string(row.state.as_str()),
                        json_string(&format_rfc3339(row.occurred_at_ms)),
                        row.occurred_at_ms,
                        json_number(row.rtt_us),
                    )
                })
                .collect();
            array(items)
        }
    }
}

fn array(items: Vec<String>) -> String {
    if items.is_empty() {
        "[]".to_string()
    } else {
        format!("[\n{}\n]", items.join(",\n"))
    }
}

fn millis(microseconds: Option<i64>) -> String {
    match microseconds {
        Some(value) => format!("{:.3} ms", value as f64 / 1_000.0),
        None => "-".to_string(),
    }
}

fn json_number(value: Option<i64>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "null".to_string(),
    }
}

fn json_option(value: &Option<String>) -> String {
    match value {
        Some(value) => json_string(value),
        None => "null".to_string(),
    }
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                escaped.push_str(&format!("\\u{:04x}", control as u32))
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

/// Left-aligned columns, two spaces apart, no trailing spaces.
fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.len());
            }
        }
    }

    let mut output = String::new();
    write_row(
        &mut output,
        &headers.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
        &widths,
    );
    for row in rows {
        write_row(&mut output, &row, &widths);
    }

    output
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_row(output: &mut String, cells: &[String], widths: &[usize]) {
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        match widths.get(index) {
            Some(width) => output.push_str(&format!("{cell:<width$}")),
            None => output.push_str(cell),
        }
    }
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TargetState;

    fn state_row() -> TargetStateRow {
        TargetStateRow {
            target: "1.1.1.1".to_string(),
            interface: Some("eth1".to_string()),
            label: None,
            state: TargetState::Up,
            first_seen_at_ms: 0,
            last_change_at_ms: 0,
            last_rtt_us: Some(1_234),
        }
    }

    fn event_row() -> EventRow {
        EventRow {
            id: 1,
            target: "1.1.1.1".to_string(),
            interface: None,
            previous_state: TargetState::Unknown,
            state: TargetState::Up,
            occurred_at_ms: 0,
            rtt_us: Some(1_234),
        }
    }

    #[test]
    fn renders_a_state_table_without_trailing_spaces() {
        let rendered = states(&[state_row()], Format::Table);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("TARGET"));
        assert!(!rendered.lines().any(|line| line.ends_with(' ')));
        assert!(lines[1].contains("up"));
        assert!(lines[1].contains("1.234 ms"));
    }

    #[test]
    fn renders_empty_results_as_empty_arrays() {
        assert_eq!(states(&[], Format::Json), "[]");
        assert_eq!(events(&[], Format::Json), "[]");
    }

    #[test]
    fn renders_events_as_json() {
        let rendered = events(&[event_row()], Format::Json);
        assert!(rendered.starts_with('['));
        assert!(rendered.contains("\"previousState\":\"unknown\""));
        assert!(rendered.contains("\"rttUs\":1234"));
        assert!(rendered.contains("\"interface\":null"));
    }
}
