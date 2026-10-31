//! The [`Logger`] type and its builder.

use std::collections::BTreeMap;
use std::fmt::{self, Display};
use std::panic::{self, UnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use chrono::Local;

use crate::context::{self, ContextGuard};
use crate::level::{Level, LevelInfo};
use crate::record::{current_thread_id, Location, Record};
use crate::registry;
use crate::sink::Sink;

/// Severity 0 means "not set" — inherit the effective level from an ancestor.
const NOTSET: u16 = 0;

/// The shared, mutable heart of a logger. Multiple [`Logger`] handles can
/// share one `Core` (via `bind`/`opt`/`patch`), so they reflect each other's
/// sink and level changes.
struct Core {
    name: Arc<str>,
    level: AtomicU16,
    propagate: AtomicBool,
    registered: bool,
    sinks: RwLock<Vec<(usize, Arc<dyn Sink>)>>,
    sink_counter: AtomicUsize,
}

impl Core {
    fn dispatch(&self, record: &Record) {
        let sinks = read(&self.sinks);
        for (_, sink) in sinks.iter() {
            if record.level.no >= sink.level() {
                sink.write(record);
            }
        }
    }

    fn for_each_sink(&self, f: impl Fn(&Arc<dyn Sink>)) {
        let sinks = read(&self.sinks);
        for (_, sink) in sinks.iter() {
            f(sink);
        }
    }
}

/// A record patcher installed via [`Logger::patch`].
type Patcher = Arc<dyn Fn(&mut Record) + Send + Sync>;

/// Per-handle options set via [`Logger::opt`] (and friends). Mirrors the flags
/// of loguru's `logger.opt(...)`.
#[derive(Clone, Default)]
struct Opts {
    /// Parse color markup inside the message itself.
    colors: bool,
    /// Attach a captured backtrace to the record.
    backtrace: bool,
    /// Write the message raw, bypassing the sink formatter.
    raw: bool,
}

/// A logger: a named set of [`Sink`]s with a severity threshold, bound context,
/// and per-handle options.
///
/// Unlike loguru's single global logger, you can have as many independent
/// loggers as you like:
///
/// * [`crate::get_logger`] returns a **named, registered** logger from a global
///   registry (like `logging.getLogger`), forming a dotted hierarchy with
///   propagation to ancestors.
/// * [`Logger::builder`] / [`Logger::new`] create a **standalone** logger that
///   is not in the registry and does not propagate.
///
/// A `Logger` handle is cheap to clone; cloning (and `bind`/`opt`/`patch`)
/// produces a new handle sharing the same underlying sinks and level.
#[derive(Clone)]
pub struct Logger {
    core: Arc<Core>,
    extra: BTreeMap<String, String>,
    opts: Opts,
    patcher: Option<Patcher>,
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|e| e.into_inner())
}

impl Logger {
    pub(crate) fn new_core(
        name: impl Into<Arc<str>>,
        level: u16,
        registered: bool,
        propagate: bool,
        sinks: Vec<Arc<dyn Sink>>,
    ) -> Arc<Logger> {
        let indexed: Vec<(usize, Arc<dyn Sink>)> = sinks.into_iter().enumerate().collect();
        let counter = indexed.len();
        Arc::new(Logger {
            core: Arc::new(Core {
                name: name.into(),
                level: AtomicU16::new(level),
                propagate: AtomicBool::new(propagate),
                registered,
                sinks: RwLock::new(indexed),
                sink_counter: AtomicUsize::new(counter),
            }),
            extra: BTreeMap::new(),
            opts: Opts::default(),
            patcher: None,
        })
    }

    /// Start building a standalone logger.
    pub fn builder() -> LoggerBuilder {
        LoggerBuilder::new()
    }

    /// Create a standalone logger with the given name and a single colored
    /// stderr console sink, at [`Level::Debug`] and above.
    pub fn new(name: impl Into<Arc<str>>) -> Arc<Logger> {
        LoggerBuilder::new()
            .name(name)
            .sink(crate::sink::ConsoleSink::stderr())
            .build()
    }

    /// The name of this logger (empty for the root logger).
    pub fn name(&self) -> &str {
        &self.core.name
    }

    /// Whether this logger is part of the global registry hierarchy.
    pub fn is_registered(&self) -> bool {
        self.core.registered
    }

    // --- Level & propagation --------------------------------------------

    /// Set the minimum level this logger emits.
    pub fn set_level(&self, level: Level) {
        self.core.level.store(level.severity(), Ordering::Relaxed);
    }

    /// Set the minimum severity number directly (for custom levels).
    pub fn set_level_no(&self, no: u16) {
        self.core.level.store(no, Ordering::Relaxed);
    }

    /// Clear this logger's own level so it inherits from its ancestors
    /// (registry loggers only; no effect on the root).
    pub fn unset_level(&self) {
        if self.core.registered && !self.core.name.is_empty() {
            self.core.level.store(NOTSET, Ordering::Relaxed);
        }
    }

    /// The effective level (own, or inherited from ancestors, default INFO).
    pub fn effective_level(&self) -> Level {
        Level::from_severity(self.effective_severity())
    }

    fn effective_severity(&self) -> u16 {
        let own = self.core.level.load(Ordering::Relaxed);
        if own != NOTSET {
            return own;
        }
        if self.core.registered {
            if let Some(sev) = registry::effective_ancestor_severity(&self.core.name) {
                return sev;
            }
        }
        Level::Info.severity()
    }

    /// Enable or disable propagation to ancestor loggers' sinks.
    pub fn set_propagate(&self, propagate: bool) {
        self.core.propagate.store(propagate, Ordering::Relaxed);
    }

    /// Whether records propagate to ancestor loggers' sinks.
    pub fn propagates(&self) -> bool {
        self.core.propagate.load(Ordering::Relaxed)
    }

    /// Whether a record at `level` meets this logger's effective level.
    #[inline]
    pub fn is_enabled(&self, level: Level) -> bool {
        self.is_enabled_no(level.severity())
    }

    /// Whether a record at severity `no` meets this logger's effective level.
    #[inline]
    pub fn is_enabled_no(&self, no: u16) -> bool {
        no >= self.effective_severity()
    }

    // --- Sink management ------------------------------------------------

    /// Add a sink and return an id for [`Logger::remove`]. Mirrors loguru's
    /// `logger.add(...)`.
    pub fn add<S: Sink + 'static>(&self, sink: S) -> usize {
        self.add_arc(Arc::new(sink))
    }

    /// Add an already-shared sink, returning its removal id.
    pub fn add_arc(&self, sink: Arc<dyn Sink>) -> usize {
        let id = self.core.sink_counter.fetch_add(1, Ordering::Relaxed);
        write(&self.core.sinks).push((id, sink));
        id
    }

    /// Remove the sink with the given id. Mirrors `logger.remove(id)`.
    pub fn remove(&self, id: usize) -> bool {
        let mut sinks = write(&self.core.sinks);
        let before = sinks.len();
        sinks.retain(|(sink_id, _)| *sink_id != id);
        sinks.len() != before
    }

    /// Remove all sinks. Mirrors `logger.remove()` with no argument.
    pub fn remove_all(&self) {
        write(&self.core.sinks).clear();
    }

    /// The number of sinks currently attached.
    pub fn sink_count(&self) -> usize {
        read(&self.core.sinks).len()
    }

    /// Flush all sinks.
    pub fn flush(&self) {
        self.core.for_each_sink(|s| s.flush());
    }

    /// Wait for all async sinks to drain and flush. Mirrors `logger.complete()`.
    pub fn complete(&self) {
        self.core.for_each_sink(|s| s.complete());
    }

    // --- Custom levels --------------------------------------------------

    /// Register (or update) a custom level and return its metadata. Mirrors
    /// loguru's `logger.level(name, no, color, icon)`.
    pub fn level(
        &self,
        name: impl Into<String>,
        no: u16,
        color: impl Into<String>,
        icon: impl Into<String>,
    ) -> LevelInfo {
        let info = LevelInfo::new(name, no, color, icon);
        crate::level::add_level(info.clone());
        info
    }

    // --- Context, options, patching -------------------------------------

    /// Return a new handle sharing this logger's core but carrying additional
    /// context fields, merged into every record. Mirrors `logger.bind(...)`.
    ///
    /// ```
    /// let log = loguru::get_logger("app");
    /// let req = log.bind([("request_id", "abc123")]);
    /// req.info("handling request");
    /// ```
    pub fn bind<I, K, V>(&self, fields: I) -> Logger
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut next = self.clone();
        for (k, v) in fields {
            next.extra.insert(k.into(), v.into());
        }
        next
    }

    /// Push thread-local context fields active until the returned guard drops.
    /// Mirrors `logger.contextualize(**kwargs)`.
    ///
    /// ```
    /// let log = loguru::get_logger("app");
    /// {
    ///     let _ctx = log.contextualize([("request_id", "abc123")]);
    ///     log.info("inside the context");
    /// } // request_id removed here
    /// ```
    pub fn contextualize<I, K, V>(&self, fields: I) -> ContextGuard
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

    /// Return a handle whose records are modified by `patcher` just before
    /// dispatch. Mirrors `logger.patch(...)`.
    pub fn patch<F>(&self, patcher: F) -> Logger
    where
        F: Fn(&mut Record) + Send + Sync + 'static,
    {
        let mut next = self.clone();
        next.patcher = Some(Arc::new(patcher));
        next
    }

    /// Return a handle, as a starting point for the `opt` chain. Equivalent to
    /// cloning the handle. Mirrors `logger.opt(...)`.
    pub fn opt(&self) -> Logger {
        self.clone()
    }

    /// Return a handle that parses color markup inside the message text.
    /// Mirrors `logger.opt(colors=True)`.
    pub fn colors(&self, yes: bool) -> Logger {
        let mut next = self.clone();
        next.opts.colors = yes;
        next
    }

    /// Return a handle that attaches a captured backtrace to each record.
    /// Mirrors `logger.opt(exception=True)`.
    pub fn with_backtrace(&self, yes: bool) -> Logger {
        let mut next = self.clone();
        next.opts.backtrace = yes;
        next
    }

    /// Return a handle that writes the message raw, bypassing the sink
    /// formatter (no timestamp/level/location). Mirrors `logger.opt(raw=True)`.
    pub fn raw(&self, yes: bool) -> Logger {
        let mut next = self.clone();
        next.opts.raw = yes;
        next
    }

    // --- Emission -------------------------------------------------------

    fn emit(&self, level: LevelInfo, location: Location, message: String, exception: Option<String>) {
        if !context::is_enabled(&self.core.name) {
            return;
        }

        let mut extra = BTreeMap::new();
        context::merge_context(&mut extra);
        for (k, v) in &self.extra {
            extra.insert(k.clone(), v.clone());
        }

        let exception = exception.or_else(|| {
            if self.opts.backtrace {
                Some(capture_backtrace())
            } else {
                None
            }
        });

        let mut record = Record {
            level,
            message,
            logger_name: Arc::clone(&self.core.name),
            location,
            time: Local::now(),
            elapsed: context::start_instant().elapsed(),
            process_id: std::process::id(),
            thread_id: current_thread_id(),
            thread_name: std::thread::current().name().map(str::to_owned),
            extra,
            exception,
            colored_message: self.opts.colors,
            raw: self.opts.raw,
        };

        if let Some(patcher) = &self.patcher {
            patcher(&mut record);
        }

        self.core.dispatch(&record);

        if self.core.registered && self.core.propagate.load(Ordering::Relaxed) {
            registry::propagate(&self.core.name, &record);
        }
    }

    /// Low-level macro entry point. Prefer the macros or per-level methods.
    #[doc(hidden)]
    pub fn __log_fmt(
        &self,
        level: Level,
        module_path: &'static str,
        function: &'static str,
        file: &'static str,
        line: u32,
        args: fmt::Arguments<'_>,
    ) {
        if !self.is_enabled(level) {
            return;
        }
        self.emit(
            level.info(),
            Location {
                module_path,
                function,
                file,
                line,
            },
            fmt::format(args),
            None,
        );
    }

    /// Log a message at an explicit [`Level`]. Captures the caller's file and
    /// line via `#[track_caller]`; the module path is captured only by macros.
    #[track_caller]
    pub fn log(&self, level: Level, message: impl Display) {
        if !self.is_enabled(level) {
            return;
        }
        self.emit(level.info(), caller_location(), message.to_string(), None);
    }

    /// Log a message at a custom level, looked up by name (case-insensitive).
    /// If the level is unknown, the message is dropped. Mirrors `logger.log`.
    #[track_caller]
    pub fn log_by_name(&self, level_name: &str, message: impl Display) {
        let info = match crate::level::get_level(level_name) {
            Some(info) => info,
            None => return,
        };
        if !self.is_enabled_no(info.no) {
            return;
        }
        self.emit(info, caller_location(), message.to_string(), None);
    }

    /// Log lazily: `build` is only called if the level is enabled. Mirrors
    /// loguru's `logger.opt(lazy=True)` for expensive message construction.
    ///
    /// ```
    /// let log = loguru::get_logger("app");
    /// log.log_lazy(loguru::Level::Debug, || expensive_debug_dump());
    /// # fn expensive_debug_dump() -> String { String::new() }
    /// ```
    #[track_caller]
    pub fn log_lazy<F, D>(&self, level: Level, build: F)
    where
        F: FnOnce() -> D,
        D: Display,
    {
        if !self.is_enabled(level) {
            return;
        }
        self.emit(level.info(), caller_location(), build().to_string(), None);
    }

    /// Emit a record at an explicit, owned location (used by the global panic
    /// hook, whose `&str` paths are not `'static`).
    pub(crate) fn emit_owned(
        &self,
        level: Level,
        message: String,
        exception: Option<String>,
    ) {
        if !self.is_enabled(level) {
            return;
        }
        self.emit(level.info(), Location::UNKNOWN, message, exception);
    }

    /// Log a message at [`Level::Error`] with a captured backtrace attached.
    /// Mirrors `logger.exception(...)`.
    #[track_caller]
    pub fn exception(&self, message: impl Display) {
        if !self.is_enabled(Level::Error) {
            return;
        }
        self.emit(
            Level::Error.info(),
            caller_location(),
            message.to_string(),
            Some(capture_backtrace()),
        );
    }

    /// Log a message at [`Level::Error`], attaching `error` and its full
    /// `source()` chain (plus a backtrace) as the record's exception text.
    ///
    /// ```
    /// use std::io::{Error, ErrorKind};
    /// let log = loguru::get_logger("app");
    /// let err = Error::new(ErrorKind::NotFound, "config.toml missing");
    /// log.exception_with(&err, "failed to start");
    /// ```
    #[track_caller]
    pub fn exception_with(&self, error: &dyn std::error::Error, message: impl Display) {
        if !self.is_enabled(Level::Error) {
            return;
        }
        let mut detail = format_error_chain(error);
        detail.push('\n');
        detail.push_str(&capture_backtrace());
        self.emit(
            Level::Error.info(),
            caller_location(),
            message.to_string(),
            Some(detail),
        );
    }

    /// Run `f`, logging any panic at [`Level::Error`] (with a backtrace) and
    /// swallowing it. Returns `Some(value)` on success, `None` if `f` panicked.
    /// Mirrors loguru's `logger.catch`.
    ///
    /// For options (reraise, a different level, a custom message), use
    /// [`Logger::catch_opts`].
    #[track_caller]
    pub fn catch<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce() -> R + UnwindSafe,
    {
        self.catch_opts().run(f)
    }

    /// Start a configurable [`Catch`] (reraise, level, message), mirroring the
    /// options of loguru's `logger.catch(...)`.
    ///
    /// ```
    /// use loguru::Level;
    /// let log = loguru::get_logger("app");
    /// let out = log.catch_opts()
    ///     .level(Level::Critical)
    ///     .message("task failed")
    ///     .run(|| 1 + 1);
    /// assert_eq!(out, Some(2));
    /// ```
    #[track_caller]
    pub fn catch_opts(&self) -> Catch<'_> {
        Catch {
            logger: self,
            level: Level::Error,
            reraise: false,
            message: "An error has been caught".to_string(),
            location: caller_location(),
            on_error: None,
        }
    }
}

/// A callback invoked with the panic message when [`Catch`] catches an error.
type OnError<'a> = Box<dyn FnOnce(&str) + 'a>;

/// A configurable error catcher created by [`Logger::catch_opts`]. Mirrors the
/// options of loguru's `logger.catch(...)`.
pub struct Catch<'a> {
    logger: &'a Logger,
    level: Level,
    reraise: bool,
    message: String,
    location: Location,
    on_error: Option<OnError<'a>>,
}

impl<'a> Catch<'a> {
    /// The level to log a caught panic at (default [`Level::Error`]).
    pub fn level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Whether to re-raise (resume unwinding) after logging (default `false`).
    pub fn reraise(mut self, reraise: bool) -> Self {
        self.reraise = reraise;
        self
    }

    /// The message prefix logged before the panic text.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// A callback run with the panic message after it is logged. Mirrors
    /// loguru's `onerror`.
    pub fn on_error<C>(mut self, callback: C) -> Self
    where
        C: FnOnce(&str) + 'a,
    {
        self.on_error = Some(Box::new(callback));
        self
    }

    fn handle(self, payload: Box<dyn std::any::Any + Send>) {
        let msg = panic_message(&*payload);
        self.logger.emit(
            self.level.info(),
            self.location.clone(),
            format!("{}: {msg}", self.message),
            Some(capture_backtrace()),
        );
        if let Some(callback) = self.on_error {
            callback(&msg);
        }
        if self.reraise {
            panic::resume_unwind(payload);
        }
    }

    /// Run `f`, catching and logging any panic. Returns `Some(value)` on
    /// success, or `None` if `f` panicked (unless `reraise` is set, in which
    /// case the panic resumes after logging).
    pub fn run<F, R>(self, f: F) -> Option<R>
    where
        F: FnOnce() -> R + UnwindSafe,
    {
        match panic::catch_unwind(f) {
            Ok(value) => Some(value),
            Err(payload) => {
                self.handle(payload);
                None
            }
        }
    }

    /// Run `f`, returning its value, or `default` if it panicked (after logging).
    /// Mirrors loguru's `default`.
    pub fn run_or<F, R>(self, default: R, f: F) -> R
    where
        F: FnOnce() -> R + UnwindSafe,
    {
        match panic::catch_unwind(f) {
            Ok(value) => value,
            Err(payload) => {
                self.handle(payload);
                default
            }
        }
    }
}

#[track_caller]
fn caller_location() -> Location {
    let caller = std::panic::Location::caller();
    Location {
        module_path: "",
        function: "",
        file: caller.file(),
        line: caller.line(),
    }
}

fn capture_backtrace() -> String {
    std::backtrace::Backtrace::capture().to_string()
}

fn format_error_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(err) = source {
        out.push_str("\nCaused by: ");
        out.push_str(&err.to_string());
        source = err.source();
    }
    out
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".to_string()
    }
}

/// Generate the per-level convenience methods (`trace`, `debug`, ...).
macro_rules! level_methods {
    ($($method:ident => $level:ident),+ $(,)?) => {
        impl Logger {
            $(
                #[doc = concat!("Log a message at the [`Level::", stringify!($level), "`] level.")]
                #[track_caller]
                pub fn $method(&self, message: impl Display) {
                    self.log(Level::$level, message);
                }
            )+
        }
    };
}

level_methods! {
    trace => Trace,
    debug => Debug,
    info => Info,
    success => Success,
    warning => Warning,
    error => Error,
    critical => Critical,
}

impl fmt::Debug for Logger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Logger")
            .field("name", &self.core.name)
            .field("level", &self.effective_level())
            .field("registered", &self.core.registered)
            .field("sinks", &self.sink_count())
            .field("extra", &self.extra)
            .finish()
    }
}

/// A builder for standalone [`Logger`]s.
pub struct LoggerBuilder {
    name: Arc<str>,
    level: Level,
    propagate: bool,
    sinks: Vec<Arc<dyn Sink>>,
}

impl LoggerBuilder {
    fn new() -> Self {
        LoggerBuilder {
            name: Arc::from(""),
            level: Level::Debug,
            propagate: false,
            sinks: Vec::new(),
        }
    }

    /// Set the logger's name (used in the default log format).
    pub fn name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the logger's minimum level (default [`Level::Debug`]).
    pub fn level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Add a sink.
    pub fn sink<S: Sink + 'static>(mut self, sink: S) -> Self {
        self.sinks.push(Arc::new(sink));
        self
    }

    /// Add an already-shared sink.
    pub fn sink_arc(mut self, sink: Arc<dyn Sink>) -> Self {
        self.sinks.push(sink);
        self
    }

    /// Build the standalone logger.
    pub fn build(self) -> Arc<Logger> {
        Logger::new_core(
            self.name,
            self.level.severity(),
            false,
            self.propagate,
            self.sinks,
        )
    }
}

impl Default for LoggerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Lets the logging macros accept a `Logger`, `&Logger`, or `Arc<Logger>` (and
/// references to them) as the target.
pub trait AsLogger {
    /// Borrow the underlying [`Logger`].
    fn as_logger(&self) -> &Logger;
}

impl AsLogger for Logger {
    #[inline]
    fn as_logger(&self) -> &Logger {
        self
    }
}

impl AsLogger for Arc<Logger> {
    #[inline]
    fn as_logger(&self) -> &Logger {
        self
    }
}

impl<T: AsLogger + ?Sized> AsLogger for &T {
    #[inline]
    fn as_logger(&self) -> &Logger {
        (**self).as_logger()
    }
}

// Registry helpers.
impl Logger {
    pub(crate) fn dispatch_only(&self, record: &Record) {
        self.core.dispatch(record);
    }

    pub(crate) fn stored_severity(&self) -> u16 {
        self.core.level.load(Ordering::Relaxed)
    }
}
