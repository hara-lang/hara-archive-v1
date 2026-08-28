use hara_wasm::kernel::{parse, Form};
use hara_wasm::{project, Runtime};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

const APPLICATION_RESOURCE: &str = "hara.cli.application";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    project: PathBuf,
    allow_file: bool,
    allow_process: bool,
    native_sockets: bool,
    arguments: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            project: PathBuf::from("."),
            allow_file: false,
            allow_process: false,
            native_sockets: false,
            arguments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Run(Options),
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplicationResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn main() {
    let command = parse_options(env::args().skip(1)).unwrap_or_else(|error| exit_error(&error));
    match command {
        Command::Help => usage(),
        Command::Version => println!("hara-app {}", env!("CARGO_PKG_VERSION")),
        Command::Run(options) => match run(options) {
            Ok(status) if status == 0 => {}
            Ok(status) => std::process::exit(status),
            Err(error) => exit_error(&error),
        },
    }
}

fn parse_options(arguments: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut arguments = arguments.into_iter();
    let mut options = Options::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--" => {
                options.arguments.extend(arguments);
                break;
            }
            "--help" | "-h" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            "--project" => {
                options.project = PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--project requires a path".to_owned())?,
                );
            }
            "--allow-file" => options.allow_file = true,
            "--allow-process" => options.allow_process = true,
            "--allow-net" | "--native-sockets" => options.native_sockets = true,
            value if value.starts_with("--project=") => {
                let path = value.trim_start_matches("--project=");
                if path.is_empty() {
                    return Err("--project requires a path".into());
                }
                options.project = PathBuf::from(path);
            }
            value if value.starts_with('-') => {
                return Err(format!(
                    "unknown hara-app option: {value}; place application options after --"
                ));
            }
            value => {
                options.arguments.push(value.to_owned());
                options.arguments.extend(arguments);
                break;
            }
        }
    }
    Ok(Command::Run(options))
}

fn run(options: Options) -> Result<i32, String> {
    let project = project::discover(&options.project)?;
    let path = project::main_file(&project)?;
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut runtime = Runtime::new();
    runtime.register_resource(
        APPLICATION_RESOURCE,
        &application_request_source(
            &project.id,
            project.root.to_string_lossy().as_ref(),
            &options,
        ),
    );
    let source_catalog = project::source_catalog(&project)?;
    runtime.register_source_catalog(&source_catalog);
    if options.allow_file {
        runtime.install_native_file_provider(project.root.to_string_lossy().as_ref());
    }
    if options.native_sockets {
        runtime.install_native_socket_provider();
    }
    if options.allow_process {
        runtime.install_native_process_provider();
    }
    let evaluated = runtime.eval_native(&source)?;
    emit_application_result(&evaluated)
}

fn application_request_source(project: &str, cwd: &str, options: &Options) -> String {
    let mut capabilities = Vec::new();
    if options.allow_file {
        capabilities.push(Form::Keyword("file".into()));
    }
    if options.allow_process {
        capabilities.push(Form::Keyword("process".into()));
    }
    if options.native_sockets {
        capabilities.push(Form::Keyword("network".into()));
    }
    let request = Form::Map(vec![
        (
            Form::Keyword("hara.cli/route".into()),
            Form::Keyword("hara.cli.route/project-run".into()),
        ),
        (
            Form::Keyword("hara.cli/arguments".into()),
            Form::Vector(
                options
                    .arguments
                    .iter()
                    .cloned()
                    .map(Form::String)
                    .collect(),
            ),
        ),
        (
            Form::Keyword("hara.cli/runtime".into()),
            Form::Keyword("native".into()),
        ),
        (
            Form::Keyword("hara.cli/cwd".into()),
            Form::String(cwd.to_owned()),
        ),
        (
            Form::Keyword("hara.cli/project".into()),
            Form::String(project.to_owned()),
        ),
        (
            Form::Keyword("hara.cli/capabilities".into()),
            Form::Set(capabilities),
        ),
    ]);
    format!(
        "(ns {APPLICATION_RESOURCE})\n\n(def request {request})\n\n(def arguments (get request :hara.cli/arguments))\n"
    )
}

fn emit_application_result(evaluated: &str) -> Result<i32, String> {
    let Some(result) = decode_application_result(evaluated)? else {
        println!("{evaluated}");
        return Ok(0);
    };
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(result.stdout.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| format!("stdout: {error}"))?;
    let mut stderr = io::stderr().lock();
    stderr
        .write_all(result.stderr.as_bytes())
        .and_then(|()| stderr.flush())
        .map_err(|error| format!("stderr: {error}"))?;
    Ok(result.exit)
}

fn decode_application_result(evaluated: &str) -> Result<Option<ApplicationResult>, String> {
    let Form::Map(entries) = parse(evaluated)? else {
        return Ok(None);
    };
    let protocol = ["hara.app/stdout", "hara.app/stderr", "hara.app/exit"]
        .iter()
        .any(|key| map_get(&entries, key).is_some());
    if !protocol {
        return Ok(None);
    }
    let stdout = result_text(map_get(&entries, "hara.app/stdout"), "hara.app/stdout")?;
    let stderr = result_text(map_get(&entries, "hara.app/stderr"), "hara.app/stderr")?;
    let exit = match map_get(&entries, "hara.app/exit") {
        None | Some(Form::Nil) => 0,
        Some(Form::Number(value)) => i32::try_from(*value)
            .map_err(|_| ":hara.app/exit is outside the 32-bit exit-code range".to_owned())?,
        Some(value) => {
            return Err(format!(
                ":hara.app/exit must be an integer or nil, received {value}"
            ));
        }
    };
    if !(0..=255).contains(&exit) {
        return Err(":hara.app/exit must be between 0 and 255".into());
    }
    Ok(Some(ApplicationResult {
        stdout,
        stderr,
        exit,
    }))
}

fn result_text(value: Option<&Form>, key: &str) -> Result<String, String> {
    match value {
        None | Some(Form::Nil) => Ok(String::new()),
        Some(Form::String(value)) => Ok(value.clone()),
        Some(value) => Err(format!(":{key} must be a string or nil, received {value}")),
    }
}

fn map_get<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
    })
}

fn usage() {
    println!("Hara application runner");
    println!();
    println!("Usage:");
    println!(
        "  hara-app [--project PATH] [--allow-file] [--allow-process] [--allow-net] -- [ARGS...]"
    );
    println!();
    println!("The application receives a data request from hara.cli.application/request.");
    println!("A map with :hara.app/stdout, :hara.app/stderr and :hara.app/exit is emitted as a process result.");
}

fn exit_error(message: &str) -> ! {
    eprintln!("hara-app: {message}");
    std::process::exit(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hara_wasm::kernel::parse_forms;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn application_arguments_are_separate_from_runner_options() {
        assert_eq!(
            parse_options(strings(&[
                "--project",
                "demo",
                "--allow-file",
                "--allow-process",
                "--",
                "vault",
                "verify",
            ]))
            .unwrap(),
            Command::Run(Options {
                project: PathBuf::from("demo"),
                allow_file: true,
                allow_process: true,
                native_sockets: false,
                arguments: strings(&["vault", "verify"]),
            })
        );
    }

    #[test]
    fn request_is_data_and_records_only_granted_capabilities() {
        let options = Options {
            project: PathBuf::from("."),
            allow_file: true,
            allow_process: false,
            native_sockets: true,
            arguments: strings(&["status"]),
        };
        let source = application_request_source("demo/app", "/tmp/demo", &options);
        let forms = parse_forms(&source).unwrap();
        assert_eq!(forms.len(), 3);
        assert!(source.contains(":hara.cli/arguments [\"status\"]"));
        assert!(source.contains(":hara.cli/capabilities #{:file :network}"));
        assert!(!source.contains(":process"));
    }

    #[test]
    fn application_result_protocol_is_strict() {
        assert_eq!(
            decode_application_result(
                "{:hara.app/stdout \"out\" :hara.app/stderr \"err\" :hara.app/exit 7}"
            )
            .unwrap(),
            Some(ApplicationResult {
                stdout: "out".into(),
                stderr: "err".into(),
                exit: 7,
            })
        );
        assert!(decode_application_result("{:hara.app/stdout 1}").is_err());
        assert!(decode_application_result("{:hara.app/exit 256}").is_err());
        assert_eq!(decode_application_result("42").unwrap(), None);
    }
}
