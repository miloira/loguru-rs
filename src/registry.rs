//! The global logger registry — a named, hierarchical set of loggers in the
//! style of Python's `logging.Manager`.
//!
//! Loggers are keyed by dotted name (`"app"`, `"app.db"`, `"app.db.pool"`).
//! The root logger has the empty name `""`. A record emitted by a logger is,
//! by default, also dispatched to the sinks of every ancestor logger that
//! exists (propagation), but ancestor loggers' *levels* do not re-filter it —
//! only the originating logger's effective level gates emission, matching
//! Python's `logging`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::level::Level;
use crate::logger::Logger;
use crate::record::Record;
use crate::sink::ConsoleSink;

struct Manager {
    root: Arc<Logger>,
    loggers: Mutex<HashMap<String, Arc<Logger>>>,
}

static MANAGER: OnceLock<Manager> = OnceLock::new();

fn manager() -> &'static Manager {
    MANAGER.get_or_init(|| {
        // The root logger starts with a single colored stderr console sink so
        // that logging works out of the box (loguru-style), at INFO and above.
        let root = Logger::new_core(
            "",
            Level::Info.severity(),
            true,
            true,
            vec![Arc::new(ConsoleSink::stderr())],
        );
        let mut loggers = HashMap::new();
        loggers.insert(String::new(), Arc::clone(&root));
        Manager {
            root,
            loggers: Mutex::new(loggers),
        }
    })
}

/// The root logger. This is the logger used by the bare logging macros and by
/// the crate-level convenience functions.
pub fn root() -> Arc<Logger> {
    Arc::clone(&manager().root)
}

/// Fetch (creating on first use) the registered logger with the given dotted
/// `name`. The empty name returns the root logger. Mirrors
/// `logging.getLogger(name)`.
pub fn get_logger(name: impl AsRef<str>) -> Arc<Logger> {
    let name = name.as_ref();
    if name.is_empty() {
        return root();
    }
    let mgr = manager();
    let mut map = lock(&mgr.loggers);
    if let Some(existing) = map.get(name) {
        return Arc::clone(existing);
    }
    // New registered loggers inherit their level (NOTSET) and propagate.
    let logger = Logger::new_core(name, 0, true, true, Vec::new());
    map.insert(name.to_string(), Arc::clone(&logger));
    logger
}

/// The names of `name`'s ancestors, nearest first, ending with the root `""`.
fn ancestor_names(name: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = name;
    while let Some(idx) = cur.rfind('.') {
        cur = &cur[..idx];
        names.push(cur.to_string());
    }
    if !name.is_empty() {
        names.push(String::new());
    }
    names
}

/// Dispatch `record` to the sinks of every existing ancestor logger of `name`.
pub(crate) fn propagate(name: &str, record: &Record) {
    let mgr = manager();
    // Collect ancestor handles under the lock, then release it before writing
    // to sinks (which may block on I/O).
    let ancestors: Vec<Arc<Logger>> = {
        let map = lock(&mgr.loggers);
        ancestor_names(name)
            .into_iter()
            .filter_map(|n| map.get(&n).cloned())
            .collect()
    };
    for ancestor in ancestors {
        ancestor.dispatch_only(record);
    }
}

/// The effective severity inherited from `name`'s nearest ancestor that has an
/// explicit level set, or `None` if none do.
pub(crate) fn effective_ancestor_severity(name: &str) -> Option<u16> {
    let mgr = manager();
    let map = lock(&mgr.loggers);
    for n in ancestor_names(name) {
        if let Some(logger) = map.get(&n) {
            let sev = logger.stored_severity();
            if sev != 0 {
                return Some(sev);
            }
        }
    }
    None
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}
