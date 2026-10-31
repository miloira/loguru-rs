//! A sink that writes to a file, with optional rotation, retention, and
//! compression.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local, Timelike};

use crate::format::{default_format, raw_line, Formatter};
use crate::level::Level;
use crate::record::Record;

use super::{Filter, Sink};

/// When a [`FileSink`] should rotate to a fresh file. Mirrors loguru's
/// `rotation`.
#[derive(Debug, Clone)]
pub enum Rotation {
    /// Rotate once the current file exceeds this many bytes.
    Size(u64),
    /// Rotate after this much wall-clock time since the last rotation.
    Interval(Duration),
    /// Rotate each day at the given local hour and minute.
    DailyAt {
        /// Hour of day, 0-23.
        hour: u32,
        /// Minute of hour, 0-59.
        minute: u32,
    },
}

/// How long rotated files are kept. Mirrors loguru's `retention`.
#[derive(Debug, Clone)]
pub enum Retention {
    /// Keep all rotated files.
    All,
    /// Keep at most this many rotated files, deleting the oldest.
    Count(usize),
    /// Delete rotated files older than this age.
    Age(Duration),
}

/// How rotated files are compressed. Mirrors loguru's `compression`.
///
/// `Gzip` requires the `compression` crate feature; without it, `Gzip` behaves
/// like `None` (rotated files are left uncompressed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// No compression.
    None,
    /// Gzip rotated files to `*.gz` (requires the `compression` feature).
    Gzip,
}

struct Inner {
    writer: Option<BufWriter<File>>,
    written: u64,
    next_rotation: Option<DateTime<Local>>,
}

/// A sink that appends formatted records to a file.
///
/// Supports size-, interval-, and daily-time-based rotation, retention by count
/// or age, and optional gzip compression of rotated files. The plain-text
/// formatter is used by default (no colors).
///
/// ```no_run
/// use loguru::{FileSink, Level, Rotation, Retention};
/// let sink = FileSink::new("app.log")?
///     .with_level(Level::Info)
///     .with_rotation(Rotation::Size(10 * 1024 * 1024))
///     .with_retention(Retention::Count(5));
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct FileSink {
    path: PathBuf,
    level: u16,
    format: Formatter,
    filter: Option<Filter>,
    rotation: Option<Rotation>,
    retention: Retention,
    compression: Compression,
    truncate: bool,
    watch: bool,
    buffer_size: usize,
    inner: Mutex<Inner>,
}

impl FileSink {
    /// Open (creating if needed, appending if present) a log file at `path`.
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let (writer, written) = open_file(&path, false, DEFAULT_BUFFER)?;
        Ok(FileSink {
            path,
            level: Level::Debug.severity(),
            format: crate::format::formatter(default_format),
            filter: None,
            rotation: None,
            retention: Retention::All,
            compression: Compression::None,
            truncate: false,
            watch: false,
            buffer_size: DEFAULT_BUFFER,
            inner: Mutex::new(Inner {
                writer: Some(writer),
                written,
                next_rotation: None,
            }),
        })
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

    /// Use a loguru-style format string (markup stripped — files are plain).
    pub fn with_format_str(mut self, template: &str) -> Self {
        self.format = crate::format::from_template(template, false);
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

    /// Set the rotation policy.
    pub fn with_rotation(mut self, rotation: Rotation) -> Self {
        match &rotation {
            Rotation::DailyAt { hour, minute } => {
                lock(&self.inner).next_rotation = Some(next_daily(Local::now(), *hour, *minute));
            }
            Rotation::Interval(interval) => {
                lock(&self.inner).next_rotation =
                    Some(Local::now() + chrono::Duration::from_std(*interval).unwrap_or_default());
            }
            Rotation::Size(_) => {}
        }
        self.rotation = Some(rotation);
        self
    }

    /// Set the retention policy.
    pub fn with_retention(mut self, retention: Retention) -> Self {
        self.retention = retention;
        self
    }

    /// Set the compression applied to rotated files.
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Truncate the file on open instead of appending (loguru's `mode="w"`).
    /// Reopens the file immediately.
    pub fn with_truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        if truncate {
            if let Ok((writer, written)) = open_file(&self.path, true, self.buffer_size) {
                let mut inner = lock(&self.inner);
                inner.writer = Some(writer);
                inner.written = written;
            }
        }
        self
    }

    /// Reopen the file if it is removed or replaced externally (e.g. by an
    /// external logrotate). Mirrors loguru's `watch=True`. Adds a lightweight
    /// existence check before each write.
    pub fn with_watch(mut self, watch: bool) -> Self {
        self.watch = watch;
        self
    }

    /// Set the write buffer size in bytes (loguru's `buffering`). Takes effect
    /// on the next (re)open; use `1` for effectively line-buffered output.
    pub fn with_buffer_size(mut self, bytes: usize) -> Self {
        self.buffer_size = bytes.max(1);
        if let Ok((writer, written)) = open_file(&self.path, false, self.buffer_size) {
            let mut inner = lock(&self.inner);
            inner.writer = Some(writer);
            inner.written = written;
        }
        self
    }

    /// Defer opening the file until the first record is written (loguru's
    /// `delay=True`).
    pub fn with_delay(self, delay: bool) -> Self {
        if delay {
            lock(&self.inner).writer = None;
        }
        self
    }

    fn should_rotate(&self, inner: &Inner, incoming: usize, now: DateTime<Local>) -> bool {
        match &self.rotation {
            None => false,
            Some(Rotation::Size(max)) => inner.written > 0 && inner.written + incoming as u64 > *max,
            Some(Rotation::Interval(_)) | Some(Rotation::DailyAt { .. }) => {
                matches!(inner.next_rotation, Some(next) if now >= next)
            }
        }
    }

    fn rotate(&self, inner: &mut Inner, now: DateTime<Local>) -> io::Result<()> {
        if let Some(writer) = inner.writer.as_mut() {
            writer.flush()?;
        }

        let stem = self.path.file_stem().and_then(|s| s.to_str()).unwrap_or("log");
        let ext = self.path.extension().and_then(|s| s.to_str());
        let stamp = now.format("%Y-%m-%d_%H-%M-%S%.3f").to_string();
        let rotated_name = match ext {
            Some(ext) => format!("{stem}.{stamp}.{ext}"),
            None => format!("{stem}.{stamp}"),
        };
        let rotated_path = match self.path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join(rotated_name),
            _ => PathBuf::from(rotated_name),
        };

        fs::rename(&self.path, &rotated_path)?;

        if self.compression == Compression::Gzip {
            if let Err(e) = compress_gzip(&rotated_path) {
                let _ = writeln!(io::stderr(), "loguru: compression failed: {e}");
            }
        }

        let (writer, written) = open_file(&self.path, self.truncate, self.buffer_size)?;
        inner.writer = Some(writer);
        inner.written = written;
        inner.next_rotation = match &self.rotation {
            Some(Rotation::Interval(interval)) => {
                Some(now + chrono::Duration::from_std(*interval).unwrap_or_default())
            }
            Some(Rotation::DailyAt { hour, minute }) => Some(next_daily(now, *hour, *minute)),
            _ => None,
        };

        self.enforce_retention(stem, ext);
        Ok(())
    }

    fn enforce_retention(&self, stem: &str, ext: Option<&str>) {
        if matches!(self.retention, Retention::All) {
            return;
        }
        let mut rotated = self.list_rotated(stem, ext);
        rotated.sort_by_key(|(_, t)| *t); // oldest first

        match &self.retention {
            Retention::All => {}
            Retention::Count(count) => {
                while rotated.len() > *count {
                    let (victim, _) = rotated.remove(0);
                    let _ = fs::remove_file(victim);
                }
            }
            Retention::Age(age) => {
                let cutoff = SystemTime::now()
                    .checked_sub(*age)
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                for (path, modified) in rotated {
                    if modified < cutoff {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
    }

    fn list_rotated(&self, stem: &str, ext: Option<&str>) -> Vec<(PathBuf, SystemTime)> {
        let dir = match self.path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let prefix = format!("{stem}.");
        match fs::read_dir(&dir) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p != &self.path)
                .filter(|p| {
                    let name = match p.file_name().and_then(|s| s.to_str()) {
                        Some(n) => n,
                        None => return false,
                    };
                    if !name.starts_with(&prefix) {
                        return false;
                    }
                    match ext {
                        // Accept both the rotated file and its compressed form.
                        Some(ext) => {
                            name.ends_with(&format!(".{ext}"))
                                || name.ends_with(&format!(".{ext}.gz"))
                        }
                        None => true,
                    }
                })
                .map(|p| {
                    let t = fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    (p, t)
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

fn next_daily(from: DateTime<Local>, hour: u32, minute: u32) -> DateTime<Local> {
    let today = from
        .with_hour(hour)
        .and_then(|d| d.with_minute(minute))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(from);
    if today > from {
        today
    } else {
        today + chrono::Duration::days(1)
    }
}

const DEFAULT_BUFFER: usize = 8 * 1024;

fn open_file(path: &Path, truncate: bool, buffer_size: usize) -> io::Result<(BufWriter<File>, u64)> {
    let mut opts = OpenOptions::new();
    opts.create(true).write(true);
    if truncate {
        opts.truncate(true);
    } else {
        opts.append(true);
    }
    let file = opts.open(path)?;
    let written = if truncate {
        0
    } else {
        file.metadata().map(|m| m.len()).unwrap_or(0)
    };
    Ok((BufWriter::with_capacity(buffer_size.max(1), file), written))
}

#[cfg(feature = "compression")]
fn compress_gzip(path: &Path) -> io::Result<()> {
    use flate2::write::GzEncoder;
    use flate2::Compression as GzLevel;

    let gz_path = {
        let mut s = path.as_os_str().to_os_string();
        s.push(".gz");
        PathBuf::from(s)
    };
    let input = fs::read(path)?;
    let out = File::create(&gz_path)?;
    let mut encoder = GzEncoder::new(BufWriter::new(out), GzLevel::default());
    encoder.write_all(&input)?;
    encoder.finish()?;
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(not(feature = "compression"))]
fn compress_gzip(_path: &Path) -> io::Result<()> {
    // The `compression` feature is disabled; leave the rotated file as-is.
    Ok(())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Sink for FileSink {
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
            raw_line(record, false)
        } else {
            (self.format)(record)
        };
        line.push('\n');
        let bytes = line.as_bytes();

        let mut inner = lock(&self.inner);
        let now = Local::now();

        // watch=true: if the file vanished (external move/delete), reopen it.
        if self.watch && inner.writer.is_some() && !self.path.exists() {
            if let Ok((writer, written)) = open_file(&self.path, false, self.buffer_size) {
                inner.writer = Some(writer);
                inner.written = written;
            }
        }

        if self.should_rotate(&inner, bytes.len(), now) {
            if let Err(e) = self.rotate(&mut inner, now) {
                let _ = writeln!(
                    io::stderr(),
                    "loguru: file rotation failed for {}: {e}",
                    self.path.display()
                );
            }
        }

        // Lazily open the file on first write (delay=true).
        if inner.writer.is_none() {
            match open_file(&self.path, self.truncate, self.buffer_size) {
                Ok((writer, written)) => {
                    inner.writer = Some(writer);
                    inner.written = written;
                }
                Err(e) => {
                    let _ = writeln!(
                        io::stderr(),
                        "loguru: could not open {}: {e}",
                        self.path.display()
                    );
                    return;
                }
            }
        }

        if let Some(writer) = inner.writer.as_mut() {
            if writer.write_all(bytes).is_ok() {
                let _ = writer.flush();
                inner.written += bytes.len() as u64;
            }
        }
    }

    fn flush(&self) {
        let mut inner = lock(&self.inner);
        if let Some(writer) = inner.writer.as_mut() {
            let _ = writer.flush();
        }
    }
}
