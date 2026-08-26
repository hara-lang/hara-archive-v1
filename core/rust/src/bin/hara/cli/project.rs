use super::Options;
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
use std::path::PathBuf;

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

fn project_for(options: &Options, args: &[String]) -> Result<project::Project, String> {
    let path = args
        .first()
        .map(PathBuf::from)
        .or_else(|| options.project.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    project::discover(&path)
}

fn eval_runtime(options: &Options) -> Result<Runtime, String> {
    let mut runtime = Runtime::new();
    if let Some(path) = options.lite_project.as_deref() {
        let project = project::discover(path)?;
        project::register_sources(&project, &mut runtime)?;
    }
    if options.project.is_some() {
        let project = project_for(options, &[])?;
        project::register_sources(&project, &mut runtime)?;
    }
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
    let broker = RuntimeBroker::start_with(
        options.root.clone().or_else(|| options.project.clone()),
        options.native_sockets,
        options.allow_process,
        options.allow_postgres,
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
    let broker = RuntimeBroker::start_with(
        options.root.clone().or_else(|| options.project.clone()),
        options.native_sockets,
        options.allow_process,
        options.allow_postgres,
    )?;
    for path in [options.lite_project.as_deref(), options.project.as_deref()]
        .into_iter()
        .flatten()
    {
        let selected = project::discover(path)?;
        for (namespace, source) in project::source_resources(&selected)? {
            broker.register_resource(&namespace, &source)?;
        }
    }
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

#[cfg(test)]
#[path = "project/tests.rs"]
mod tests;
