//! `GET /api/v1/faults`: the faults, newest first, one page at a time.

use std::net::TcpStream;

use crate::history::{Fault, FaultFilter, History, HistorySource};
use crate::json;
use crate::time::{format_rfc3339, now_ms};

use super::{
    close, instant, open, reject, resolve, truncated, Envelope, Meta, Params, Rejection, Reply,
    StreamError,
};

/// The faults, newest first, streamed one keyset page at a time.
pub(super) fn route(stream: &mut TcpStream, query: &str, source: &dyn HistorySource, reply: Reply) {
    let params = match Params::parse(query, &["target", "from", "to", "limit", "offset"]) {
        Ok(params) => params,
        Err(rejection) => return reject(stream, rejection, reply),
    };
    let Some(store) = open(stream, source, reply) else {
        return;
    };
    let target = match resolve(store.as_ref(), params.target.as_deref()) {
        Ok(target) => target,
        Err(rejection) => return reject(stream, rejection, reply),
    };

    let total = match store.count_faults(&FaultFilter {
        target: target.clone(),
        from_ms: params.from,
        to_ms: params.to,
        limit: None,
        offset: None,
        after_ms: None,
        after_id: None,
    }) {
        Ok(total) => total,
        Err(error) => return reject(stream, Rejection::failed(&error), reply),
    };

    let now = now_ms();
    let meta = Meta {
        limit: params.limit,
        offset: params.offset,
        total,
        target: target.as_deref(),
        from: params.from,
        to: params.to,
        generated_at: now,
    };

    let mut envelope = Envelope::begin(stream, reply);
    if let Err(error) = walk(store.as_ref(), &params, target.as_deref(), &mut envelope) {
        return truncated(error);
    }
    if let Err(error) = envelope.finish(&meta) {
        return truncated(StreamError::Write(error));
    }
    close(stream, reply);
}

/// How many faults one keyset read asks for, so the memory of an answer is one page.
const PAGE: u32 = 200;

/// Write the requested faults, one keyset page at a time. The first read honours the
/// `offset` of the request; every later one continues from the last row it saw.
fn walk(
    history: &dyn History,
    params: &Params,
    target: Option<&str>,
    envelope: &mut Envelope,
) -> Result<(), StreamError> {
    let mut after: (Option<i64>, Option<i64>) = (None, None);
    let mut written = 0;
    while written < params.limit {
        let want = (params.limit - written).min(PAGE);
        let page = history
            .faults(&FaultFilter {
                target: target.map(str::to_string),
                from_ms: params.from,
                to_ms: params.to,
                limit: Some(want),
                offset: (written == 0).then_some(params.offset),
                after_ms: after.0,
                after_id: after.1,
            })
            .map_err(StreamError::History)?;
        if page.is_empty() {
            break;
        }
        for fault in &page {
            envelope
                .record(&fault_record(fault))
                .map_err(StreamError::Write)?;
        }
        written += page.len() as u32;
        let last = page.last().expect("a page holds at least one fault");
        after = (Some(last.started_at_ms), Some(last.event_id));
        if page.len() as u32 == want {
            continue;
        }
        break;
    }
    Ok(())
}

fn fault_record(fault: &Fault) -> String {
    let duration = fault.ended_at_ms.map(|ended| ended - fault.started_at_ms);
    format!(
        "  {{\"id\": {}, \"target\": {}, \"label\": {}, \"interface\": {}, \
         \"occurredAt\": {}, \"occurredAtMs\": {}, \"cause\": {}, \"causeMessage\": {}, \
         \"endedAt\": {}, \"endedAtMs\": {}, \"durationMs\": {}, \"ongoing\": {}}}",
        fault.event_id,
        json::string(&fault.target),
        json::option_string(&fault.label),
        json::option_string(&fault.interface),
        json::string(&format_rfc3339(fault.started_at_ms)),
        fault.started_at_ms,
        json::option_string(&fault.cause),
        json::option_string(&fault.cause_message),
        instant(fault.ended_at_ms),
        json::option_i64(fault.ended_at_ms),
        json::option_i64(duration),
        fault.ended_at_ms.is_none(),
    )
}
