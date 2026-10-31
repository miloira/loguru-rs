//! The [`Record`] type: a single log event, fully materialized.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local};

use crate::level::LevelInfo;

/// The source location a log record originated from.
#[derive(Debug, Clone)]
pub struct Location {
    /// The module path, e.g. `my_crate::db::pool`. Empty when a record is
    /// created through a `#[track_caller]` method rather than a macro.
    pub module_path: &'static str,
    /// The function name, if captured by a macro (empty otherwise). Rust has no
    /// stable runtime function-name API, so this is best-effort.
    pub function: &'static str,
    /// The source file path.
    pub file: &'static str,
    /// The 1-based line number.
    pub line: u32,
}

impl Location {
    /// An empty, unknown location.
    pub const UNKNOWN: Location = Location {
        module_path: "",
        function: "",
        file: "",
        line: 0,
    };

    /// The file name without its directory, e.g. `main.rs`.
    pub fn file_name(&self) -> &str {
        let f = self.file;
        match f.rfind(['/', '\\']) {
            Some(idx) => &f[idx + 1..],
            None => f,
        }
    }
}

/// A single log event with everything a sink or formatter needs.
///
/// Records are owned (no borrowed data) so they can be handed to multiple
/// sinks and moved across threads (e.g. into an async/`enqueue` sink).
#[derive(Debug, Clone)]
pub struct Record {
    /// The severity of the event (name, number, color, icon).
    pub level: LevelInfo,
    /// The fully formatted message text.
    pub message: String,
    /// The name of the logger that produced the record.
    pub logger_name: Arc<str>,
    /// Where the record was emitted from.
    pub location: Location,
    /// The wall-clock time the record was created, in local time.
    pub time: DateTime<Local>,
    /// Time elapsed since the logging system was first initialized.
    pub elapsed: Duration,
    /// The OS process id.
    pub process_id: u32,
    /// A stable per-thread numeric id (assigned by this crate).
    pub thread_id: u64,
    /// The name of the emitting thread, if it has one.
    pub thread_name: Option<String>,
    /// Structured context fields (bound via `bind`/`contextualize`), merged
    /// with any fields attached to this specific call.
    pub extra: BTreeMap<String, String>,
    /// A captured backtrace/error description, present when logged via
    /// `opt().exception(...)` or `logger.exception(...)`.
    pub exception: Option<String>,
    /// Whether [`Record::message`] contains loguru color markup that sinks
    /// should parse (set by `opt().colors(true)`).
    pub colored_message: bool,
    /// Whether the record should be written raw (just the message, bypassing
    /// the sink formatter). Set by `opt().raw(true)`.
    pub raw: bool,
}

/// A stable, small numeric id for the current thread.
///
/// `std::thread::ThreadId` does not expose its number on stable Rust, so this
/// assigns sequential ids lazily per thread.
pub(crate) fn current_thread_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static ID: u64 = COUNTER.fetch_add(1, Ordering::Relaxed);
    }
    ID.with(|&id| id)
}
