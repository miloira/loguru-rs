//! Process-wide logging context: the `contextualize` stack, module activation
//! (`enable`/`disable`), and the elapsed-time clock.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

thread_local! {
    /// A stack of context maps pushed by `contextualize`. All frames are merged
    /// into each record's `extra`, nearest frame winning.
    static CONTEXT: RefCell<Vec<BTreeMap<String, String>>> = const { RefCell::new(Vec::new()) };
}

/// Snapshot the merged thread-local context into `target` (older frames first,
/// so newer frames override).
pub(crate) fn merge_context(target: &mut BTreeMap<String, String>) {
    CONTEXT.with(|c| {
        for frame in c.borrow().iter() {
            for (k, v) in frame {
                target.insert(k.clone(), v.clone());
            }
        }
    });
}

/// Push a context frame; the returned guard pops it on drop. Mirrors loguru's
/// `logger.contextualize(**kwargs)`.
#[must_use = "the context is only active while the guard is alive"]
pub struct ContextGuard {
    _private: (),
}

impl ContextGuard {
    pub(crate) fn push(fields: BTreeMap<String, String>) -> ContextGuard {
        CONTEXT.with(|c| c.borrow_mut().push(fields));
        ContextGuard { _private: () }
    }
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        CONTEXT.with(|c| {
            c.borrow_mut().pop();
        });
    }
}

// --- Module activation (enable / disable) -------------------------------

fn activation() -> &'static Mutex<Vec<(String, bool)>> {
    static ACTIVATION: OnceLock<Mutex<Vec<(String, bool)>>> = OnceLock::new();
    ACTIVATION.get_or_init(|| Mutex::new(Vec::new()))
}

/// Enable logging for records whose logger name starts with `name` (undoing a
/// previous [`disable`]). An empty name affects all loggers.
pub fn enable(name: &str) {
    set_activation(name, true);
}

/// Disable logging for records whose logger name starts with `name`. Mirrors
/// loguru's `logger.disable(name)`.
pub fn disable(name: &str) {
    set_activation(name, false);
}

fn set_activation(name: &str, state: bool) {
    let mut rules = lock(activation());
    if let Some(entry) = rules.iter_mut().find(|(n, _)| n == name) {
        entry.1 = state;
    } else {
        rules.push((name.to_string(), state));
    }
    // Longest prefixes first, so the most specific rule wins.
    rules.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
}

/// Whether a logger with the given `name` is currently enabled. The most
/// specific matching prefix rule wins; with no rule, logging is enabled.
pub(crate) fn is_enabled(name: &str) -> bool {
    let rules = lock(activation());
    for (prefix, state) in rules.iter() {
        if prefix.is_empty() || name == prefix || name.starts_with(&format!("{prefix}.")) {
            return *state;
        }
    }
    true
}

// --- Elapsed clock ------------------------------------------------------

/// The instant the logging system first started, used for `{elapsed}`.
pub(crate) fn start_instant() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
