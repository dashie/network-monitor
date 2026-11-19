//! `GET /api/v1/targets`: every target, ordered by key.

use std::net::TcpStream;

use crate::history::{HistorySource, TargetStateRow};
use crate::json;
use crate::time::{format_rfc3339, now_ms};
use crate::web::page;

use super::{
    close, open, reject, truncated, Envelope, Meta, Params, Rejection, Reply, StreamError,
};

/// Every target, ordered by key, streamed one record at a time.
pub(super) fn route(stream: &mut TcpStream, query: &str, source: &dyn HistorySource, reply: Reply) {
    let now = now_ms();
    let params = match Params::parse(query, &["limit", "offset"], now) {
        Ok(params) => params,
        Err(rejection) => return reject(stream, rejection, reply),
    };
    let Some(store) = open(stream, source, reply) else {
        return;
    };
    let states = match store.states() {
        Ok(states) => states,
        Err(error) => return reject(stream, Rejection::failed(&error), reply),
    };

    let meta = Meta {
        limit: params.limit,
        offset: params.offset,
        total: states.len() as i64,
        target: None,
        from: None,
        to: None,
        generated_at: now,
        step: None,
        tz: None,
    };

    let mut envelope = Envelope::begin(stream, reply);
    let page = states
        .iter()
        .skip(params.offset as usize)
        .take(params.limit as usize);
    for row in page {
        if let Err(error) = envelope.record(&target_record(row, now)) {
            return truncated(StreamError::Write(error));
        }
    }
    if let Err(error) = envelope.finish(&meta) {
        return truncated(StreamError::Write(error));
    }
    close(stream, reply);
}

fn target_record(row: &TargetStateRow, now: i64) -> String {
    format!(
        "  {{\"target\": {}, \"label\": {}, \"interface\": {}, \"state\": {}, \
         \"stateSince\": {}, \"stateSinceMs\": {}, \"firstSeenAt\": {}, \"firstSeenAtMs\": {}, \
         \"lastRttUs\": {}, \"upForMs\": {}}}",
        json::string(&row.target),
        json::option_string(&row.label),
        json::option_string(&row.interface),
        json::string(row.state.as_str()),
        json::string(&format_rfc3339(row.last_change_at_ms)),
        row.last_change_at_ms,
        json::string(&format_rfc3339(row.first_seen_at_ms)),
        row.first_seen_at_ms,
        json::option_i64(row.last_rtt_us),
        json::option_i64(page::up_for(row.state, row.last_change_at_ms, now)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degraded_target_is_stated_by_its_own_name() {
        let row = TargetStateRow {
            target: "tcp://1.1.1.1:443".to_string(),
            interface: None,
            label: None,
            state: crate::store::TargetState::Degraded,
            first_seen_at_ms: 0,
            last_change_at_ms: 1_000,
            last_rtt_us: Some(250_000),
        };
        let record = target_record(&row, 5_000);
        assert!(
            record.contains("\"state\": \"degraded\""),
            "unexpected record: {record}"
        );
    }
}
