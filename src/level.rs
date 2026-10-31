//! Log severity levels, including user-defined custom levels.
//!
//! Like loguru, levels are identified by name and carry a numeric severity
//! (`no`), a color markup string, and an icon. The seven built-in levels
//! ([`Level`]) match loguru's names, numbers, colors, and icons, and you can
//! register additional custom levels with [`add_level`] / [`Logger::level`].
//!
//! [`Logger::level`]: crate::Logger::level

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

/// Full metadata for a level: its name, numeric severity, color markup, and
/// icon. This is what a [`Record`](crate::Record) carries, so custom levels
/// render and filter just like the built-in ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelInfo {
    /// The uppercase level name, e.g. `"INFO"`.
    pub name: String,
    /// The numeric severity. Higher is more severe.
    pub no: u16,
    /// loguru-style color markup applied by the `<level>` format tag, e.g.
    /// `"<green><bold>"`. Empty for no color.
    pub color: String,
    /// A short icon/emoji for the `{level.icon}` format token.
    pub icon: String,
}

impl LevelInfo {
    /// Create level metadata.
    pub fn new(
        name: impl Into<String>,
        no: u16,
        color: impl Into<String>,
        icon: impl Into<String>,
    ) -> Self {
        LevelInfo {
            name: name.into(),
            no,
            color: color.into(),
            icon: icon.into(),
        }
    }
}

/// The seven built-in severity levels (matching loguru).
///
/// Ordered by severity: `Trace < Debug < Info < Success < Warning < Error <
/// Critical`. Use [`Level::info`] to get the full [`LevelInfo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Finest-grained diagnostics (severity 5).
    Trace,
    /// Debugging information (severity 10).
    Debug,
    /// General information (severity 20).
    Info,
    /// A highlighted success (severity 25).
    Success,
    /// A warning (severity 30).
    Warning,
    /// A recoverable error (severity 40).
    Error,
    /// A severe, usually fatal error (severity 50).
    Critical,
}

impl Level {
    /// All built-in levels, least to most severe.
    pub const ALL: [Level; 7] = [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Success,
        Level::Warning,
        Level::Error,
        Level::Critical,
    ];

    /// The numeric severity of this level (matches loguru).
    #[inline]
    pub const fn severity(self) -> u16 {
        match self {
            Level::Trace => 5,
            Level::Debug => 10,
            Level::Info => 20,
            Level::Success => 25,
            Level::Warning => 30,
            Level::Error => 40,
            Level::Critical => 50,
        }
    }

    /// The uppercase name of this level, e.g. `"INFO"`.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Success => "SUCCESS",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
            Level::Critical => "CRITICAL",
        }
    }

    /// The default loguru color markup for this level.
    #[inline]
    pub const fn color(self) -> &'static str {
        match self {
            Level::Trace => "<cyan><bold>",
            Level::Debug => "<blue><bold>",
            Level::Info => "<bold>",
            Level::Success => "<green><bold>",
            Level::Warning => "<yellow><bold>",
            Level::Error => "<red><bold>",
            Level::Critical => "<RED><bold>",
        }
    }

    /// The default loguru icon for this level.
    #[inline]
    pub const fn icon(self) -> &'static str {
        match self {
            Level::Trace => "✏️",
            Level::Debug => "🐞",
            Level::Info => "ℹ️",
            Level::Success => "✅",
            Level::Warning => "⚠️",
            Level::Error => "❌",
            Level::Critical => "☠️",
        }
    }

    /// The full [`LevelInfo`] for this built-in level.
    pub fn info(self) -> LevelInfo {
        LevelInfo::new(self.name(), self.severity(), self.color(), self.icon())
    }

    /// Map a numeric severity to the closest built-in [`Level`] (used only for
    /// display of the logger's effective level).
    pub(crate) fn from_severity(sev: u16) -> Level {
        match sev {
            0..=5 => Level::Trace,
            6..=10 => Level::Debug,
            11..=20 => Level::Info,
            21..=25 => Level::Success,
            26..=30 => Level::Warning,
            31..=40 => Level::Error,
            _ => Level::Critical,
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl From<Level> for LevelInfo {
    fn from(level: Level) -> Self {
        level.info()
    }
}

/// Error returned when a string cannot be parsed into a [`Level`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseLevelError(String);

impl fmt::Display for ParseLevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown log level: {:?}", self.0)
    }
}

impl std::error::Error for ParseLevelError {}

impl FromStr for Level {
    type Err = ParseLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "TRACE" => Ok(Level::Trace),
            "DEBUG" => Ok(Level::Debug),
            "INFO" => Ok(Level::Info),
            "SUCCESS" => Ok(Level::Success),
            "WARNING" | "WARN" => Ok(Level::Warning),
            "ERROR" | "ERR" => Ok(Level::Error),
            "CRITICAL" | "CRIT" | "FATAL" => Ok(Level::Critical),
            other => Err(ParseLevelError(other.to_string())),
        }
    }
}

// --- Global level registry (custom levels) ------------------------------

fn levels() -> &'static Mutex<HashMap<String, LevelInfo>> {
    static LEVELS: OnceLock<Mutex<HashMap<String, LevelInfo>>> = OnceLock::new();
    LEVELS.get_or_init(|| {
        let mut map = HashMap::new();
        for level in Level::ALL {
            map.insert(level.name().to_string(), level.info());
        }
        Mutex::new(map)
    })
}

/// Register a new custom level, or update an existing one. Mirrors loguru's
/// `logger.level(name, no, color, icon)`.
///
/// ```
/// use loguru::{add_level, LevelInfo};
/// add_level(LevelInfo::new("NOTICE", 23, "<magenta><bold>", "📢"));
/// loguru::logger().log_by_name("NOTICE", "something noteworthy");
/// ```
pub fn add_level(info: LevelInfo) {
    let mut map = lock(levels());
    map.insert(info.name.clone(), info);
}

/// Look up level metadata by name (case-insensitive). Returns `None` if no such
/// level is registered.
pub fn get_level(name: &str) -> Option<LevelInfo> {
    let key = name.trim().to_ascii_uppercase();
    lock(levels()).get(&key).cloned()
}

/// All registered levels (built-in and custom), sorted by severity.
pub fn all_levels() -> Vec<LevelInfo> {
    let mut v: Vec<LevelInfo> = lock(levels()).values().cloned().collect();
    v.sort_by_key(|l| l.no);
    v
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
