//! Rendering of query results: an aligned table for people, JSON for scripts.

use crate::cli::Format;
use crate::json;
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
                        json::string(&row.target),
                        json::option_string(&row.interface),
                        json::option_string(&row.label),
                        json::string(row.state.as_str()),
                        json::string(&format_rfc3339(row.last_change_at_ms)),
                        row.last_change_at_ms,
                        json::option_i64(row.last_rtt_us),
                    )
                })
                .collect();
            json::array(items)
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
                        row.cause.clone().unwrap_or_else(|| "-".to_string()),
                        millis(row.rtt_us),
                    ]
                })
                .collect();
            table(
                &[
                    "OCCURRED AT",
                    "TARGET",
                    "INTERFACE",
                    "TRANSITION",
                    "CAUSE",
                    "RTT",
                ],
                body,
            )
        }
        Format::Json => {
            let items: Vec<String> = rows
                .iter()
                .map(|row| {
                    format!(
                        "  {{\"id\":{}, \"target\":{}, \"interface\":{}, \"previousState\":{}, \
                         \"state\":{}, \"occurredAt\":{}, \"occurredAtMs\":{}, \"rttUs\":{}, \
                         \"cause\":{}, \"causeMessage\":{}}}",
                        row.id,
                        json::string(&row.target),
                        json::option_string(&row.interface),
                        json::string(row.previous_state.as_str()),
                        json::string(row.state.as_str()),
                        json::string(&format_rfc3339(row.occurred_at_ms)),
                        row.occurred_at_ms,
                        json::option_i64(row.rtt_us),
                        json::option_string(&row.cause),
                        json::option_string(&row.cause_message),
                    )
                })
                .collect();
            json::array(items)
        }
    }
}

fn millis(microseconds: Option<i64>) -> String {
    match microseconds {
        Some(value) => format!("{:.3} ms", value as f64 / 1_000.0),
        None => "-".to_string(),
    }
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
            cause: None,
            cause_message: None,
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
        assert!(
            rendered.contains("\"cause\":null"),
            "an up event has no cause"
        );
    }

    #[test]
    fn an_event_table_states_the_cause_of_a_disconnection() {
        let mut row = event_row();
        row.state = TargetState::Down;
        row.cause = Some("TCP.REFUSED".to_string());
        row.cause_message = Some("the connection to 1.1.1.1:443 was refused".to_string());

        let rendered = events(&[row], Format::Table);
        let header = rendered.lines().next().unwrap_or_default();
        assert!(
            header.contains("CAUSE"),
            "the column has a header: {header}"
        );
        assert!(
            rendered.contains("TCP.REFUSED"),
            "the code is printed under it: {rendered}"
        );

        let mut for_json = event_row();
        for_json.state = TargetState::Down;
        for_json.cause = Some("TCP.REFUSED".to_string());
        for_json.cause_message = Some("the connection to 1.1.1.1:443 was refused".to_string());
        let json = events(&[for_json], Format::Json);
        assert!(
            json.contains("\"causeMessage\":\"the connection to 1.1.1.1:443 was refused\""),
            "the JSON carries the message as well as the code: {json}"
        );

        let mut quiet = event_row();
        quiet.cause = None;
        assert!(
            events(&[quiet], Format::Table).contains("-"),
            "an event without a cause states a dash, like the other empty columns"
        );
    }
}
