# loguru-rs

An ergonomic logging library for Rust: a faithful port of Python's
[loguru](https://github.com/Delgan/loguru), extended with the multi-instance,
named-logger model of Python's standard `logging` module.

## Why

Python's loguru is a joy to use, but it exposes a **single global `logger`**.
That is awkward when a library wants to configure logging without stepping on
the application, or when one program needs to route subsystems to different
destinations.

`loguru-rs` keeps loguru's full feature set and fixes the single-instance
limitation the way `logging` does: a **registry of named loggers** in a dotted
hierarchy (with propagation), plus fully **independent standalone loggers**.

> New here? Read the step-by-step [使用教程 / Tutorial](./TUTORIAL.md).

## Feature parity with loguru

| loguru | loguru-rs |
| --- | --- |
| `logger.info("...")` | `logger.info("...")` / `info!(...)` macro |
| `TRACE … SUCCESS … CRITICAL` | `Level` enum, same numbers |
| custom levels (`logger.level`) | `add_level` / `Logger::level`, with color + icon |
| format DSL `{time:...}`, `{level}`, `<green>…</>` | full template + color-markup engine |
| sinks (`add`/`remove`) | `add`/`remove`, returns an id |
| per-sink `level`, `format`, `filter` | `with_level`/`with_format_str`/`with_filter` |
| `serialize=True` | `json_format` |
| file `rotation` / `retention` | `Rotation` (size/interval/daily) + `Retention` |
| `enqueue=True` | `AsyncSink` |
| callable sinks | `FnSink` |
| `logger.bind(...)` | `Logger::bind` |
| `logger.contextualize(...)` | `contextualize` (scoped guard) |
| `logger.patch(...)` | `Logger::patch` |
| `logger.opt(colors=)` | `.colors(true)` |
| `logger.opt(raw=True)` | `.raw(true)` |
| `logger.opt(lazy=True)` | `Logger::log_lazy` |
| `logger.opt(exception=True)` | `.with_backtrace(true)` |
| `logger.catch` (+ reraise/level/message) | `Logger::catch` / `catch_opts` |
| `logger.exception(...)` | `Logger::exception` / `exception_with` (Error chain) |
| `logger.enable/disable` | `enable` / `disable` |
| `logger.complete()` | `Logger::complete` |
| `logger.configure(...)` | `configure` |
| auto color only on a TTY | `std::io::IsTerminal` detection |
| file `compression` | `Compression::Gzip` (feature `compression`) |
| file `mode`/`delay` | `with_truncate` / `with_delay` |
| file `watch` / `buffering` | `with_watch` / `with_buffer_size` |
| sink = any file-like object | `WriterSink<W: Write>` |
| `catch(default=, onerror=)` | `catch_opts().run_or(..)` / `.on_error(..)` |
| global exception hook | `install_panic_hook` |

### What does not map to Rust

The only loguru feature with no Rust equivalent is `diagnose` (printing local
variable *values* inside tracebacks), which needs runtime local-variable
reflection Rust lacks. A few options are not meaningful in Rust rather than
missing:

- `opt(depth=)` / `opt(record=)` / `opt(capture=)` — call-site capture comes
  from `#[track_caller]`/macros, and per-call fields come from `bind`.
- `catch` filtering by exception *type* / `exclude` — Rust panic payloads are
  untyped (`Box<dyn Any>`), so there are no exception classes to match.
- `add(encoding=/errors=/newline=/context=)` — Rust I/O is UTF-8 and the
  multiprocessing `context` is Python-specific.

Behavioral note: Rust's default panic hook still prints its own line before
`catch` / `install_panic_hook` logs the panic.

## Quick start

```rust
loguru::info!("hello from {}", "loguru-rs");
loguru::success!("done in {} ms", 42);
loguru::warning!("disk {}% full", 87);
```

```
2026-10-31 12:00:00.123 | INFO     | my_crate:main:12 - hello from loguru-rs
```

## Named loggers (like `logging.getLogger`)

```rust
use loguru::{get_logger, Level};

let app = get_logger("app");
app.set_level(Level::Debug);

let db = get_logger("app.db");         // inherits DEBUG from `app`
db.debug("connection pool opened");
loguru::info!(to: db, "query took {} ms", 12);
```

Records from `app.db` propagate to the sinks of `app` and the root. The
originating logger's effective level gates emission; ancestors' levels do not
re-filter. Turn it off with `db.set_propagate(false)`.

## Standalone loggers

```rust
use loguru::{Logger, Level, FileSink, ConsoleSink};

let audit = Logger::builder()
    .name("audit").level(Level::Info)
    .sink(FileSink::new("audit.log")?)
    .build();

let ui = Logger::builder().name("ui").sink(ConsoleSink::stdout()).build();
```

## The format DSL

Format strings mix **color markup** and **field placeholders**:

```rust
use loguru::ConsoleSink;
let sink = ConsoleSink::stderr().with_format_str(
    "<green>{time:YYYY-MM-DD HH:mm:ss.SSS}</green> | <level>{level: <8}</level> \
     | {name}:{function}:{line} - <level>{message}</level> {extra}",
    true, // colorize
);
```

Fields: `{time:…}` (pendulum-style tokens: `YYYY MM DD HH mm ss SSS`),
`{level}`, `{level.no}`, `{level.icon}`, `{message}`, `{name}`, `{function}`,
`{module}`, `{file}`, `{line}`, `{process}`, `{thread}`, `{elapsed}`,
`{exception}`, `{extra}`, `{extra[key]}`. Python-style alignment is supported,
e.g. `{level: <8}`.

Markup: `<red> <green> <blue> … <bold> <dim> <underline>`, bright via
`<RED>`/`<light-red>`, `<fg #ff8800>`, `<bg blue>`, `<level>` (the record's
level color), and closing `</>` / `</tag>`.

## Custom levels

```rust
use loguru::LevelInfo;
loguru::add_level(LevelInfo::new("NOTICE", 23, "<magenta><bold>", "📢"));
loguru::logger().log_by_name("NOTICE", "something noteworthy");
```

## Sinks

- `ConsoleSink::stderr()` / `stdout()` — colored by default, honors `NO_COLOR`.
- `FileSink::new(path)` — append, with `Rotation::{Size, Interval, DailyAt}`
  and `Retention::{All, Count, Age}`.
- `FnSink::new(|line| ...)` / `FnSink::from_record(|rec| ...)` — callable sinks.
- `WriterSink::new(writer)` — write to any `std::io::Write` (buffer, socket, …).
- `AsyncSink::new(inner)` — process records on a background thread (`enqueue`).

`FileSink` also supports `with_watch(true)` (reopen on external rotate/delete),
`with_buffer_size(n)` (buffering), `with_truncate(true)` (mode="w"), and
`with_delay(true)` (open on first write).

All sinks support `.with_level(...)`, `.with_format_str(...)`/`.with_format(...)`,
and `.with_filter(...)`.

```rust
use loguru::{FileSink, Rotation, Retention, Level, filter};

loguru::remove_all();
loguru::add(
    FileSink::new("app.log")?
        .with_rotation(Rotation::Size(10 * 1024 * 1024))
        .with_retention(Retention::Count(5))
        .with_filter(filter(|r| r.level.no >= Level::Warning.severity())),
);
```

## Context, options, patching

```rust
let log = loguru::get_logger("app");

let req = log.bind([("request_id", "r-42")]);        // persistent context
req.info("handling request");

{
    let _ctx = loguru::contextualize([("user", "alice")]); // scoped
    req.warning("slow");
}

log.colors(true).info("status <green>OK</green>");    // opt(colors=True)
log.raw(true).info("raw line, no timestamp/level");   // opt(raw=True)
log.log_lazy(Level::Debug, || expensive());           // opt(lazy=True)
log.patch(|r| { r.extra.insert("svc".into(), "api".into()); }).info("tagged");
log.exception("failed");                               // ERROR + backtrace
log.exception_with(&err, "failed to load config");     // + Error source chain
let _ = log.catch(|| risky());                         // log panics, keep going
let _ = log.catch_opts().level(Level::Critical).reraise(true).run(|| risky());

loguru::install_panic_hook(); // log every uncaught panic via the root logger
```

Enable gzip compression of rotated files with the `compression` feature:

```toml
loguru = { path = ".", features = ["compression"] }
```

```rust
use loguru::{FileSink, Rotation, Compression};
let sink = FileSink::new("app.log")?
    .with_rotation(Rotation::Size(10 * 1024 * 1024))
    .with_compression(Compression::Gzip);
```

## Enable / disable

```rust
loguru::disable("app.db");   // suppress records from app.db and children
loguru::enable("app.db");
```

## Macros vs. methods

- **Methods** (`logger.info("...")`) take any `Display` and capture file+line
  via `#[track_caller]`.
- **Macros** (`loguru::info!("x = {}", x)`) additionally capture module path and
  function, and format lazily. Use `info!(to: logger, ...)` for a specific
  logger, or `info!(...)` for the global root.

## Example

```
cargo run --example demo
```

## License

MIT — see [LICENSE](LICENSE).
