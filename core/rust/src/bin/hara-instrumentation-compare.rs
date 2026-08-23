use std::{env, fs, path::PathBuf, process};

use serde_json::Value;

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-instrumentation-compare: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let rust_path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: hara-instrumentation-compare RUST-REPORT JAVA-REPORT")?,
    );
    let java_path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: hara-instrumentation-compare RUST-REPORT JAVA-REPORT")?,
    );
    if arguments.next().is_some() {
        return Err("usage: hara-instrumentation-compare RUST-REPORT JAVA-REPORT".into());
    }
    let rust = read_report(&rust_path)?;
    let java = read_report(&java_path)?;
    for report in [&rust, &java] {
        if report.get("schema").and_then(Value::as_str)
            != Some("hara.instrumentation.conformance-report/0-alpha")
        {
            return Err("unsupported instrumentation report schema".into());
        }
    }
    if rust["corpus"] != java["corpus"] || rust["cases"] != java["cases"] {
        return Err("Rust and Java instrumentation reports differ in corpus, state, event sequence, phase, generation, or location".into());
    }
    if rust["runtime"] == java["runtime"] {
        return Err("instrumentation reports must identify different runtimes".into());
    }
    let cases = rust["cases"].as_array().map_or(0, Vec::len);
    println!("cross-runtime instrumentation comparator passed: {cases} cases");
    Ok(())
}

fn read_report(path: &PathBuf) -> Result<Value, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("invalid report {}: {error}", path.display()))
}
