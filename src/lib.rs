//! # loguru
//!
//! An ergonomic logging library for Rust, a faithful port of Python's
//! [loguru](https://github.com/Delgan/loguru), extended with the
//! multi-instance, named-logger model of Python's standard `logging` module.
//!
//! loguru (the Python library) is delightful but exposes a *single* global
//! `logger`. This crate keeps loguru's feature set — the `SUCCESS` level and
//! custom levels with icons, a color-markup format DSL, pluggable sinks with
//! per-sink level/format/filter, file rotation and retention, async
//! (`enqueue`) sinks, `bind`/`contextualize`/`patch`/`opt`/`catch`/
//! `exception`/`complete`, and `enable`/`disable` — while fixing the
//! single-logger limitation the way `logging` does: a registry of **named
//! loggers** in a dotted hierarchy, plus fully independent **standalone**
//! loggers.
//!
//! ## Quick start
//!
//! ```
//! loguru::info!("hello from {}", "loguru-rs");
//! loguru::success!("done in {} ms", 42);
//! loguru::warning!("disk {}% full", 87);
//! ```
//!
//! ## Named loggers (like `logging.getLogger`)
//!
//! ```
//! use loguru::{get_logger, Level};
//! let db = get_logger("app.db");
//! db.set_level(Level::Debug);
//! db.info("connected");
//! loguru::info!(to: db, "query took {} ms", 12);
//! ```
//!
//! ## Standalone loggers
//!
//! ```no_run
//! use loguru::{Logger, Level, FileSink, ConsoleSink};
//! let audit = Logger::builder()
//!     .name("audit")
//!     .level(Level::Info)
//!     .sink(FileSink::new("audit.log")?)
//!     .build();
//! audit.info("user logged in");
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! ## The format DSL
//!
//! Format strings mix color markup (`<green>`, `<level>`, `</>`) with field
//! placeholders (`{time:YYYY-MM-DD HH:mm:ss.SSS}`, `{level: <8}`, `{name}`,
//! `{function}`, `{line}`, `{message}`, `{extra[key]}`). See [`DEFAULT_FORMAT`].

#![warn(missing_docs)]

pub mod format;

mod context;
mod level;
mod logger;
#[macro_use]
mod macros;
mod record;
mod registry;
mod sink;

use std::sync::Arc;

pub use context::{disable, enable, ContextGuard};
pub use format::{
    colorize, default_format, default_format_colored, formatter, from_template, json_format,
    should_colorize, Formatter, DEFAULT_FORMAT,
};
pub use level::{add_level, all_levels, get_level, Level, LevelInfo, ParseLevelError};
pub use logger::{AsLogger, Catch, Logger, LoggerBuilder};
pub use record::{Location, Record};
pub use sink::{
    filter, AsyncSink, Compression, ConsoleSink, FileSink, Filter, FnSink, Retention, Rotation,
    Sink, WriterSink,
};

/// The global root logger, used by the bare logging macros (`loguru::info!`)
/// and the crate-level convenience functions. Writes colored output to stderr
/// at [`Level::Info`] by default.
pub fn logger() -> Arc<Logger> {
    registry::root()
}

/// Fetch (creating on first use) the registered logger with the given dotted
/// `name`, like `logging.getLogger(name)`. The empty name returns the root.
pub fn get_logger(name: impl AsRef<str>) -> Arc<Logger> {
    registry::get_logger(name)
}

/// Set the level of the global root logger.
pub fn set_level(level: Level) {
    logger().set_level(level);
}

/// Add a sink to the global root logger, returning an id for [`remove`].
pub fn add<S: Sink + 'static>(sink: S) -> usize {
    logger().add(sink)
}

/// Add an already-shared sink to the global root logger.
pub fn add_arc(sink: Arc<dyn Sink>) -> usize {
    logger().add_arc(sink)
}

/// Remove a sink from the global root logger by id.
pub fn remove(id: usize) -> bool {
    logger().remove(id)
}

/// Remove all sinks from the global root logger, including the default stderr
/// sink. Mirrors loguru's `logger.remove()` — a common first step before
/// adding your own configured sinks.
pub fn remove_all() {
    logger().remove_all();
}

/// Flush the global root logger's sinks.
pub fn flush() {
    logger().flush();
}

/// Wait for the global root logger's async sinks to drain.
pub fn complete() {
    logger().complete();
}

/// Push thread-local context fields active until the returned guard drops.
/// Mirrors loguru's `logger.contextualize(**kwargs)`. The context is global
/// (not tied to a particular logger).
pub fn contextualize<I, K, V>(fields: I) -> ContextGuard
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let map = fields
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    ContextGuard::push(map)
}

/// Bulk-reconfigure the global root logger, like loguru's
/// `logger.configure(handlers=..., levels=...)`.
///
/// If `remove_existing` is true, all current root sinks (including the default)
/// are removed first. Each sink in `sinks` is added. If `level` is given, it is
/// applied to the root logger. Returns the ids of the added sinks.
pub fn configure(
    remove_existing: bool,
    level: Option<Level>,
    sinks: Vec<Arc<dyn Sink>>,
) -> Vec<usize> {
    let root = logger();
    if remove_existing {
        root.remove_all();
    }
    if let Some(level) = level {
        root.set_level(level);
    }
    sinks.into_iter().map(|s| root.add_arc(s)).collect()
}

/// Install a global panic hook that logs every uncaught panic through the root
/// logger at [`Level::Critical`], with the panic location and a backtrace,
/// before the process aborts/unwinds. Analogous to loguru overriding the
/// exception hook.
///
/// This replaces the current panic hook. Call once, early in `main`.
pub fn install_panic_hook() {
    let logger = logger();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "Box<dyn Any>".to_string()
        };
        let location = match info.location() {
            Some(loc) => format!(" at {}:{}:{}", loc.file(), loc.line(), loc.column()),
            None => String::new(),
        };
        let backtrace = std::backtrace::Backtrace::capture().to_string();
        logger.emit_owned(
            Level::Critical,
            format!("Uncaught panic{location}: {msg}"),
            Some(backtrace),
        );
    }));
}

/// A convenient glob-import of the crate's most-used items.
pub mod prelude {
    pub use crate::{
        critical, debug, error, info, log, success, trace, warning, AsLogger, AsyncSink,
        ConsoleSink, FileSink, FnSink, Level, LevelInfo, Logger, Sink,
    };
}
