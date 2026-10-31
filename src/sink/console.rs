//! A sink that writes to standard output or standard error.

use std::io::{IsTerminal, Write};

use crate::format::{from_template, raw_line, should_colorize, Formatter, DEFAULT_FORMAT};
use crate::level::Level;
use crate::record::Record;

use super::{Filter, Sink};

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    fn is_terminal(self) -> bool {
        match self {
            Stream::Stdout => std::io::stdout().is_terminal(),
            Stream::Stderr => std::io::stderr().is_terminal(),
        }
    }
}

/// A sink that writes formatted records to the console (stdout or stderr).
///
/// By default it writes to stderr with colors (honoring `NO_COLOR`) at
/// [`Level::Debug`], using the default loguru-style format.
///
/// ```
/// use loguru::{ConsoleSink, Level};
/// let sink = ConsoleSink::stdout()
///     .with_level(Level::Info)
///     .with_colors(false);
/// ```
pub struct ConsoleSink {
    stream: Stream,
    level: u16,
    colorize: bool,
    format: Formatter,
    filter: Option<Filter>,
}

impl ConsoleSink {
    fn new(stream: Stream) -> Self {
        // Colorize only when colors are allowed AND the destination is a real
        // terminal (so piping to a file or pager stays clean), like loguru.
        let colorize = should_colorize() && stream.is_terminal();
        ConsoleSink {
            stream,
            level: Level::Debug.severity(),
            colorize,
            format: from_template(DEFAULT_FORMAT, colorize),
            filter: None,
        }
    }

    /// A console sink writing to standard error (loguru's default destination).
    pub fn stderr() -> Self {
        Self::new(Stream::Stderr)
    }

    /// A console sink writing to standard output.
    pub fn stdout() -> Self {
        Self::new(Stream::Stdout)
    }

    /// Set the minimum level this sink emits.
    pub fn with_level(mut self, level: Level) -> Self {
        self.level = level.severity();
        self
    }

    /// Set the minimum severity number directly (useful for custom levels).
    pub fn with_level_no(mut self, no: u16) -> Self {
        self.level = no;
        self
    }

    /// Enable or disable ANSI colors, resetting the formatter to the default
    /// loguru layout (colored or stripped). Call [`ConsoleSink::with_format`]
    /// afterwards to use a custom format.
    pub fn with_colors(mut self, colorize: bool) -> Self {
        self.colorize = colorize;
        self.format = from_template(DEFAULT_FORMAT, colorize);
        self
    }

    /// Use a loguru-style format string. `colorize` controls ANSI output.
    pub fn with_format_str(mut self, template: &str, colorize: bool) -> Self {
        self.colorize = colorize;
        self.format = from_template(template, colorize);
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

impl Sink for ConsoleSink {
    fn level(&self) -> u16 {
        self.level
    }

    fn write(&self, record: &Record) {
        if let Some(filter) = &self.filter {
            if !filter(record) {
                return;
            }
        }
        let line = if record.raw {
            raw_line(record, self.colorize)
        } else {
            (self.format)(record)
        };
        match self.stream {
            Stream::Stdout => {
                let stdout = std::io::stdout();
                let mut lock = stdout.lock();
                let _ = writeln!(lock, "{line}");
            }
            Stream::Stderr => {
                let stderr = std::io::stderr();
                let mut lock = stderr.lock();
                let _ = writeln!(lock, "{line}");
            }
        }
    }

    fn flush(&self) {
        match self.stream {
            Stream::Stdout => {
                let _ = std::io::stdout().flush();
            }
            Stream::Stderr => {
                let _ = std::io::stderr().flush();
            }
        }
    }
}
