//! A sink that forwards records to a user-supplied closure (loguru's callable
//! sink).

use std::sync::Arc;

use crate::format::{default_format, raw_line, Formatter};
use crate::level::Level;
use crate::record::Record;

use super::{Filter, Sink};

enum Callback {
    /// Receives the formatted log line.
    Text(Arc<dyn Fn(&str) + Send + Sync>),
    /// Receives the raw record.
    RecordRef(Arc<dyn Fn(&Record) + Send + Sync>),
}

/// A sink that calls a closure for each record.
///
/// Use [`FnSink::new`] for a closure that receives the formatted string (like
/// loguru's callable sink), or [`FnSink::from_record`] for one that receives
/// the raw [`Record`].
///
/// ```
/// use loguru::FnSink;
/// let sink = FnSink::new(|line| { /* ship `line` somewhere */ let _ = line; });
/// ```
pub struct FnSink {
    level: u16,
    colorize: bool,
    format: Formatter,
    filter: Option<Filter>,
    callback: Callback,
}

impl FnSink {
    /// Build a sink from a closure receiving the formatted log line.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        FnSink {
            level: Level::Debug.severity(),
            colorize: false,
            format: crate::format::formatter(default_format),
            filter: None,
            callback: Callback::Text(Arc::new(f)),
        }
    }

    /// Build a sink from a closure receiving the raw [`Record`].
    pub fn from_record<F>(f: F) -> Self
    where
        F: Fn(&Record) + Send + Sync + 'static,
    {
        FnSink {
            level: Level::Debug.severity(),
            colorize: false,
            format: crate::format::formatter(default_format),
            filter: None,
            callback: Callback::RecordRef(Arc::new(f)),
        }
    }

    /// Set the minimum level this sink emits.
    pub fn with_level(mut self, level: Level) -> Self {
        self.level = level.severity();
        self
    }

    /// Set the minimum severity number directly (for custom levels).
    pub fn with_level_no(mut self, no: u16) -> Self {
        self.level = no;
        self
    }

    /// Use a loguru-style format string for the formatted-line callback.
    pub fn with_format_str(mut self, template: &str, colorize: bool) -> Self {
        self.colorize = colorize;
        self.format = crate::format::from_template(template, colorize);
        self
    }

    /// Use a custom [`Formatter`] for the formatted-line callback.
    pub fn with_format(mut self, format: Formatter) -> Self {
        self.format = format;
        self
    }

    /// Only emit records for which `filter` returns `true`.
    pub fn with_filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl Sink for FnSink {
    fn level(&self) -> u16 {
        self.level
    }

    fn write(&self, record: &Record) {
        if let Some(filter) = &self.filter {
            if !filter(record) {
                return;
            }
        }
        match &self.callback {
            Callback::Text(f) => {
                let line = if record.raw {
                    raw_line(record, self.colorize)
                } else {
                    (self.format)(record)
                };
                f(&line);
            }
            Callback::RecordRef(f) => f(record),
        }
    }
}
