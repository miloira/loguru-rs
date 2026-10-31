# loguru-rs 使用教程

本教程带你从零上手 `loguru-rs`：一个把 Python [loguru](https://github.com/Delgan/loguru)
的易用性移植到 Rust 的日志库，并用 Python `logging` 的"命名多实例"模型解决了
loguru 单一全局 logger 的限制。

> 想看每个 API 的速查表，请看 [README](./README.md)；可运行示例见
> [`examples/demo.rs`](./examples/demo.rs)（`cargo run --example demo`）。

---

## 目录

1. [安装](#1-安装)
2. [最快上手](#2-最快上手)
3. [日志等级](#3-日志等级)
4. [宏 vs 方法](#4-宏-vs-方法)
5. [命名 logger 与层级传播](#5-命名-logger-与层级传播)
6. [独立 logger（多实例）](#6-独立-logger多实例)
7. [Sink：日志去向](#7-sink日志去向)
8. [格式 DSL 与颜色标记](#8-格式-dsl-与颜色标记)
9. [文件轮转、保留与压缩](#9-文件轮转保留与压缩)
10. [过滤器](#10-过滤器)
11. [上下文：bind / contextualize / patch](#11-上下文bind--contextualize--patch)
12. [opt：colors / raw / lazy / backtrace](#12-optcolors--raw--lazy--backtrace)
13. [异常与 panic：exception / catch / 全局钩子](#13-异常与-panicexception--catch--全局钩子)
14. [自定义等级](#14-自定义等级)
15. [启用/禁用与批量配置](#15-启用禁用与批量配置)
16. [异步 sink 与 complete](#16-异步-sink-与-complete)
17. [线程安全](#17-线程安全)
18. [常见配方](#18-常见配方)

---

## 1. 安装

在 `Cargo.toml` 中加入依赖（目前以本地路径为例；发布到 crates.io 后用版本号）：

```toml
[dependencies]
loguru = { path = "../loguru-rs" }
```

需要 gzip 压缩轮转时，开启 `compression` feature：

```toml
loguru = { path = "../loguru-rs", features = ["compression"] }
```

---

## 2. 最快上手

开箱即用——根 logger 默认把带颜色的日志写到 stderr（终端时着色，管道时自动不着色）：

```rust
fn main() {
    loguru::info!("你好, {}", "loguru-rs");
    loguru::success!("任务 #{} 完成", 1);
    loguru::warning!("磁盘已用 {}%", 87);
    loguru::error!("出错了");
}
```

输出：

```
2026-10-31 12:00:00.123 | INFO     | my_crate:main:3 - 你好, loguru-rs
2026-10-31 12:00:00.123 | SUCCESS  | my_crate:main:4 - 任务 #1 完成
```

---

## 3. 日志等级

七个内置等级，从低到高（数值与 loguru 一致）：

| 等级 | 数值 | 宏 / 方法 |
|---|---|---|
| TRACE | 5 | `trace!` / `.trace()` |
| DEBUG | 10 | `debug!` / `.debug()` |
| INFO | 20 | `info!` / `.info()` |
| SUCCESS | 25 | `success!` / `.success()` |
| WARNING | 30 | `warning!` / `.warning()` |
| ERROR | 40 | `error!` / `.error()` |
| CRITICAL | 50 | `critical!` / `.critical()` |

设置等级阈值（低于阈值的记录被直接丢弃）：

```rust
use loguru::Level;
loguru::set_level(Level::Warning); // 根 logger 只输出 WARNING 及以上
```

---

## 4. 宏 vs 方法

两种写法都可以，各有取舍：

- **方法**：`logger.info("...")`，接收任意 `Display`，用 `#[track_caller]`
  自动捕获**文件名和行号**。
- **宏**：`loguru::info!("x = {}", x)`，额外捕获**模块路径和函数名**，并且是**惰性
  格式化**——等级未启用时不会执行格式化。

```rust
let log = loguru::get_logger("app");

// 方法
log.info("简单消息");
log.warning(format!("动态消息 {}", 42));

// 宏（针对指定 logger 用 `to:`，否则作用于全局根 logger）
loguru::info!(to: log, "带格式 {}", 42);
loguru::info!("作用于全局根 logger");
```

---

## 5. 命名 logger 与层级传播

像 `logging.getLogger("a.b")` 一样，用点号名字构成层级：

```rust
use loguru::{get_logger, Level};

let app = get_logger("app");
app.set_level(Level::Debug);

let db = get_logger("app.db");   // 自己没设等级 → 继承 "app" 的 DEBUG
db.debug("打开连接池");           // 可见，因为继承了 DEBUG
```

**传播规则**：`app.db` 产生的记录会**向上**派发到 `app` 和根 logger 的 sink。
是否输出由**产生记录的那个 logger 的有效等级**决定，祖先的等级不会二次过滤
（与 Python `logging` 一致）。关掉传播：

```rust
db.set_propagate(false); // app.db 不再向上派发
```

---

## 6. 独立 logger（多实例）

这是相对 loguru 最关键的增强：你可以创建任意多个**完全独立**、互不共享全局状态的
logger，各有自己的 sink、等级和上下文。

```rust
use loguru::{Logger, Level, FileSink, ConsoleSink};

let audit = Logger::builder()
    .name("audit")
    .level(Level::Info)
    .sink(FileSink::new("audit.log")?)
    .build();

let ui = Logger::builder()
    .name("ui")
    .sink(ConsoleSink::stdout())
    .build();

audit.info("用户登录");
ui.debug("渲染一帧");
```

库作者可以用独立 logger 做内部日志，而不去动应用的全局配置。

---

## 7. Sink：日志去向

一个 sink 就是一个输出目的地（对应 loguru 的 sink / `logging` 的 handler）。
每个 sink 都有独立的**等级、格式、过滤器**。

| Sink | 用途 |
|---|---|
| `ConsoleSink::stderr()` / `stdout()` | 控制台，默认着色（遵守 `NO_COLOR`、仅终端着色） |
| `FileSink::new(path)` | 写文件，支持轮转/保留/压缩 |
| `FnSink::new(\|line\| ...)` | 回调收到格式化后的字符串 |
| `FnSink::from_record(\|rec\| ...)` | 回调收到原始 `Record` |
| `WriterSink::new(writer)` | 写入任意 `std::io::Write`（内存、socket、管道…） |
| `AsyncSink::new(inner)` | 后台线程处理（对应 `enqueue=True`） |

增删 sink（`add` 返回一个 id，用于 `remove`）：

```rust
use loguru::{ConsoleSink, FileSink, Level};

loguru::remove_all(); // 清掉默认 stderr sink（常见的第一步）
let id = loguru::add(ConsoleSink::stderr().with_level(Level::Info));
loguru::add(FileSink::new("app.log")?);
loguru::remove(id);   // 按 id 移除
```

在独立 logger 上同理，用 `logger.add(...) / logger.remove(id)`。

---

## 8. 格式 DSL 与颜色标记

格式字符串由**颜色标记**和**字段占位符**混合组成（loguru 风格）。

```rust
use loguru::ConsoleSink;

let sink = ConsoleSink::stderr().with_format_str(
    "<green>{time:YYYY-MM-DD HH:mm:ss.SSS}</green> | <level>{level: <8}</level> \
     | {name}:{function}:{line} - <level>{message}</level> {extra}",
    true, // 是否着色
);
```

**字段占位符**：

| 占位符 | 含义 |
|---|---|
| `{time:...}` | 时间，token 风格：`YYYY MM DD HH mm ss SSS A ZZ dddd` 等 |
| `{level}` / `{level.no}` / `{level.icon}` | 等级名 / 数值 / 图标 |
| `{message}` | 消息 |
| `{name}` | logger 名字（根为 `root`） |
| `{function}` / `{module}` | 函数名 / 模块路径（宏才有函数名） |
| `{file}` / `{line}` | 文件名 / 行号 |
| `{process}` / `{thread}` | 进程 id / 线程 id |
| `{elapsed}` | 启动至今耗时 |
| `{exception}` | 回溯/错误链文本 |
| `{extra}` / `{extra[key]}` | 全部上下文字段 / 指定字段 |

支持 Python 风格对齐，例如 `{level: <8}`（左对齐、宽度 8）。

**颜色标记**：`<red> <green> <blue> <bold> <dim> <underline>`，亮色用 `<RED>` 或
`<light-red>`，真彩 `<fg #ff8800>`、背景 `<bg blue>`、`<level>`（取当前等级的颜色），
闭合用 `</>` 或 `</tag>`。着色关闭时标记会被自动剥离。

---

## 9. 文件轮转、保留与压缩

```rust
use loguru::{FileSink, Rotation, Retention, Compression};
use std::time::Duration;

let sink = FileSink::new("app.log")?
    // 轮转：按大小 / 按时间间隔 / 每天定点
    .with_rotation(Rotation::Size(10 * 1024 * 1024))        // 10 MiB
    // .with_rotation(Rotation::Interval(Duration::from_secs(3600)))
    // .with_rotation(Rotation::DailyAt { hour: 0, minute: 0 })
    // 保留：全部 / 最多 N 个 / 超过时长删除
    .with_retention(Retention::Count(5))
    // .with_retention(Retention::Age(Duration::from_secs(7 * 24 * 3600)))
    // 压缩（需 `compression` feature，否则相当于不压缩）
    .with_compression(Compression::Gzip);
```

其他文件选项：

```rust
FileSink::new("app.log")?
    .with_truncate(true)      // mode="w"：打开时清空
    .with_delay(true)         // delay：首次写入时才创建文件
    .with_watch(true)         // watch：被外部移走/删除后自动重开
    .with_buffer_size(1);     // buffering：1 相当于行缓冲
```

---

## 10. 过滤器

每个 sink 可以带一个谓词，只输出返回 `true` 的记录：

```rust
use loguru::{FileSink, Level, filter};

let sink = FileSink::new("errors.log")?
    .with_filter(filter(|rec| rec.level.no >= Level::Error.severity()));
```

也可以按名字、按 extra 字段等任意条件过滤。

---

## 11. 上下文：bind / contextualize / patch

**bind**：返回一个带固定上下文字段的新 logger handle（共享同一套 sink）：

```rust
let log = loguru::get_logger("app");
let req = log.bind([("request_id", "r-42")]);
req.info("处理请求"); // 记录里带 request_id=r-42
```

**contextualize**：把字段压入线程局部栈，作用域内所有记录都带上，离开作用域自动弹出：

```rust
{
    let _ctx = loguru::contextualize([("user", "alice")]);
    log.info("在上下文内");   // 带 user=alice
} // 离开后 user 消失
```

**patch**：返回一个会在派发前改写每条记录的新 handle：

```rust
let tagged = log.patch(|rec| {
    rec.extra.insert("service".into(), "api".into());
});
tagged.info("带 service=api");
```

> 要在输出里看到这些字段，记得在格式串里包含 `{extra}` 或 `{extra[key]}`。

---

## 12. opt：colors / raw / lazy / backtrace

对应 loguru 的 `logger.opt(...)`：

```rust
use loguru::Level;
let log = loguru::get_logger("app");

// colors：解析消息内部的颜色标记
log.colors(true).info("状态 <green>OK</green>，延迟 <yellow>偏高</yellow>");

// raw：绕过 formatter，只输出消息本身（无时间/等级）
log.raw(true).info("一行原始输出");

// lazy：等级未启用时不构造消息
log.log_lazy(Level::Debug, || format!("昂贵计算: {}", expensive()));

// backtrace：给每条记录附加回溯
log.with_backtrace(true).error("附带回溯");
```

---

## 13. 异常与 panic：exception / catch / 全局钩子

```rust
let log = loguru::get_logger("app");

// 以 ERROR 记录并附回溯
log.exception("出错了");

// 附带 Error 及其 source() 链
let err = std::io::Error::new(std::io::ErrorKind::NotFound, "config.toml 不存在");
log.exception_with(&err, "加载配置失败");

// 捕获闭包里的 panic，记录后继续（返回 Option）
let r = log.catch(|| might_panic());

// 带选项的 catch：等级、重抛、消息、默认值、错误回调
let v = log.catch_opts()
    .level(Level::Critical)
    .message("任务失败")
    .on_error(|msg| eprintln!("额外处理: {msg}"))
    .run_or(0, || compute()); // panic 时返回默认值 0

// 全局 panic 钩子：所有未捕获 panic 以 CRITICAL 记录
loguru::install_panic_hook();
```

> 说明：Rust 默认 panic hook 仍会在我们记录之前先打印它自己的一行，这是运行时行为。

---

## 14. 自定义等级

注册带编号、颜色、图标的自定义等级，然后按名字记录：

```rust
use loguru::LevelInfo;

loguru::add_level(LevelInfo::new("NOTICE", 23, "<magenta><bold>", "📢"));
loguru::logger().log_by_name("NOTICE", "一条 NOTICE 级别的消息");
```

---

## 15. 启用/禁用与批量配置

**启用/禁用**（按 logger 名前缀，最具体的规则优先）：

```rust
loguru::disable("app.db");  // 抑制 app.db 及其子 logger
loguru::enable("app.db");
```

**批量配置**根 logger（对应 `logger.configure(...)`）：

```rust
use loguru::{ConsoleSink, FileSink, Level};
use std::sync::Arc;

loguru::configure(
    true,                 // 先移除现有 sink
    Some(Level::Info),    // 设置根等级
    vec![
        Arc::new(ConsoleSink::stderr()),
        Arc::new(FileSink::new("app.log")?),
    ],
);
```

---

## 16. 异步 sink 与 complete

把慢 I/O 放到后台线程，避免阻塞日志调用（对应 `enqueue=True`）：

```rust
use loguru::{AsyncSink, FileSink, formatter, json_format};

let log = loguru::get_logger("events");
log.add(AsyncSink::new(
    FileSink::new("events.jsonl")?.with_format(formatter(json_format)),
));

log.info("写到后台线程");
log.complete(); // 等待队列排空（程序退出前调用）
```

`AsyncSink` 在被 drop 时也会自动排空并 join 后台线程。

---

## 17. 线程安全

`Logger`（即 `Arc<Logger>`）和所有 sink 都是 `Send + Sync`。handle 克隆很廉价，
可自由跨线程共享：

```rust
use std::thread;
let log = loguru::get_logger("worker");
let handles: Vec<_> = (0..4).map(|i| {
    let log = log.clone();
    thread::spawn(move || log.info(format!("worker {i} 启动")))
}).collect();
for h in handles { h.join().unwrap(); }
```

---

## 18. 常见配方

**控制台 + 文件，不同等级：**

```rust
use loguru::{ConsoleSink, FileSink, Level, Rotation};

loguru::remove_all();
loguru::add(ConsoleSink::stderr().with_level(Level::Info));
loguru::add(
    FileSink::new("logs/app.log")?
        .with_level(Level::Debug)
        .with_rotation(Rotation::Size(20 * 1024 * 1024)),
);
```

**结构化 JSON 日志：**

```rust
use loguru::{FileSink, formatter, json_format};
loguru::add(FileSink::new("app.jsonl")?.with_format(formatter(json_format)));
```

**把日志收集进内存 buffer（测试时常用）：**

```rust
use loguru::{WriterSink, Logger};
use std::sync::{Arc, Mutex};
use std::io::Write;

let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
struct Shared(Arc<Mutex<Vec<u8>>>);
impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().write(b) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
let log = Logger::builder().name("test").sink(WriterSink::new(Shared(buf.clone()))).build();
log.info("captured");
log.flush();
assert!(String::from_utf8_lossy(&buf.lock().unwrap()).contains("captured"));
```

---

更完整的功能对照与"Rust 下不适用的特性"说明，见 [README](./README.md)。
