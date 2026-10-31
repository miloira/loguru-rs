//! A tour of loguru-rs. Run with: `cargo run --example demo`

use std::sync::Arc;

use loguru::{
    filter, formatter, get_logger, json_format, AsyncSink, ConsoleSink, FileSink, Level, LevelInfo,
    Logger, Retention, Rotation,
};

fn main() -> std::io::Result<()> {
    // 1. Global root logger, works out of the box.
    loguru::info!("global root logger, no setup required");
    loguru::success!("operation #{} finished", 1);

    // Reconfigure the root logger with a custom format string (the loguru DSL).
    loguru::remove_all();
    loguru::add(
        ConsoleSink::stderr()
            .with_level(Level::Trace)
            .with_format_str(
                "{time:HH:mm:ss.SSS} | <level>{level: <8}</level> | {name}:{function}:{line} - <level>{message}</level> {extra}",
                true,
            ),
    );
    loguru::set_level(Level::Trace);
    loguru::trace!("now TRACE is visible, with a custom format");

    // 2. Custom level with its own number, color, and icon.
    loguru::add_level(LevelInfo::new("NOTICE", 23, "<magenta><bold>", "📢"));
    loguru::logger().log_by_name("NOTICE", "a custom NOTICE-level message");

    // 3. Named registry loggers with hierarchy + propagation.
    let app = get_logger("app");
    app.set_level(Level::Debug);
    let db = get_logger("app.db");
    db.debug("opened connection pool (level inherited from `app`)");
    loguru::info!(to: db, "query took {} ms", 12);

    // bind (persistent context) and contextualize (scoped context).
    let request = app.bind([("request_id", "r-42")]);
    request.info("handling request");
    {
        let _ctx = loguru::contextualize([("user", "alice")]);
        request.warning("slow handler");
    }

    // opt(colors): markup inside the message itself.
    app.colors(true)
        .info("status: <green>OK</green>, latency <yellow>high</yellow>");

    // patch: mutate every record from this handle.
    let tagged = app.patch(|rec| {
        rec.extra.insert("service".into(), "api".into());
    });
    tagged.info("patched record carries service=api");

    // exception + exception_with(Error chain) + catch + catch_opts.
    app.exception("something went wrong (with backtrace)");
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "config.toml missing");
    app.exception_with(&io_err, "failed to load config");
    let caught: Option<i32> = app.catch(|| panic!("boom in a closure"));
    assert!(caught.is_none());
    let ok = app
        .catch_opts()
        .level(Level::Warning)
        .message("task failed")
        .run(|| 7 * 6);
    assert_eq!(ok, Some(42));

    // raw (bypass formatter) and lazy evaluation.
    app.raw(true).info("--- a raw line, no timestamp/level ---");
    app.log_lazy(Level::Debug, || format!("expensive: {}", 2 + 2));

    // enable/disable activation.
    loguru::disable("app.db");
    db.error("this is suppressed because app.db is disabled");
    loguru::enable("app.db");

    // 4. Standalone loggers with filters, rotation, JSON, and async.
    let audit: Arc<Logger> = Logger::builder()
        .name("audit")
        .level(Level::Info)
        .sink(
            FileSink::new("target/audit.log")?
                .with_rotation(Rotation::Size(1024 * 1024))
                .with_retention(Retention::Count(3))
                .with_filter(filter(|rec| rec.level.no >= Level::Warning.severity())),
        )
        .build();
    audit.info("info is below the sink filter, so the file skips it");
    audit.warning("this warning is written to target/audit.log");

    let events = Logger::builder()
        .name("events")
        .sink(AsyncSink::new(
            FileSink::new("target/events.jsonl")?.with_format(formatter(json_format)),
        ))
        .build();
    events.info("structured event written as JSON on a background thread");
    events.complete(); // wait for the async sink to drain

    println!("\nWrote target/audit.log and target/events.jsonl");
    Ok(())
}
