use crate::cli::Options;
use hara_wasm::native_cli::RuntimeBroker;
use hara_wasm::resp::RespServer;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::history::DefaultHistory;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Config, Context, Editor, Helper};
use std::collections::HashSet;
use std::env;
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

const COMMANDS: &[&str] = &[
    "/docs",
    "/walkthrough",
    "/help",
    "/history",
    "/clear",
    "/splash",
    "/status",
    "/resp",
    "/ns",
    "/doc",
    "/apropos",
    "/time",
    "/quit",
    "/exit",
];

const DOC_TOPICS: &[&str] = &[
    "language",
    "collections",
    "functions",
    "namespaces",
    "interop",
    "repl",
];
const WALKTHROUGH_ACTIONS: &[&str] = &["next", "prev", "1", "2", "3", "4", "5", "stop"];
const RESP_ACTIONS: &[&str] = &["start", "stop", "restart"];

struct RespController {
    host: String,
    port: u16,
    broker: RuntimeBroker,
    server: Option<RespServer>,
}

impl RespController {
    fn new(host: String, port: u16, broker: RuntimeBroker) -> Self {
        Self {
            host,
            port,
            broker,
            server: None,
        }
    }

    fn start(&mut self, endpoint: Option<&str>) -> Result<String, String> {
        if let Some(endpoint) = endpoint {
            let (host, port) = parse_endpoint(endpoint, &self.host)?;
            self.host = host;
            self.port = port;
        }
        if self.server.is_none() {
            self.server = Some(RespServer::start(
                &self.host,
                self.port,
                self.broker.clone(),
            )?);
        }
        Ok(self.status())
    }

    fn stop(&mut self) -> String {
        if let Some(mut server) = self.server.take() {
            server.stop();
        }
        self.status()
    }

    fn restart(&mut self, endpoint: Option<&str>) -> Result<String, String> {
        self.stop();
        self.start(endpoint)
    }

    fn status(&self) -> String {
        self.server.as_ref().map_or_else(
            || "RESP ○ offline".into(),
            |server| format!("RESP ● {}", server.endpoint()),
        )
    }
}

pub(crate) fn parse_endpoint(value: &str, fallback_host: &str) -> Result<(String, u16), String> {
    if let Ok(port) = value.parse::<u16>() {
        return Ok((fallback_host.into(), port));
    }
    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| "endpoint must be PORT or HOST:PORT".to_owned())?;
    let port = port
        .parse::<u16>()
        .map_err(|_| "port must be between 0 and 65535".to_owned())?;
    Ok((host.into(), port))
}

pub(crate) fn run_repl(options: &Options, offline: bool) -> Result<(), String> {
    let root = options.root.clone().or_else(|| options.project.clone());
    let source_catalog = crate::cli::project_source_catalog(options)?;
    let broker = if options.lite_mode {
        let source_catalog = source_catalog
            .ok_or_else(|| "hara-lite requires a bundled or explicit project.edn".to_owned())?;
        RuntimeBroker::start_with_source_catalog(
            root,
            options.native_sockets,
            options.allow_process,
            options.allow_postgres,
            options.backend.runtime_name(),
            source_catalog,
        )?
    } else if let Some(source_catalog) = source_catalog {
        RuntimeBroker::start_with_backend_and_source_catalog(
            root,
            options.native_sockets,
            options.allow_process,
            options.allow_postgres,
            options.backend.runtime_name(),
            source_catalog,
        )?
    } else {
        RuntimeBroker::start_with_backend(
            root,
            options.native_sockets,
            options.allow_process,
            options.allow_postgres,
            options.backend.runtime_name(),
        )?
    };
    let mut resp = RespController::new(options.host.clone(), options.port, broker.clone());
    if !offline {
        resp.start(None)?;
    }
    if !is_terminal() {
        return run_plain_repl(&broker, &mut resp);
    }

    let color = !options.no_color
        && env::var_os("NO_COLOR").is_none()
        && env::var("TERM").map_or(true, |term| term != "dumb");
    clear_terminal();
    print_header(&resp.status(), !options.no_splash, color);
    let symbols = Arc::new(RwLock::new(Vec::new()));
    let helper = ReplHelper {
        symbols: symbols.clone(),
        hinter: HistoryHinter::new(),
        color,
    };
    let config = Config::builder().auto_add_history(false).build();
    let mut editor = Editor::<ReplHelper, DefaultHistory>::with_config(config)
        .map_err(|error| format!("terminal initialization failed: {error}"))?;
    editor.set_helper(Some(helper));
    let history_file = options.history_file.clone().unwrap_or_else(history_file);
    if !options.no_history {
        let _ = editor.load_history(&history_file);
    }
    let mut state = ReplState::default();

    loop {
        *symbols.write().expect("completion symbols") = completion_values(&broker);
        let namespace = broker.namespace("ROOT").unwrap_or_else(|_| "user".into());
        match editor.readline(&session_prompt(&namespace, color)) {
            Ok(line) => {
                let source = line.trim();
                if source.is_empty() {
                    continue;
                }
                if source.starts_with('/')
                    || matches!(source, ":help" | ":history" | ":quit" | ":exit")
                {
                    let history = editor
                        .history()
                        .iter()
                        .map(|entry| entry.to_owned())
                        .collect::<Vec<_>>();
                    if !repl_command(source, &broker, &mut resp, &history, &mut state, color)? {
                        break;
                    }
                    continue;
                }
                if !options.no_history {
                    let _ = editor.add_history_entry(source);
                    let _ = editor.save_history(&history_file);
                }
                let started = Instant::now();
                match broker.eval("ROOT", source) {
                    Ok(value) => println!("=> {value}\n"),
                    Err(error) => eprintln!("{error}\n"),
                }
                state.last_elapsed = Some(started.elapsed());
            }
            Err(ReadlineError::Interrupted) => println!("^C\n"),
            Err(ReadlineError::Eof) => break,
            Err(error) => return Err(format!("terminal read failed: {error}")),
        }
    }
    if !options.no_history {
        let _ = editor.save_history(&history_file);
    }
    Ok(())
}

#[derive(Default)]
struct ReplState {
    last_elapsed: Option<std::time::Duration>,
    walkthrough_step: usize,
}

fn run_plain_repl(broker: &RuntimeBroker, resp: &mut RespController) -> Result<(), String> {
    let mut entered = Vec::new();
    let mut pending = String::new();
    let mut state = ReplState::default();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|error| format!("stdin: {error}"))?;
        let trimmed = line.trim();
        if pending.is_empty()
            && (trimmed.starts_with('/')
                || matches!(trimmed, ":help" | ":history" | ":quit" | ":exit"))
        {
            if !repl_command(trimmed, broker, resp, &entered, &mut state, false)? {
                break;
            }
            continue;
        }
        if !pending.is_empty() {
            pending.push('\n');
        }
        pending.push_str(&line);
        if incomplete(&pending) {
            continue;
        }
        let source = pending.trim();
        if !source.is_empty() {
            entered.push(source.into());
            let started = Instant::now();
            match broker.eval("ROOT", source) {
                Ok(value) => println!("=> {value}\n"),
                Err(error) => eprintln!("{error}\n"),
            }
            state.last_elapsed = Some(started.elapsed());
        }
        pending.clear();
    }
    if !pending.trim().is_empty() {
        return Err("Incomplete source".into());
    }
    Ok(())
}

fn repl_command(
    source: &str,
    broker: &RuntimeBroker,
    resp: &mut RespController,
    history: &[String],
    state: &mut ReplState,
    color: bool,
) -> Result<bool, String> {
    let command = source.trim();
    let parts = command.split_whitespace().collect::<Vec<_>>();
    match parts.first().copied().unwrap_or("") {
        "/quit" | "/exit" | ":quit" | ":exit" => return Ok(false),
        "/help" | ":help" => print_interactive_help(),
        "/docs" => print_docs(parts.get(1).copied().unwrap_or("")),
        "/walkthrough" => {
            state.walkthrough_step =
                print_walkthrough(parts.get(1).copied().unwrap_or(""), state.walkthrough_step);
        }
        "/history" | ":history" => {
            let query = parts.get(1..).unwrap_or(&[]).join(" ");
            for (index, value) in history.iter().enumerate() {
                if query.is_empty() || fuzzy_score(&query, value).is_some() {
                    println!("{}: {value}", index + 1);
                }
            }
            println!();
        }
        "/doc" if parts.len() == 2 => show_documentation(broker, parts[1]),
        "/apropos" if parts.len() >= 2 => print_apropos(broker, &parts[1..].join(" ")),
        "/time" => println!(
            "{}\n",
            state
                .last_elapsed
                .map(format_elapsed)
                .unwrap_or_else(|| "No evaluation yet.".into())
        ),
        "/clear" => {
            clear_terminal();
            print_header(&resp.status(), true, color);
        }
        "/splash" => print_header(&resp.status(), true, color),
        "/status" => print_header(&resp.status(), false, color),
        "/ns" if parts.len() == 1 => println!("{}\n", broker.namespace("ROOT")?),
        "/resp" if parts.len() == 1 => println!("{}\n", resp.status()),
        "/resp" if parts.get(1) == Some(&"start") => {
            println!("{}\n", resp.start(parts.get(2).copied())?)
        }
        "/resp" if parts.get(1) == Some(&"stop") => println!("{}\n", resp.stop()),
        "/resp" if parts.get(1) == Some(&"restart") => {
            println!("{}\n", resp.restart(parts.get(2).copied())?)
        }
        "/doc" => println!("Usage: /doc SYMBOL\n"),
        "/apropos" => println!("Usage: /apropos QUERY\n"),
        "/resp" => {
            println!("Usage: /resp [start [PORT|HOST:PORT]|stop|restart [PORT|HOST:PORT]]\n")
        }
        name => eprintln!("Unknown command: {name}. Try /help.\n"),
    }
    Ok(true)
}

fn print_interactive_help() {
    println!("\nREPL");
    help_entry("/help", "show this command guide");
    help_entry("/docs [TOPIC]", "browse built-in Hara documentation");
    help_entry(
        "/walkthrough [next|prev|1-5|stop]",
        "navigate the guided tour",
    );
    help_entry("/history [QUERY]", "search persistent input history");
    help_entry("/clear", "clear the terminal and redraw the menu");
    help_entry("/splash", "redraw the splash and menu");
    help_entry("/time", "show the last evaluation time");
    println!("\nSESSION · ROOT");
    help_entry("/status", "show session and listener status");
    help_entry("/ns", "show the current namespace");
    help_entry("/doc SYMBOL", "show symbol documentation");
    help_entry("/apropos QUERY", "search documented symbols");
    println!("\nRESP LISTENER");
    help_entry("/resp", "show listener status");
    help_entry("/resp start [PORT|HOST:PORT]", "start the listener");
    help_entry("/resp stop", "stop the listener; keep ROOT");
    help_entry(
        "/resp restart [PORT|HOST:PORT]",
        "restart the listener; keep ROOT",
    );
    println!("\nEXIT");
    help_entry("/quit", "leave Hara");
    help_entry("/exit", "leave Hara");
    println!("\nTab completes commands and visible Hara symbols.\n");
}

fn help_entry(command: &str, description: &str) {
    println!("  {command:<36} {description}");
}

fn print_docs(topic: &str) {
    println!();
    match topic.to_ascii_lowercase().as_str() {
        "" => {
            println!("HARA DOCS");
            help_entry("/docs language", "forms, literals, and evaluation");
            help_entry("/docs collections", "vectors, maps, sets, and sequences");
            help_entry("/docs functions", "bindings, functions, and metadata");
            help_entry("/docs namespaces", ".hal files, ns, :require, and :import");
            help_entry("/docs interop", "platform-neutral member access");
            help_entry("/docs repl", "discovery and terminal commands");
            println!("\nUse /doc SYMBOL for live API metadata or /walkthrough to learn by doing.");
        }
        "language" | "basics" => doc_section("LANGUAGE", &[
            "Lists are calls: (+ 1 2) evaluates to 3.",
            "Literal data includes nil, booleans, numbers, strings, keywords, vectors, maps, and sets.",
            "Core forms include quote, if, do, let, fn, def, and ns.",
        ]),
        "collections" => doc_section("COLLECTIONS", &[
            "Vectors: [1 2 3]    Maps: {:name \"Hara\"}    Sets: #{:a :b}",
            "Use get, assoc, dissoc, conj, count, first, rest, map, filter, and reduce.",
            "Collections are persistent values; updates return a new value.",
        ]),
        "functions" => doc_section("FUNCTIONS", &[
            "Anonymous: (fn [x] (* x x))",
            "Named: (def square (fn [x] (* x x)))",
            "Functions support fixed and variadic parameter lists; /doc shows arglists and metadata.",
        ]),
        "namespaces" | "packages" => doc_section("NAMESPACES · PACKAGES", &[
            "Hara source files use the .hal extension.",
            "Declare dependencies in ns with :require and aliases with :as.",
            "Runtime capabilities belong behind packaged modules and explicit :import declarations.",
        ]),
        "interop" => doc_section("INTEROP", &[
            "The . form is the platform-neutral member operation.",
            "(. value member args...) invokes a member; (. value field field) reads a field.",
            "The active runtime maps those operations onto native or packaged values.",
        ]),
        "repl" | "terminal" => doc_section("REPL", &[
            "Tab completes slash commands and visible symbols at the cursor.",
            "Use /doc SYMBOL, /apropos QUERY, /history, /time, /status, and /resp.",
            "Use /walkthrough next and /walkthrough prev to navigate the guided tour.",
        ]),
        value => println!("Unknown docs topic: {value}\nTry /docs for the topic index."),
    }
    println!();
}

fn doc_section(title: &str, lines: &[&str]) {
    println!("{title}");
    for line in lines {
        println!("  {line}");
    }
}

const WALKTHROUGH: &[(&str, &str, &str)] = &[
    (
        "Expressions",
        "Hara evaluates data-shaped expressions from the inside out.",
        "Try: (+ 1 2 3)",
    ),
    (
        "Data",
        "Vectors, maps, sets, keywords, strings, and numbers are literal values.",
        "Try: (get {:name \"Hara\" :kind :language} :name)",
    ),
    (
        "Bindings and functions",
        "Use let for local names and fn for reusable behavior.",
        "Try: (let [double (fn [x] (* x 2))] (double 21))",
    ),
    (
        "Namespaces and packages",
        "Hara source files use .hal; ns :require loads packaged interfaces.",
        "Example: (ns app.core (:require [demo.000-answer-42 :as answer]))",
    ),
    (
        "Discover and connect",
        "Use /doc SYMBOL and /apropos QUERY to explore; /resp controls the listener.",
        "Try: /doc map",
    ),
];

fn print_walkthrough(action: &str, current: usize) -> usize {
    if matches!(action, "stop" | "exit") {
        println!("\nWalkthrough closed. Use /walkthrough to begin again.\n");
        return 0;
    }
    let step = match action {
        "" | "start" => 1,
        "next" => (current.max(1) + 1).min(WALKTHROUGH.len()),
        "prev" | "previous" => current.saturating_sub(1).max(1),
        value => match value.parse::<usize>() {
            Ok(value) if (1..=WALKTHROUGH.len()).contains(&value) => value,
            _ => {
                println!("Usage: /walkthrough [next|prev|1-5|stop]\n");
                return current;
            }
        },
    };
    let page = WALKTHROUGH[step - 1];
    println!("\nWALKTHROUGH {step}/{} · {}", WALKTHROUGH.len(), page.0);
    println!("  {}\n  {}\n", page.1, page.2);
    if step < WALKTHROUGH.len() {
        println!("  Continue: /walkthrough next");
    } else {
        println!("  Complete. Use /docs to keep exploring.");
    }
    println!("  Navigate: /walkthrough prev · /walkthrough 1-5 · /walkthrough stop\n");
    step
}

fn show_documentation(broker: &RuntimeBroker, symbol: &str) {
    let escaped = symbol.replace('\\', "\\\\").replace('"', "\\\"");
    let doc = broker.eval("ROOT", &format!("(get (meta #'{escaped}) :doc)"));
    let arglists = broker.eval("ROOT", &format!("(get (meta #'{escaped}) :arglists)"));
    println!("\nDocumentation: {symbol}");
    if let Ok(value) = arglists {
        if value != "nil" {
            println!("  Arglists: {value}");
        }
    }
    match doc {
        Ok(value) if value != "nil" => println!("  {}", value.trim_matches('"')),
        _ => println!("No documentation for {symbol}"),
    }
    println!();
}

fn print_apropos(broker: &RuntimeBroker, query: &str) {
    if let Ok(symbols) = broker.complete("ROOT", "") {
        for symbol in symbols {
            if fuzzy_score(query, &symbol).is_none() {
                continue;
            }
            let escaped = symbol.replace('\\', "\\\\").replace('"', "\\\"");
            if let Ok(doc) = broker.eval("ROOT", &format!("(get (meta #'{escaped}) :doc)")) {
                if doc != "nil" {
                    println!("{symbol} — {}", doc.trim_matches('"'));
                }
            }
        }
    }
    println!();
}

fn format_elapsed(elapsed: std::time::Duration) -> String {
    let nanos = elapsed.as_nanos();
    if nanos < 1_000_000 {
        format!("{nanos} ns")
    } else if nanos < 1_000_000_000 {
        format!("{:.2} ms", nanos as f64 / 1_000_000.0)
    } else {
        format!("{:.2} s", elapsed.as_secs_f64())
    }
}

fn fuzzy_score(query: &str, value: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    if value == query {
        return Some(0);
    }
    let q = query.to_ascii_lowercase();
    let v = value.to_ascii_lowercase();
    if v.starts_with(&q) {
        return Some(10 + value.len().saturating_sub(query.len()));
    }
    if let Some(index) = v.find(&q) {
        return Some(100 + index);
    }
    let mut chars = q.chars();
    let mut wanted = chars.next()?;
    let mut gaps = 0;
    for ch in v.chars() {
        if ch == wanted {
            if let Some(next) = chars.next() {
                wanted = next;
            } else {
                return Some(200 + gaps);
            }
        } else {
            gaps += 1;
        }
    }
    None
}

fn completion_values(broker: &RuntimeBroker) -> Vec<String> {
    let mut values = COMMANDS
        .iter()
        .map(|value| (*value).into())
        .collect::<Vec<_>>();
    if let Ok(mut symbols) = broker.complete("ROOT", "") {
        values.append(&mut symbols);
    }
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
    values
}

struct ReplHelper {
    symbols: Arc<RwLock<Vec<String>>>,
    hinter: HistoryHinter,
    color: bool,
}

impl Helper for ReplHelper {}
impl Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, _position: usize) -> std::borrow::Cow<'l, str> {
        if !self.color || !line.starts_with('/') {
            return std::borrow::Cow::Borrowed(line);
        }
        let end = line.find(char::is_whitespace).unwrap_or(line.len());
        let command = &line[..end];
        let known = COMMANDS
            .iter()
            .any(|value| *value == command || value.starts_with(command));
        let shade = if known { "\x1b[36;1m" } else { "\x1b[31;1m" };
        let argument = if end < line.len() {
            format!("{}\x1b[35m{}", &line[end..=end], &line[end + 1..])
        } else {
            String::new()
        };
        std::borrow::Cow::Owned(format!("{shade}{command}\x1b[0m{argument}\x1b[0m"))
    }
}
impl Hinter for ReplHelper {
    type Hint = String;
    fn hint(&self, line: &str, position: usize, context: &Context<'_>) -> Option<String> {
        command_hint(line).or_else(|| self.hinter.hint(line, position, context))
    }
}
impl Validator for ReplHelper {
    fn validate(&self, context: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(if incomplete(context.input()) {
            ValidationResult::Incomplete
        } else {
            ValidationResult::Valid(None)
        })
    }
}
impl Completer for ReplHelper {
    type Candidate = Pair;
    fn complete(
        &self,
        line: &str,
        position: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let start = line[..position]
            .rfind(|ch: char| ch.is_whitespace() || "()[]{}\"'".contains(ch))
            .map_or(0, |index| index + 1);
        let prefix = &line[start..position];
        let choices: Vec<String> = if line.starts_with("/docs ") {
            DOC_TOPICS.iter().map(|v| (*v).into()).collect()
        } else if line.starts_with("/walkthrough ") {
            WALKTHROUGH_ACTIONS.iter().map(|v| (*v).into()).collect()
        } else if line.starts_with("/resp ") {
            RESP_ACTIONS.iter().map(|v| (*v).into()).collect()
        } else {
            self.symbols.read().expect("completion symbols").clone()
        };
        let mut values = choices
            .into_iter()
            .enumerate()
            .filter_map(|(order, value)| {
                fuzzy_score(prefix, &value).map(|score| (score, order, value))
            })
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        Ok((
            start,
            values
                .into_iter()
                .map(|(_, _, value)| Pair {
                    display: value.clone(),
                    replacement: value,
                })
                .collect(),
        ))
    }
}

fn command_hint(line: &str) -> Option<String> {
    let fixed = match line {
        "/docs" => Some("  [language · collections · functions · namespaces · interop · repl]"),
        "/walkthrough" => Some("  [next · prev · 1-5 · stop]"),
        "/resp" => Some("  [start · stop · restart]"),
        "/doc" => Some("  SYMBOL"),
        "/apropos" => Some("  QUERY"),
        "/history" => Some("  [QUERY]"),
        _ => None,
    };
    if let Some(value) = fixed {
        return Some(value.into());
    }
    for command in COMMANDS {
        if line.starts_with('/')
            && !line.contains(' ')
            && command.starts_with(line)
            && *command != line
        {
            return Some(command[line.len()..].into());
        }
    }
    None
}

fn incomplete(source: &str) -> bool {
    let mut stack = Vec::new();
    let mut string = false;
    let mut escape = false;
    let mut comment = false;
    for ch in source.chars() {
        if comment {
            if ch == '\n' {
                comment = false;
            }
            continue;
        }
        if string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                string = false;
            }
            continue;
        }
        match ch {
            ';' => comment = true,
            '"' => string = true,
            '(' | '[' | '{' => stack.push(ch),
            ')' => {
                if stack.last() == Some(&'(') {
                    stack.pop();
                }
            }
            ']' => {
                if stack.last() == Some(&'[') {
                    stack.pop();
                }
            }
            '}' => {
                if stack.last() == Some(&'{') {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    string || !stack.is_empty()
}

fn history_file() -> PathBuf {
    env::var_os("HARA_HISTORY")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".hara_history")))
        .unwrap_or_else(|| PathBuf::from(".hara_history"))
}

const DEFAULT_SPLASH: &str = r#"


                               ░░░▒▒▓▒▒░░░
                          ░░░░░▒▒▒▒▒▓▒▒▒▒▒░░░░░
                     ░░░░░▒▒▒▒▒▒▓▓▓▓▓▓▓▓▓▒▒▒▒▒▒░░░░░
                ░░░░░▒▒▒▒▒▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▒▒▒▒▒▒▒░░░░░

          ██╗       ██╗   ███████╗    ███████╗     ███████╗
          ██║       ██║  ██╔═════██╗  ██╔════██╗   ██╔═════██╗
          ██║  ●────██║  ██║     ██║  ██║     ██║  ██║     ██║
          ████████████║  ███████████║  ████████╔╝   ███████████║
          ██╔═══════██║  ██╔══════██║  ██╔═══██╗    ██╔══════██║
          ██║ ──●── ██║  ██║  ●───██║  ██║    ██╗   ██║  ●───██║
          ██║       ██║  ██║      ██║  ██║     ██╗  ██║      ██║
          ╚═╝       ╚═╝  ╚═╝      ╚═╝  ╚═╝     ╚═╝  ╚═╝      ╚═╝
                ·───────●───────────────●───────────────·
      "#;

fn print_header(resp: &str, include_splash: bool, color: bool) {
    if include_splash {
        println!("{}\n", rendered_splash(color));
    }
    println!("{:<52}SESSION ROOT", "HARA · RUST");
    println!("{}", tagline("JOURNEY WITHIN", color));
    println!("────────────────────────────────────────────────────────────────\n");
    println!("  /docs  Docs       /walkthrough  Tour");
    println!("  /help  Help       /history      History");
    println!("  /status Status    /resp         Listener");
    println!("  /clear Clear      /quit         Exit\n");
    println!("RESP  {resp}\n");
}

fn rendered_splash(color: bool) -> String {
    let value = DEFAULT_SPLASH.trim_end();
    if !color {
        return value.into();
    }
    let lines = value.lines().collect::<Vec<_>>();
    let triangle = &[
        (255, 246, 150),
        (235, 246, 185),
        (170, 226, 230),
        (85, 170, 255),
    ];
    let word = &[
        (105, 245, 255),
        (35, 185, 255),
        (45, 105, 255),
        (105, 65, 235),
        (185, 65, 220),
        (70, 20, 100),
        (5, 8, 20),
    ];
    let word_length = (lines.len().saturating_sub(8)).max(1);
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index < 2 || index == 6 {
                return (*line).to_owned();
            }
            let (position, stops) = if index < 7 {
                ((index - 2) as f64 / 3.0, triangle.as_slice())
            } else {
                ((index - 7) as f64 / word_length as f64, word.as_slice())
            };
            let (r, g, b) = gradient(position, stops);
            format!("\x1b[38;2;{r};{g};{b}m{line}\x1b[0m")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn gradient(position: f64, stops: &[(i32, i32, i32)]) -> (i32, i32, i32) {
    let scaled = position.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let from = (scaled as usize).min(stops.len() - 2);
    let phase = scaled - from as f64;
    let blend = |a: i32, b: i32| (a as f64 + (b - a) as f64 * phase).round() as i32;
    (
        blend(stops[from].0, stops[from + 1].0),
        blend(stops[from].1, stops[from + 1].1),
        blend(stops[from].2, stops[from + 1].2),
    )
}

fn tagline(text: &str, color: bool) -> String {
    if !color {
        return text.into();
    }
    let stops = [
        (100, 245, 255),
        (45, 145, 255),
        (125, 75, 235),
        (220, 90, 205),
    ];
    let length = text.chars().count().saturating_sub(1).max(1);
    let mut result = String::new();
    for (index, ch) in text.chars().enumerate() {
        if ch.is_whitespace() {
            result.push(ch);
            continue;
        }
        let (r, g, b) = gradient(index as f64 / length as f64, &stops);
        result.push_str(&format!("\x1b[38;2;{r};{g};{b}m{ch}"));
    }
    result.push_str("\x1b[0m");
    result
}

fn session_prompt(namespace: &str, color: bool) -> String {
    if color {
        format!("\x1b[2m[\x1b[0m\x1b[36;1m{namespace}\x1b[0m\x1b[2m] \x1b[0m")
    } else {
        format!("[{namespace}] ")
    }
}

fn clear_terminal() {
    print!("\x1b[2J\x1b[H");
}

fn is_terminal() -> bool {
    unsafe { libc_isatty(0) != 0 }
}

#[cfg(unix)]
unsafe fn libc_isatty(fd: i32) -> i32 {
    unsafe extern "C" {
        fn isatty(fd: i32) -> i32;
    }
    unsafe { isatty(fd) }
}
#[cfg(not(unix))]
unsafe fn libc_isatty(_fd: i32) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::{command_hint, fuzzy_score, gradient, incomplete, rendered_splash, DEFAULT_SPLASH};
    use crate::cli::Options;
    use hara_wasm::native_cli::RuntimeBroker;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hara-repl-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn multiline_detection_ignores_strings_comments_and_escapes() {
        assert!(incomplete("(defn value [x]\n  (+ x 1)"));
        assert!(!incomplete("(defn value [x]\n  (+ x 1))"));
        assert!(!incomplete("(str \"[\" ) ; ("));
    }

    #[test]
    fn splash_and_gradients_match_the_java_repl_contract() {
        assert_eq!(
            gradient(0.0, &[(255, 246, 150), (85, 170, 255)]),
            (255, 246, 150)
        );
        assert_eq!(
            gradient(0.5, &[(255, 246, 150), (85, 170, 255)]),
            (170, 208, 203)
        );
        assert_eq!(rendered_splash(false), DEFAULT_SPLASH.trim_end());
        assert!(rendered_splash(true).contains("\x1b[38;2;255;246;150m"));
        assert!(rendered_splash(true).contains("\x1b[38;2;5;8;20m"));
    }

    #[test]
    fn completion_scoring_and_hints_match_java_behavior() {
        assert!(fuzzy_score("mp", "map").is_some());
        assert!(fuzzy_score("zzz", "map").is_none());
        assert_eq!(command_hint("/re"), Some("sp".into()));
        assert!(command_hint("/docs").unwrap().contains("language"));
    }

    #[test]
    fn project_resources_reach_root_and_future_sessions() {
        let root = temp("project-resources");
        fs::create_dir_all(root.join("packages/core/src/demo")).unwrap();
        fs::create_dir_all(root.join("src/demo")).unwrap();
        fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/repl :project/version \"1.0.0\" :project/source-paths [\"packages/core/src\" \"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}").unwrap();
        fs::write(
            root.join("packages/core/src/demo/helper.hal"),
            "(ns demo.helper) (defn answer [] 40)",
        )
        .unwrap();
        fs::write(
            root.join("src/demo/app.hal"),
            "(ns demo.app (:require [demo.helper :as helper])) (defn answer [] (+ 2 (helper/answer)))",
        )
        .unwrap();

        let mut options = Options::default();
        options.project = Some(root.clone());
        options.root = Some(root.clone());
        let catalog = crate::cli::project_source_catalog(&options)
            .unwrap()
            .unwrap();
        assert_eq!(
            catalog.namespaces().collect::<Vec<_>>(),
            vec!["demo.app", "demo.helper"]
        );
        let broker = RuntimeBroker::start_with_backend_and_source_catalog(
            Some(root.clone()),
            false,
            false,
            false,
            "interpreter",
            catalog,
        )
        .unwrap();
        assert!(broker.resources().unwrap().is_empty());
        assert_eq!(
            broker
                .eval(
                    "ROOT",
                    "(ns repl.probe (:require [demo.app :as app])) (app/answer)",
                )
                .unwrap(),
            "42"
        );
        broker.create("SECOND").unwrap();
        assert_eq!(
            broker
                .eval(
                    "SECOND",
                    "(ns repl.second (:require [demo.app :as app])) (app/answer)",
                )
                .unwrap(),
            "42"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
