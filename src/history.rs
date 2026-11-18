//! The read contract of the page, the JSON API and the export.
//!
//! The web surface answers one question — "what does the history say?" — and it must not
//! care where that history comes from. `History` is the seam: [`crate::store::Store`]
//! implements it over the event database, and the simulated source implements it over a
//! history generated on demand (see ADR-0032). `HistorySource` is its factory, because a
//! SQLite connection belongs to one request and is not shared between threads.
//!
//! The types this contract speaks in are re-exported here, so a consumer of the page
//! depends on this module and not on the database module behind it.

use thiserror::Error;

pub use crate::store::{
    DailyDrops, EventFilter, EventRow, ExportedEvent, Fault, FaultFilter, TargetState,
    TargetStateRow, TargetTimeline, TimelineSegment,
};

/// Why a read failed: the history could not be read, in whatever way its source reads it.
///
/// A database that is missing, locked, or written by a newer version, and a simulated
/// window that cannot be generated, are the same thing to the caller: the answer cannot be
/// built, and the reason is a sentence for the operator.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct HistoryError(pub String);

impl HistoryError {
    /// The reason, as the error states it.
    pub fn message(&self) -> &str {
        &self.0
    }
}

/// Everything the read surface asks of a history.
///
/// The methods mirror the questions the page, the API and the export ask, not the shape of
/// a database: an implementation is free to hold rows, generate them, or fetch them from
/// anywhere else.
pub trait History: Send {
    /// Current state of every known target, in the order the lanes are drawn.
    fn states(&self) -> Result<Vec<TargetStateRow>, HistoryError>;

    /// One target, found by label, key, or bare address.
    fn find_target(&self, needle: &str) -> Result<Option<TargetStateRow>, HistoryError>;

    /// The canonical key a needle names, when it names one.
    fn resolve_target(&self, needle: &str) -> Result<Option<String>, HistoryError>;

    /// One lane per target over `[from_ms, to_ms)`.
    fn timeline(&self, from_ms: i64, to_ms: i64) -> Result<Vec<TargetTimeline>, HistoryError>;

    /// One target's lane over `[from_ms, to_ms)`, without reading the others.
    ///
    /// The segments are the ones `timeline` would return for this target. An unknown target
    /// answers one lane with no segment, so a caller that already resolved the key never has
    /// to handle a missing lane.
    fn timeline_of(
        &self,
        target: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<TargetTimeline, HistoryError>;

    /// Faults bucketed per UTC day, for one target or for all of them.
    fn daily_drops(
        &self,
        target: Option<&str>,
        from_ms: i64,
        to_ms: i64,
        now_ms: i64,
    ) -> Result<Vec<DailyDrops>, HistoryError>;

    /// Faults matching the filter, newest first, with its paging and keyset cursor.
    fn faults(&self, filter: &FaultFilter) -> Result<Vec<Fault>, HistoryError>;

    /// How many faults the filter matches, paging aside.
    fn count_faults(&self, filter: &FaultFilter) -> Result<i64, HistoryError>;

    /// The newest faults of any target.
    fn recent_faults(&self, limit: u32) -> Result<Vec<Fault>, HistoryError>;

    /// The newest faults of one target.
    fn recent_faults_for(&self, target: &str, limit: u32) -> Result<Vec<Fault>, HistoryError>;

    /// Events matching the filter, newest first.
    fn events(&self, filter: &EventFilter) -> Result<Vec<EventRow>, HistoryError>;

    /// The events after `after_id`, oldest first, in pages, as the export walks them.
    fn events_after(
        &self,
        target: Option<&str>,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<ExportedEvent>, HistoryError>;
}

/// Where a request gets its history from.
///
/// One instance lives for the process and hands out a [`History`] per request, which keeps
/// the same shape the page had when it opened a database connection per request.
pub trait HistorySource: Send + Sync {
    /// Open a history for one request.
    fn open(&self) -> Result<Box<dyn History>, HistoryError>;

    /// What the process is reading, for the startup line: a database path, or the fact that
    /// the history is simulated.
    fn describe(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The error keeps the reason the source gave, so a page can print it.
    #[test]
    fn an_error_states_its_reason() {
        let error = HistoryError("the event database /tmp/x.db does not exist".to_string());
        assert_eq!(
            error.to_string(),
            "the event database /tmp/x.db does not exist"
        );
        assert_eq!(
            error.message(),
            "the event database /tmp/x.db does not exist"
        );
    }

    /// The contract speaks in the domain types, so a consumer of the page never names the
    /// database module.
    #[test]
    fn the_contract_reexports_the_types_it_speaks_in() {
        fn takes(_: &FaultFilter, _: &EventFilter) {}
        let _: Option<TargetStateRow> = None;
        let _: Option<TargetTimeline> = None;
        let _: Vec<DailyDrops> = Vec::new();
        let _: Vec<Fault> = Vec::new();
        let _: Vec<EventRow> = Vec::new();
        let _: Vec<ExportedEvent> = Vec::new();
        takes(&FaultFilter::default(), &EventFilter::default());
    }
}
