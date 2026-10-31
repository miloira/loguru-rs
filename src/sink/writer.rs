//! A sink that writes to any [`std::io::Write`] destination.

use std::io::Write;
use std::sync::Mutex;

use crate::format::{default_format, raw_line, Formatter};
use crate::level::Level;
use crate::record::Record;

use super::{Filter, Sink};

/// A sink that writes formatted records to any `Write` destination: an
/// in-memory `Vec<u8>`, a `TcpStream`, a pipe, a custom writer, etc. This
/// mirrors loguru accepting any file-like object as a sink.
///
/// ```
/// use loguru::{WriterSink, Logger};
/// use std::sync::{Arc, Mutex};
///
/// // Capture logs into a shared buffer.
/// let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
/// let log = Logger::builder()
///     .name("mem")
///     .sink(WriterSink::new(SharedBuf(buf.clone())))
///     .build();
/// log.info("captured");
/// log.flush();
/// assert!(String::from_utf8_lossy(&buf.lock().unwrap()).contains("captured"));
///
/// // A tiny `Write` adapter over the shared buffer.
/// struct SharedBuf(Arc<Mutex<Vec<u8>>>);
/// impl std::io::Write for SharedBuf {
///     fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().write(b) }
///     fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
/// }
/// ```
pub struct WriterSink<W: Write + Send> {
    level: u16,
    colorize: bool,
    format: Formatter,
    filter: Option<Filter>,
    writer: Mutex<W>,
}

impl<W: Write + Send> WriterSink<W> {
    /// Build a sink writing to `writer`, using the plain-text formatter at
    /// [`Level::Debug`].
    pub fn new(writer: W) -> Self {
        WriterSink {
            level: Level::Debug.severity(),
            colorize: false,
            format: crate::format::formatter(default_format),
            filter: None,
            writer: Mutex::new(writer),
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

    /// Use a loguru-style format string. `colorize` controls ANSI output.
    pub fn with_format_str(mut self, template: &str, colorize: bool) -> Self {
        self.colorize = colorize;
        self.format = crate::format::from_template(template, colorize);
        self
    }

    /// Use a custom [`Formatter`].
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

impl<W: Write + Send> Sink for WriterSink<W> {
    fn level(&self) -> u16 {
        self.level
    }

    fn write(&self, record: &Record) {
        if let Some(filter) = &self.filter {
            if !filter(record) {
                return;
            }
        }
        let mut line = if record.raw {
            raw_line(record, self.colorize)
        } else {
            (self.format)(record)
        };
        line.push('\n');
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writer.write_all(line.as_bytes());
        let _ = writer.flush();
    }

    fn flush(&self) {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writer.flush();
    }
}
