use super::{ExecutionBackend, Options};
use crate::repl;
#[cfg(feature = "halc-encoder")]
use hara_wasm::kernel::{halc::encode_halc_module, parse_forms, Form};
use hara_wasm::native_cli::{install_native_kernel, RuntimeBroker};
use hara_wasm::project;
use hara_wasm::resp::{RespConnection, RespServer, RespValue};
use hara_wasm::Runtime;
use std::fs;
use std::io::{self, BufRead};
use std::net::TcpStream;

#[cfg(feature = "halc-encoder")]
pub(crate) fn compile_halc(args: &[String]) -> Result<(), String> {
    let source_path = args
        .first()
        .ok_or_else(|| "compile-halc requires SOURCE.hal --output OUTPUT.halc".to_owned())?;
    let output_index = args
        .iter()
        .position(|argument| argument == "--output")
        .ok_or_else(|| "compile-halc requires --output OUTPUT.halc".to_owned())?;
    let output_path = args
        .get(output_index + 1)
        .ok_or_else(|| "compile-halc requires --output OUTPUT.halc".to_owned())?;
    let resource = args
        .iter()
        .position(|argument| argument == "--resource")
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
        .unwrap_or(source_path);
    let source = fs::read_to_string(source_path)
        .map_err(|error| format!("cannot read {source_path}: {error}"))?;
    let forms = parse_forms(&source)?;
    let namespace = forms
        .iter()
        .find_map(|form| match form {
            Form::List(values)
                if matches!(values.first(), Some(Form::Symbol(head)) if head == "ns" || head == "ns+") =>
            {
                match values.get(1) {
                    Some(Form::Symbol(namespace)) => Some(namespace.clone()),
                    _ => None,
                }
            }
            _ => None,
        })
        .ok_or_else(|| format!("{source_path} does not declare an ns or ns+ namespace"))?;
    let artifact = encode_halc_module(&namespace, resource, &source, forms)?;
    if let Some(parent) = std::path::Path::new(output_path).parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::write(output_path, artifact).map_err(|error| format!("cannot write {output_path}: {error}"))
}

pub(crate) fn source_catalog_for_options(
    options: &Options,
) -> Result<Option<project::SourceCatalog>, String> {
    let mut projects = Vec::new();
    for path in [options.lite_project.as_deref(), options.project.as_deref()]
        .into_iter()
        .flatten()
    {
        let selected = project::discover(path)?;
        if projects
            .iter()
            .any(|current: &project::Project| current.manifest_path == selected.manifest_path)
        {
            continue;
        }
        projects.push(selected);
    }
    if projects.is_empty() {
        return Ok(None);
    }
    let references = projects.iter().collect::<Vec<_>>();
    project::source_catalogs(&references).map(Some)
}

fn eval_runtime(options: &Options) -> Result<Runtime, String> {
    let source_catalog = source_catalog_for_options(options)?;
    let mut runtime = if options.lite_mode {
        let source_catalog = source_catalog
            .as_ref()
            .ok_or_else(|| "hara-lite requires a bundled or explicit project.edn".to_owned())?;
        let mut runtime = Runtime::core();
        runtime.register_source_catalog(source_catalog);
        runtime.bootstrap_source_foundation()?;
        runtime
    } else {
        let mut runtime = Runtime::new();
        if let Some(source_catalog) = source_catalog.as_ref() {
            runtime.register_source_catalog(source_catalog);
        }
        runtime
    };
    runtime
        .set_execution_backend(options.backend.runtime_name())
        .map_err(|_| backend_error(options.backend))?;
    if let Some(root) = options.root.as_ref().or(options.project.as_ref()) {
        runtime.install_native_file_provider(root.to_string_lossy().as_ref());
    }
    if options.native_sockets {
        runtime.install_native_socket_provider();
    }
    if options.allow_process {
        runtime.install_native_process_provider();
    }
    if options.allow_postgres {
        runtime.install_native_module(hara_db_postgres::module())?;
    }
    let broker = RuntimeBroker::start_with_backend(
        options.root.clone().or_else(|| options.project.clone()),
        options.native_sockets,
        options.allow_process,
        options.allow_postgres,
        options.backend.runtime_name(),
    )?;
    install_native_kernel(&mut runtime, broker);
    Ok(runtime)
}

pub(crate) fn direct_eval(options: &Options, source: &str) -> Result<(), String> {
    if source.is_empty() {
        return Err("eval requires a Hara expression".into());
    }
    let mut runtime = eval_runtime(options)?;
    println!("{}", runtime.eval_native(source)?);
    Ok(())
}

pub(crate) fn run_file(options: &Options, path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let is_halc = path.ends_with(".halc")
        || path.ends_with(".hir")
        || bytes.starts_with(b"HALC")
        || bytes.starts_with(b"HIR\0");
    let mut runtime = eval_runtime(options)?;
    if is_halc {
        println!("{}", runtime.eval_halc(&bytes)?);
    } else {
        println!(
            "{}",
            runtime.eval_native(
                &String::from_utf8(bytes)
                    .map_err(|error| format!("{path} is not valid UTF-8: {error}"))?
            )?
        );
    }
    Ok(())
}

pub(crate) fn run_headless(options: &Options) -> Result<(), String> {
    if options.offline {
        return Err("--offline cannot be used with headless".into());
    }
    let root = options.root.clone().or_else(|| options.project.clone());
    let broker = if options.lite_mode {
        let source_catalog = source_catalog_for_options(options)?
            .ok_or_else(|| "hara-lite requires a bundled or explicit project.edn".to_owned())?;
        RuntimeBroker::start_with_source_catalog(
            root,
            options.native_sockets,
            options.allow_process,
            options.allow_postgres,
            options.backend.runtime_name(),
            source_catalog,
        )?
    } else if let Some(source_catalog) = source_catalog_for_options(options)? {
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
    let server = RespServer::start(&options.host, options.port, broker)?;
    println!("HARA RESP {} · session ROOT", server.endpoint());
    loop {
        std::thread::park();
    }
}

pub(crate) fn run_remote(endpoint: &str) -> Result<(), String> {
    let (host, port) = repl::parse_endpoint(endpoint, "127.0.0.1")?;
    let stream = TcpStream::connect((host.as_str(), port))
        .map_err(|error| format!("remote connect failed: {error}"))?;
    let mut connection = RespConnection::new(stream)?;
    connection.write(&RespValue::array(["HELLO", "4", "CLIENT", "HARA-REMOTE"]))?;
    println!(
        "{}",
        response_text(connection.read()?.ok_or("remote closed")?)
    );
    let mut request = 0_u64;
    for line in io::stdin().lock().lines() {
        let source = line.map_err(|error| format!("stdin: {error}"))?;
        if matches!(source.trim(), "/quit" | ":quit") {
            connection.write(&RespValue::array(["QUIT"]))?;
            break;
        }
        request += 1;
        let id = format!("REMOTE-{request}");
        connection.write(&RespValue::array(["EVAL", &id, source.trim()]))?;
        if let Some(value) = connection.read()? {
            println!("{}", response_text(value));
        }
        let _ = connection.read()?;
    }
    Ok(())
}

fn response_text(value: RespValue) -> String {
    match value {
        RespValue::Array(Some(values)) => values
            .into_iter()
            .map(response_text)
            .collect::<Vec<_>>()
            .join(" "),
        RespValue::Bulk(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        RespValue::Simple(value) | RespValue::Error(value) => value,
        RespValue::Integer(value) => value.to_string(),
        RespValue::Bulk(None) | RespValue::Array(None) => "nil".into(),
    }
}

fn backend_error(backend: ExecutionBackend) -> String {
    match backend {
        ExecutionBackend::Native => {
            "native backend requires a native build with the direct-native feature".into()
        }
        ExecutionBackend::Interpreter => "cannot configure interpreter backend".into(),
    }
}

#[cfg(test)]
#[path = "project/tests.rs"]
mod tests;
