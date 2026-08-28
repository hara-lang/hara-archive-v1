use hara_wasm::cli_app;
use hara_wasm::project as project_model;
use std::env;
use std::io::{self, Read};
use std::path::PathBuf;

#[path = "cli/hara.rs"]
mod hara;
#[path = "cli/project.rs"]
mod project;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionBackend {
    Interpreter,
    Native,
}

impl Default for ExecutionBackend {
    fn default() -> Self {
        Self::Native
    }
}

impl ExecutionBackend {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "interpreter" => Ok(Self::Interpreter),
            "native" => Ok(Self::Native),
            _ => Err(format!(
                "unknown backend {value}; expected interpreter or native"
            )),
        }
    }

    pub(crate) fn runtime_name(self) -> &'static str {
        match self {
            Self::Interpreter => "interpreter",
            Self::Native => "direct-native",
        }
    }
}

#[derive(Default)]
pub(crate) struct Options {
    pub(crate) root: Option<PathBuf>,
    pub(crate) project: Option<PathBuf>,
    pub(crate) lite_project: Option<PathBuf>,
    pub(crate) lite_mode: bool,
    pub(crate) backend: ExecutionBackend,
    pub(crate) native_sockets: bool,
    pub(crate) allow_file: bool,
    pub(crate) allow_process: bool,
    pub(crate) allow_postgres: bool,
    pub(crate) log_requests: bool,
    pub(crate) offline: bool,
    pub(crate) host: String,
    pub(crate) port: u16,
    command: Vec<String>,
    pub(crate) history_file: Option<PathBuf>,
    pub(crate) no_history: bool,
    pub(crate) no_splash: bool,
    pub(crate) no_color: bool,
}

pub(crate) fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        host: "127.0.0.1".into(),
        port: 1311,
        ..Options::default()
    };
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--" {
            options.command.extend(args);
            break;
        }
        match argument.as_str() {
            "--help" | "-h" => {
                options.command.push(argument);
                options.command.extend(args);
                break;
            }
            "--version" | "-V" => {
                options.command.push(argument);
                options.command.extend(args);
                break;
            }
            "--root" => options.root = Some(PathBuf::from(required(&mut args, "--root")?)),
            "--project" => options.project = Some(PathBuf::from(required(&mut args, "--project")?)),
            "--backend" => {
                options.backend = ExecutionBackend::parse(&required(&mut args, "--backend")?)?
            }
            "--native-sockets" | "--allow-net" => options.native_sockets = true,
            "--allow-file" => options.allow_file = true,
            "--allow-process" => options.allow_process = true,
            "--allow-postgres" => options.allow_postgres = true,
            "--log-requests" => options.log_requests = true,
            "--offline" => options.offline = true,
            "--no-history" => options.no_history = true,
            "--no-splash" => options.no_splash = true,
            "--no-color" => options.no_color = true,
            "--history" => {
                options.history_file = Some(PathBuf::from(required(&mut args, "--history")?))
            }
            "--host" => options.host = required(&mut args, "--host")?,
            "--port" => {
                options.port = required(&mut args, "--port")?
                    .parse()
                    .map_err(|_| "--port must be between 0 and 65535".to_owned())?
            }
            value if value.starts_with("--history=") => {
                options.history_file = Some(PathBuf::from(&value[10..]))
            }
            value if value.starts_with("--root=") => {
                options.root = Some(PathBuf::from(option_value(value, "--root")?))
            }
            value if value.starts_with("--project=") => {
                options.project = Some(PathBuf::from(option_value(value, "--project")?))
            }
            value if value.starts_with("--backend=") => {
                options.backend = ExecutionBackend::parse(option_value(value, "--backend")?)?
            }
            value if value.starts_with("--host=") => {
                options.host = option_value(value, "--host")?.to_owned()
            }
            value if value.starts_with("--port=") => {
                options.port = option_value(value, "--port")?
                    .parse()
                    .map_err(|_| "--port must be between 0 and 65535".to_owned())?
            }
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            value => {
                options.command.push(value.into());
                options.command.extend(args);
                break;
            }
        }
    }
    Ok(options)
}

fn required(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn option_value<'a>(argument: &'a str, option: &str) -> Result<&'a str, String> {
    let value = argument
        .strip_prefix(option)
        .and_then(|value| value.strip_prefix('='))
        .unwrap_or_default();
    if value.is_empty() {
        Err(format!("{option} requires a value"))
    } else {
        Ok(value)
    }
}

pub(crate) fn run(mut options: Options) -> Result<(), String> {
    options.lite_project = distributed_lite_project();
    if options.allow_postgres {
        if let Some(path) = options.project.as_deref() {
            let project = project_model::discover(path)?;
            if !project
                .capabilities
                .iter()
                .any(|value| value == "db/postgres")
            {
                return Err("project must declare :db/postgres before --allow-postgres".into());
            }
        }
    }
    // Project aliases are argv-only macros, expanded before the normative
    // route table.  A directory without project.edn simply has no aliases.
    let expanded = match project_model::discover(
        options
            .project
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new(".")),
    ) {
        Ok(project) => project_model::expand_aliases(&project, &options.command)?,
        Err(_) => options.command.clone(),
    };
    if expanded.first().map(String::as_str) == Some("package")
        && expanded.get(1).map(String::as_str) == Some("profile")
    {
        return hara_wasm::package::run(&expanded[1..]);
    }
    if expanded.first().map(String::as_str) == Some("package")
        && expanded.get(1).map(String::as_str) == Some("build")
        && expanded[2..].iter().any(|argument| {
            argument == "--package"
                || argument.starts_with("--package=")
                || argument == "--profile"
                || argument.starts_with("--profile=")
        })
    {
        return hara_wasm::package::run(&expanded[1..]);
    }
    hara::run(&options, &expanded)
}

/// Runs the dependency-light native CLI surface without loading `tool.cli`.
///
/// This path is intentionally limited to evaluator and REPL operations so a
/// source checkout remains usable while higher-level HAL tooling is being
/// repaired. Project sources are indexed lazily from `project.edn` when
/// `--project` is supplied; the evaluator reads each namespace's `.hal` file
/// when it is required.
pub(crate) fn run_lite(mut options: Options) -> Result<(), String> {
    options.lite_mode = true;
    options.lite_project = bundled_lite_project();
    let command = options.command.clone();
    match command.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            usage_lite();
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("hara lite {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("eval") => project::direct_eval(&options, &command[1..].join(" ")),
        Some("run" | "--file") => project::run_file(
            &options,
            command
                .get(1)
                .ok_or_else(|| "run requires a file path".to_owned())?,
        ),
        Some("stdin") => {
            let mut source = String::new();
            io::stdin()
                .read_to_string(&mut source)
                .map_err(|error| format!("stdin: {error}"))?;
            project::direct_eval(&options, &source)
        }
        Some("headless" | "server") => project::run_headless(&options),
        Some("remote") => project::run_remote(
            command
                .get(1)
                .ok_or_else(|| "remote requires HOST:PORT".to_owned())?,
        ),
        Some("repl") | None => crate::repl::run_repl(&options, true),
        Some(command) => Err(format!("unknown lite command: {command}")),
    }
}

pub(crate) fn project_source_catalog(
    options: &Options,
) -> Result<Option<hara_wasm::project::SourceCatalog>, String> {
    project::source_catalog_for_options(options)
}

fn bundled_lite_project() -> Option<PathBuf> {
    distributed_lite_project().or_else(|| {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../");
        repository
            .join("project.edn")
            .is_file()
            .then_some(repository)
    })
}

fn distributed_lite_project() -> Option<PathBuf> {
    if let Some(path) = env::var_os("HARA_LITE_PROJECT") {
        return Some(PathBuf::from(path));
    }
    let executable = env::current_exe().ok()?;
    let prefix = executable.parent()?.parent()?;
    let project = prefix.join("share/hara-lite");
    project.join("project.edn").is_file().then_some(project)
}

pub(crate) fn usage_lite() {
    println!("Hara Lite · native Rust runtime");
    println!();
    println!("Usage:");
    println!("  hara-lite [OPTIONS] repl");
    println!("  hara-lite [OPTIONS] eval EXPRESSION");
    println!("  hara-lite [OPTIONS] run FILE");
    println!("  hara-lite [OPTIONS] stdin");
    println!("  hara-lite [OPTIONS] headless");
    println!("  hara-lite remote HOST:PORT");
    println!();
    println!("Options:");
    println!("  --project PATH, --root PATH");
    println!("  --backend native|interpreter (default: native)");
    println!("  --allow-file, --allow-net, --allow-process, --allow-postgres");
    println!("  --no-history, --no-splash, --no-color");
}

pub(crate) fn error_exit_code(error: &str) -> i32 {
    if error.starts_with("unknown ")
        || error.starts_with("usage:")
        || error.starts_with("unavailable:")
        || error.starts_with("--offline cannot")
        || error.contains(" requires ")
        || error.contains("cannot read")
        || error.contains("Cannot read")
        || error.contains("not found")
    {
        cli_app::CliOutcome::UsageError.exit_code()
    } else {
        cli_app::CliOutcome::Failed.exit_code()
    }
}

pub(crate) fn exit_error(message: &str, status: i32) -> ! {
    eprintln!("hara: {message}");
    std::process::exit(status)
}

#[cfg(test)]
mod tests {
    use super::{ExecutionBackend, Options};

    #[test]
    fn native_is_the_cli_backend_default() {
        assert_eq!(Options::default().backend, ExecutionBackend::Native);
    }

    #[test]
    fn cli_backend_names_map_to_runtime_backends() {
        assert_eq!(
            ExecutionBackend::parse("native").unwrap().runtime_name(),
            "direct-native"
        );
        assert_eq!(
            ExecutionBackend::parse("interpreter")
                .unwrap()
                .runtime_name(),
            "interpreter"
        );
        assert!(ExecutionBackend::parse("unknown").is_err());
    }
}
