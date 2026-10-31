//! Formatting: the loguru-style format-string template engine, color markup,
//! and built-in formatters.
//!
//! A format string mixes literal text, **color markup** tags like `<green>`,
//! `<level>`, `</>`, and **field placeholders** like `{time:YYYY-MM-DD}`,
//! `{level: <8}`, `{name}`, `{message}`, `{extra[key]}`. See [`DEFAULT_FORMAT`]
//! for the default layout and [`from_template`] to compile your own.

use std::fmt::Write as _;
use std::sync::Arc;

use crate::record::Record;

/// A formatter renders a [`Record`] into a string (no trailing newline).
pub type Formatter = Arc<dyn Fn(&Record) -> String + Send + Sync>;

const RESET: &str = "\x1b[0m";

/// The default loguru-style format string.
pub const DEFAULT_FORMAT: &str = "<green>{time:YYYY-MM-DD HH:mm:ss.SSS}</green> | <level>{level: <8}</level> | <cyan>{name}</cyan>:<cyan>{function}</cyan>:<cyan>{line}</cyan> - <level>{message}</level>";

/// Build a [`Formatter`] from a closure.
pub fn formatter<F>(f: F) -> Formatter
where
    F: Fn(&Record) -> String + Send + Sync + 'static,
{
    Arc::new(f)
}

/// Compile a loguru-style format string into a [`Formatter`].
///
/// `colorize` controls whether color markup produces ANSI escapes (`true`) or
/// is stripped (`false`).
///
/// ```
/// use loguru::format::from_template;
/// let fmt = from_template("{time:HH:mm:ss} <level>{level}</level> {message}", true);
/// ```
pub fn from_template(template: &str, colorize: bool) -> Formatter {
    let compiled = Template::compile(template);
    Arc::new(move |record: &Record| compiled.render(record, colorize))
}

/// The default plain-text formatter (markup stripped).
pub fn default_format(record: &Record) -> String {
    default_template().render(record, false)
}

/// The default colorized formatter (markup -> ANSI).
pub fn default_format_colored(record: &Record) -> String {
    default_template().render(record, true)
}

fn default_template() -> &'static Template {
    use std::sync::OnceLock;
    static T: OnceLock<Template> = OnceLock::new();
    T.get_or_init(|| Template::compile(DEFAULT_FORMAT))
}

/// Returns `true` if ANSI colors are allowed at all (honors the
/// [`NO_COLOR`](https://no-color.org) convention). Note this does not check
/// whether the destination is a terminal; see [`ConsoleSink`] for that.
///
/// [`ConsoleSink`]: crate::ConsoleSink
pub fn should_colorize() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// Render a record in "raw" mode: just the message (with markup resolved if the
/// record carries colored markup), bypassing the normal field layout. Used when
/// a record was logged via `opt().raw(true)`.
pub fn raw_line(record: &Record, colorize_on: bool) -> String {
    if record.colored_message {
        colorize(&record.message, &record.level.color, colorize_on)
    } else {
        record.message.clone()
    }
}

// ===================================================================
// Color markup
// ===================================================================

/// Translate a single opening markup tag to an ANSI SGR sequence, or `None` if
/// the tag is not recognized (in which case it is treated as literal text).
fn tag_to_ansi(tag: &str) -> Option<String> {
    let t = tag.trim();
    let lower = t.to_ascii_lowercase();

    // Styles.
    let style = match lower.as_str() {
        "bold" | "b" => Some("1"),
        "dim" | "d" => Some("2"),
        "italic" | "i" => Some("3"),
        "underline" | "u" => Some("4"),
        "blink" => Some("5"),
        "reverse" | "v" => Some("7"),
        "hide" | "h" => Some("8"),
        "strike" | "s" => Some("9"),
        "normal" | "n" => Some("22"),
        _ => None,
    };
    if let Some(code) = style {
        return Some(format!("\x1b[{code}m"));
    }

    // Explicit foreground/background: "fg X" / "bg X".
    if let Some(rest) = lower.strip_prefix("fg ") {
        return color_code(rest.trim(), false, t.strip_prefix("fg ").unwrap_or(rest).trim());
    }
    if let Some(rest) = lower.strip_prefix("bg ") {
        return color_code(rest.trim(), true, t.strip_prefix("bg ").unwrap_or(rest).trim());
    }

    // Bare color name (foreground). Uppercase name = bright variant.
    color_code(&lower, false, t)
}

/// Build an ANSI color code for a color spec (name, `#rrggbb`, or 0-255).
///
/// `orig` is the original (case-preserving) spec, used to detect uppercase =
/// bright for named colors.
fn color_code(spec: &str, background: bool, orig: &str) -> Option<String> {
    // Hex: #rrggbb
    if let Some(hex) = spec.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(rgb) = u32::from_str_radix(hex, 16) {
                let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
                let lead = if background { 48 } else { 38 };
                return Some(format!("\x1b[{lead};2;{r};{g};{b}m"));
            }
        }
        return None;
    }
    // 8-bit palette index.
    if let Ok(n) = spec.parse::<u8>() {
        let lead = if background { 48 } else { 38 };
        return Some(format!("\x1b[{lead};5;{n}m"));
    }

    let (name, bright) = match spec.strip_prefix("light-") {
        Some(rest) => (rest, true),
        None => {
            // An all-uppercase named color means the bright variant.
            let is_upper = orig.chars().any(|c| c.is_ascii_uppercase())
                && !orig.chars().any(|c| c.is_ascii_lowercase());
            (spec, is_upper)
        }
    };

    let base = match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        _ => return None,
    };
    let mut code = base + if background { 40 } else { 30 };
    if bright {
        code += 60;
    }
    Some(format!("\x1b[{code}m"))
}

/// Render markup `input` into `out`, maintaining a shared ANSI code stack so
/// that nesting across literal/field boundaries works. `level_color` is the
/// current record level's markup, used to expand `<level>`.
///
/// When `colorize` is false, tags are stripped and only text is emitted.
fn render_markup(out: &mut String, stack: &mut Vec<String>, input: &str, level_color: &str, colorize: bool) {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && i + 1 < bytes.len() && (bytes[i + 1] == b'<' || bytes[i + 1] == b'>') {
            // Escaped angle bracket.
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if c == b'<' {
            if let Some(end) = input[i..].find('>') {
                let tag = &input[i + 1..i + end];
                i += end + 1;
                apply_tag(out, stack, tag, level_color, colorize);
                continue;
            }
        }
        // Copy a run of plain text until the next special char.
        let start = i;
        while i < bytes.len() && bytes[i] != b'<' && bytes[i] != b'\\' {
            i += 1;
        }
        out.push_str(&input[start..i]);
        if i == start {
            // No progress (lone '\\' or '<'); emit the char literally.
            out.push(bytes[i] as char);
            i += 1;
        }
    }
}

fn apply_tag(out: &mut String, stack: &mut Vec<String>, tag: &str, level_color: &str, colorize: bool) {
    if let Some(inner) = tag.strip_prefix('/') {
        // Closing tag: </>, </tag>. Pop one entry and reapply the rest.
        let _ = inner; // the specific tag name is not required to match
        if !stack.is_empty() {
            stack.pop();
        }
        if colorize {
            out.push_str(RESET);
            for code in stack.iter() {
                out.push_str(code);
            }
        }
        return;
    }

    if tag.eq_ignore_ascii_case("level") {
        // Expand the level's own markup into a single combined code.
        let mut combined = String::new();
        let mut tmp_stack: Vec<String> = Vec::new();
        collect_codes(level_color, &mut combined, &mut tmp_stack);
        stack.push(combined.clone());
        if colorize {
            out.push_str(&combined);
        }
        return;
    }

    match tag_to_ansi(tag) {
        Some(code) => {
            if colorize {
                out.push_str(&code);
            }
            stack.push(code);
        }
        None => {
            // Unknown tag: treat as literal text.
            out.push('<');
            out.push_str(tag);
            out.push('>');
        }
    }
}

/// Collect the concatenated ANSI codes of a (possibly multi-tag) markup string
/// like `"<green><bold>"` into `combined`.
fn collect_codes(markup: &str, combined: &mut String, _stack: &mut Vec<String>) {
    let mut i = 0;
    let bytes = markup.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(end) = markup[i..].find('>') {
                let tag = &markup[i + 1..i + end];
                i += end + 1;
                if !tag.starts_with('/') && !tag.eq_ignore_ascii_case("level") {
                    if let Some(code) = tag_to_ansi(tag) {
                        combined.push_str(&code);
                    }
                }
                continue;
            }
        }
        i += 1;
    }
}

/// Colorize (or strip) a standalone markup string, e.g. a message logged with
/// `opt().colors(true)`.
pub fn colorize(input: &str, level_color: &str, colorize: bool) -> String {
    let mut out = String::with_capacity(input.len());
    let mut stack = Vec::new();
    render_markup(&mut out, &mut stack, input, level_color, colorize);
    if colorize && !stack.is_empty() {
        out.push_str(RESET);
    }
    out
}

// ===================================================================
// Template engine
// ===================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum Align {
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone)]
struct Spec {
    fill: char,
    align: Option<Align>,
    width: usize,
}

impl Spec {
    fn none() -> Self {
        Spec {
            fill: ' ',
            align: None,
            width: 0,
        }
    }

    fn apply(&self, value: &str, numeric: bool) -> String {
        let len = value.chars().count();
        if self.width == 0 || len >= self.width {
            return value.to_string();
        }
        let pad = self.width - len;
        let align = self
            .align
            .unwrap_or(if numeric { Align::Right } else { Align::Left });
        let fill: String = std::iter::repeat(self.fill).take(pad).collect();
        match align {
            Align::Left => format!("{value}{fill}"),
            Align::Right => format!("{fill}{value}"),
            Align::Center => {
                let left = pad / 2;
                let right = pad - left;
                let lf: String = std::iter::repeat(self.fill).take(left).collect();
                let rf: String = std::iter::repeat(self.fill).take(right).collect();
                format!("{lf}{value}{rf}")
            }
        }
    }
}

fn parse_spec(spec: &str) -> Spec {
    let chars: Vec<char> = spec.chars().collect();
    if chars.is_empty() {
        return Spec::none();
    }
    let mut fill = ' ';
    let mut align = None;
    let mut rest_start = 0;
    if chars.len() >= 2 && matches!(chars[1], '<' | '>' | '^') {
        fill = chars[0];
        align = Some(to_align(chars[1]));
        rest_start = 2;
    } else if matches!(chars[0], '<' | '>' | '^') {
        align = Some(to_align(chars[0]));
        rest_start = 1;
    }
    let width: usize = chars[rest_start..]
        .iter()
        .collect::<String>()
        .trim()
        .parse()
        .unwrap_or(0);
    Spec { fill, align, width }
}

fn to_align(c: char) -> Align {
    match c {
        '>' => Align::Right,
        '^' => Align::Center,
        _ => Align::Left,
    }
}

#[derive(Debug, Clone)]
enum FieldKind {
    Time(String),
    Level,
    LevelName,
    LevelNo,
    LevelIcon,
    Message,
    Name,
    Function,
    Module,
    File,
    FileName,
    FilePath,
    Line,
    Process,
    ProcessName,
    Thread,
    ThreadName,
    Elapsed,
    Exception,
    ExtraAll,
    ExtraKey(String),
    Unknown(String),
}

#[derive(Debug, Clone)]
struct Field {
    kind: FieldKind,
    spec: Spec,
}

#[derive(Debug, Clone)]
enum Seg {
    /// Literal text that may contain color markup.
    Markup(String),
    Field(Field),
}

/// A compiled format template.
#[derive(Debug, Clone)]
struct Template {
    segs: Vec<Seg>,
}

impl Template {
    fn compile(src: &str) -> Template {
        let mut segs = Vec::new();
        let mut lit = String::new();
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'{' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    lit.push('{');
                    i += 2;
                    continue;
                }
                if let Some(end) = src[i..].find('}') {
                    let inner = &src[i + 1..i + end];
                    if !lit.is_empty() {
                        segs.push(Seg::Markup(std::mem::take(&mut lit)));
                    }
                    segs.push(Seg::Field(parse_field(inner)));
                    i += end + 1;
                    continue;
                }
            }
            if c == b'}' && i + 1 < bytes.len() && bytes[i + 1] == b'}' {
                lit.push('}');
                i += 2;
                continue;
            }
            lit.push(c as char);
            i += 1;
        }
        if !lit.is_empty() {
            segs.push(Seg::Markup(lit));
        }
        Template { segs }
    }

    fn render(&self, record: &Record, colorize_on: bool) -> String {
        let mut out = String::with_capacity(128 + record.message.len());
        let mut stack: Vec<String> = Vec::new();
        let level_color = &record.level.color;
        for seg in &self.segs {
            match seg {
                Seg::Markup(text) => {
                    render_markup(&mut out, &mut stack, text, level_color, colorize_on);
                }
                Seg::Field(field) => {
                    let (value, numeric) = field_value(&field.kind, record);
                    if matches!(field.kind, FieldKind::Message) && record.colored_message {
                        // Parse markup inside the message, honoring the stack.
                        render_markup(&mut out, &mut stack, &value, level_color, colorize_on);
                    } else {
                        out.push_str(&field.spec.apply(&value, numeric));
                    }
                }
            }
        }
        if colorize_on && !stack.is_empty() {
            out.push_str(RESET);
        }
        out
    }
}

fn parse_field(inner: &str) -> Field {
    // Split name and spec at the first ':'.
    let (name, spec_str) = match inner.find(':') {
        Some(idx) => (&inner[..idx], &inner[idx + 1..]),
        None => (inner, ""),
    };
    let name = name.trim();

    let kind = if let Some(key) = name
        .strip_prefix("extra[")
        .and_then(|s| s.strip_suffix(']'))
    {
        FieldKind::ExtraKey(key.to_string())
    } else {
        match name {
            "time" => FieldKind::Time(if spec_str.is_empty() {
                "YYYY-MM-DD HH:mm:ss.SSS".to_string()
            } else {
                spec_str.to_string()
            }),
            "level" => FieldKind::Level,
            "level.name" => FieldKind::LevelName,
            "level.no" => FieldKind::LevelNo,
            "level.icon" => FieldKind::LevelIcon,
            "message" => FieldKind::Message,
            "name" => FieldKind::Name,
            "function" => FieldKind::Function,
            "module" => FieldKind::Module,
            "file" => FieldKind::File,
            "file.name" => FieldKind::FileName,
            "file.path" => FieldKind::FilePath,
            "line" => FieldKind::Line,
            "process" | "process.id" => FieldKind::Process,
            "process.name" => FieldKind::ProcessName,
            "thread" | "thread.id" => FieldKind::Thread,
            "thread.name" => FieldKind::ThreadName,
            "elapsed" => FieldKind::Elapsed,
            "exception" => FieldKind::Exception,
            "extra" => FieldKind::ExtraAll,
            other => FieldKind::Unknown(other.to_string()),
        }
    };

    // Time uses its whole spec as a datetime format, not an align/width spec.
    let spec = match &kind {
        FieldKind::Time(_) => Spec::none(),
        _ => parse_spec(spec_str),
    };

    Field { kind, spec }
}

/// Compute a field's raw value and whether it is numeric (for default
/// alignment).
fn field_value(kind: &FieldKind, record: &Record) -> (String, bool) {
    match kind {
        FieldKind::Time(spec) => {
            let strftime = translate_time_format(spec);
            (record.time.format(&strftime).to_string(), false)
        }
        FieldKind::Level | FieldKind::LevelName => (record.level.name.clone(), false),
        FieldKind::LevelNo => (record.level.no.to_string(), true),
        FieldKind::LevelIcon => (record.level.icon.clone(), false),
        FieldKind::Message => (record.message.clone(), false),
        FieldKind::Name => {
            let n = &*record.logger_name;
            (if n.is_empty() { "root" } else { n }.to_string(), false)
        }
        FieldKind::Function => (record.location.function.to_string(), false),
        FieldKind::Module => (record.location.module_path.to_string(), false),
        FieldKind::File | FieldKind::FileName => (record.location.file_name().to_string(), false),
        FieldKind::FilePath => (record.location.file.to_string(), false),
        FieldKind::Line => (record.location.line.to_string(), true),
        FieldKind::Process => (record.process_id.to_string(), true),
        FieldKind::ProcessName => ("MainProcess".to_string(), false),
        FieldKind::Thread => (record.thread_id.to_string(), true),
        FieldKind::ThreadName => (
            record.thread_name.clone().unwrap_or_default(),
            false,
        ),
        FieldKind::Elapsed => (format_elapsed(record.elapsed), false),
        FieldKind::Exception => (record.exception.clone().unwrap_or_default(), false),
        FieldKind::ExtraAll => {
            let mut s = String::new();
            let mut first = true;
            for (k, v) in &record.extra {
                if !first {
                    s.push_str(", ");
                }
                first = false;
                let _ = write!(s, "{k}={v}");
            }
            (s, false)
        }
        FieldKind::ExtraKey(key) => {
            (record.extra.get(key).cloned().unwrap_or_default(), false)
        }
        FieldKind::Unknown(name) => (format!("{{{name}}}"), false),
    }
}

fn format_elapsed(d: std::time::Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    let ms = d.subsec_millis();
    format!("{h}:{m:02}:{s:02}.{ms:03}")
}

/// Translate a loguru/pendulum-style datetime format into a chrono strftime
/// string. Text inside `[...]` is treated as a literal.
fn translate_time_format(fmt: &str) -> String {
    // Tokens, longest first so greedy matching is correct.
    const TOKENS: &[(&str, &str)] = &[
        ("SSSSSS", "%6f"),
        ("YYYY", "%Y"),
        ("dddd", "%A"),
        ("MMMM", "%B"),
        ("MMM", "%b"),
        ("ddd", "%a"),
        ("SSS", "%3f"),
        ("YY", "%y"),
        ("MM", "%m"),
        ("DD", "%d"),
        ("HH", "%H"),
        ("hh", "%I"),
        ("mm", "%M"),
        ("ss", "%S"),
        ("ZZ", "%z"),
        ("zz", "%Z"),
        ("A", "%p"),
        ("Z", "%:z"),
        ("M", "%m"),
        ("D", "%d"),
        ("H", "%H"),
        ("h", "%I"),
        ("m", "%M"),
        ("s", "%S"),
        ("d", "%a"),
        ("X", "%s"),
        ("E", "%u"),
    ];

    let mut out = String::with_capacity(fmt.len() + 8);
    let bytes = fmt.as_bytes();
    let mut i = 0;
    'outer: while i < fmt.len() {
        if bytes[i] == b'[' {
            if let Some(end) = fmt[i..].find(']') {
                out.push_str(&fmt[i + 1..i + end]);
                i += end + 1;
                continue;
            }
        }
        // chrono uses '%' specially; escape a literal '%'.
        if bytes[i] == b'%' {
            out.push_str("%%");
            i += 1;
            continue;
        }
        for (token, repl) in TOKENS {
            if fmt[i..].starts_with(token) {
                out.push_str(repl);
                i += token.len();
                continue 'outer;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

// ===================================================================
// JSON formatter (serialize=True)
// ===================================================================

/// Format a record as a single-line JSON object (loguru's `serialize=True`).
pub fn json_format(record: &Record) -> String {
    let mut out = String::with_capacity(160 + record.message.len());
    out.push('{');
    let _ = write!(
        out,
        "\"time\":\"{}\",\"level\":\"{}\",\"severity\":{},\"elapsed\":{:.3},\"logger\":",
        record.time.to_rfc3339(),
        record.level.name,
        record.level.no,
        record.elapsed.as_secs_f64(),
    );
    push_json_string(&mut out, &record.logger_name);
    out.push_str(",\"message\":");
    push_json_string(&mut out, &record.message);
    if !record.location.module_path.is_empty() {
        out.push_str(",\"module\":");
        push_json_string(&mut out, record.location.module_path);
    }
    if !record.location.function.is_empty() {
        out.push_str(",\"function\":");
        push_json_string(&mut out, record.location.function);
    }
    if !record.location.file.is_empty() {
        out.push_str(",\"file\":");
        push_json_string(&mut out, record.location.file);
        let _ = write!(out, ",\"line\":{}", record.location.line);
    }
    let _ = write!(out, ",\"process\":{}", record.process_id);
    let _ = write!(out, ",\"thread\":{}", record.thread_id);
    if let Some(thread) = &record.thread_name {
        out.push_str(",\"thread_name\":");
        push_json_string(&mut out, thread);
    }
    if let Some(exc) = &record.exception {
        out.push_str(",\"exception\":");
        push_json_string(&mut out, exc);
    }
    if !record.extra.is_empty() {
        out.push_str(",\"extra\":{");
        let mut first = true;
        for (k, v) in &record.extra {
            if !first {
                out.push(',');
            }
            first = false;
            push_json_string(&mut out, k);
            out.push(':');
            push_json_string(&mut out, v);
        }
        out.push('}');
    }
    out.push('}');
    out
}

fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
