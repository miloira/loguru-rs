//! Sinks are the destinations a logger writes records to.
//!
//! A sink corresponds to a loguru "sink" or a Python `logging` "handler". Each
//! sink has its own minimum severity, its own [`Formatter`], and an optional
//! [`Filter`], so one logger can route records to several destinations with
//! different levels, formats, and rules.
//!
//! [`Formatter`]: crate::format::Formatter

mod async_sink;
mod console;
mod file;
mod func;
mod writer;

pub use async_sink::AsyncSink;
pub use console::ConsoleSink;
pub use file::{Compression, FileSink, Retention, Rotation};
pub use func::FnSink;
pub use writer::WriterSink;

use std::sync::Arc;

use crate::record::Record;

/// A predicate deciding whether a sink should emit a given record. Mirrors
/// loguru's per-sink `filter`.
pub type Filter = Arc<dyn Fn(&Record) -> bool + Send + Sync>;

/// A destination for log records.
///
/// Implementors are shared across threads (`Send + Sync`) and receive records
/// by shared reference, so they must handle their own interior synchronization.
pub trait Sink: Send + Sync {
    /// The minimum severity number this sink emits. The logger drops records
    /// below it before calling [`Sink::write`].
    fn level(&self) -> u16;

    /// Handle a single record (already past the level check).
    fn write(&self, record: &Record);

    /// Flush any buffered output. Default: no-op.
    fn flush(&self) {}

    /// Wait for all queued records to be processed (for async sinks) and flush.
    /// Default: just [`flush`](Sink::flush).
    fn complete(&self) {
        self.flush();
    }
}

/// Build a [`Filter`] from a closure.
pub fn filter<F>(f: F) -> Filter
where
    F: Fn(&Record) -> bool + Send + Sync + 'static,
{
    Arc::new(f)
}
