//! Logging macros.
//!
//! Unlike the `#[track_caller]` methods on [`Logger`](crate::Logger), the
//! macros also capture the **module path** of the call site and format their
//! arguments with `format_args!`, so nothing is allocated when the record is
//! filtered out by level.
//!
//! Each macro has two forms:
//!
//! ```
//! use loguru::{info, get_logger};
//! let log = get_logger("app");
//!
//! info!("using the global root logger: {}", 1 + 1);
//! info!(to: log, "using an explicit logger: {}", 2 + 2);
//! ```

/// Log at an explicit [`Level`](crate::Level) to a specific logger.
///
/// `log!(target, level, "fmt", args...)` where `target` is anything
/// implementing [`AsLogger`](crate::AsLogger) (a `Logger`, `&Logger`, or
/// `Arc<Logger>`).
#[macro_export]
macro_rules! log {
    ($target:expr, $level:expr, $($arg:tt)+) => {{
        // Bind behind a reference so a temporary target (e.g. `logger()`)
        // lives for the whole block, without moving a caller's variable.
        let __loguru_target = &$target;
        let __loguru_logger = $crate::AsLogger::as_logger(__loguru_target);
        let __loguru_level = $level;
        if __loguru_logger.is_enabled(__loguru_level) {
            __loguru_logger.__log_fmt(
                __loguru_level,
                ::core::module_path!(),
                $crate::__function!(),
                ::core::file!(),
                ::core::line!(),
                ::core::format_args!($($arg)+),
            );
        }
    }};
}

/// Best-effort current function name, via the `type_name` trick. Returns a
/// `&'static str`. Hidden implementation detail of the logging macros.
#[doc(hidden)]
#[macro_export]
macro_rules! __function {
    () => {{
        fn __loguru_f() {}
        fn __loguru_type_name_of<T>(_: T) -> &'static str {
            ::core::any::type_name::<T>()
        }
        let __name = __loguru_type_name_of(__loguru_f);
        let __trimmed = match __name.strip_suffix("::__loguru_f") {
            ::core::option::Option::Some(s) => s,
            ::core::option::Option::None => __name,
        };
        match __trimmed.rfind("::") {
            ::core::option::Option::Some(__i) => &__trimmed[__i + 2..],
            ::core::option::Option::None => __trimmed,
        }
    }};
}

/// Log at [`Level::Trace`](crate::Level::Trace). Use `trace!(to: logger, ...)`
/// to target a specific logger, or `trace!(...)` for the global root logger.
#[macro_export]
macro_rules! trace {
    (to: $target:expr, $($arg:tt)+) => {
        $crate::log!($target, $crate::Level::Trace, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::logger(), $crate::Level::Trace, $($arg)+)
    };
}

/// Log at [`Level::Debug`](crate::Level::Debug). Use `debug!(to: logger, ...)`
/// to target a specific logger, or `debug!(...)` for the global root logger.
#[macro_export]
macro_rules! debug {
    (to: $target:expr, $($arg:tt)+) => {
        $crate::log!($target, $crate::Level::Debug, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::logger(), $crate::Level::Debug, $($arg)+)
    };
}

/// Log at [`Level::Info`](crate::Level::Info). Use `info!(to: logger, ...)`
/// to target a specific logger, or `info!(...)` for the global root logger.
#[macro_export]
macro_rules! info {
    (to: $target:expr, $($arg:tt)+) => {
        $crate::log!($target, $crate::Level::Info, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::logger(), $crate::Level::Info, $($arg)+)
    };
}

/// Log at [`Level::Success`](crate::Level::Success). Use
/// `success!(to: logger, ...)` to target a specific logger, or `success!(...)`
/// for the global root logger.
#[macro_export]
macro_rules! success {
    (to: $target:expr, $($arg:tt)+) => {
        $crate::log!($target, $crate::Level::Success, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::logger(), $crate::Level::Success, $($arg)+)
    };
}

/// Log at [`Level::Warning`](crate::Level::Warning). Use
/// `warning!(to: logger, ...)` to target a specific logger, or `warning!(...)`
/// for the global root logger.
#[macro_export]
macro_rules! warning {
    (to: $target:expr, $($arg:tt)+) => {
        $crate::log!($target, $crate::Level::Warning, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::logger(), $crate::Level::Warning, $($arg)+)
    };
}

/// Log at [`Level::Error`](crate::Level::Error). Use `error!(to: logger, ...)`
/// to target a specific logger, or `error!(...)` for the global root logger.
#[macro_export]
macro_rules! error {
    (to: $target:expr, $($arg:tt)+) => {
        $crate::log!($target, $crate::Level::Error, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::logger(), $crate::Level::Error, $($arg)+)
    };
}

/// Log at [`Level::Critical`](crate::Level::Critical). Use
/// `critical!(to: logger, ...)` to target a specific logger, or
/// `critical!(...)` for the global root logger.
#[macro_export]
macro_rules! critical {
    (to: $target:expr, $($arg:tt)+) => {
        $crate::log!($target, $crate::Level::Critical, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::logger(), $crate::Level::Critical, $($arg)+)
    };
}
