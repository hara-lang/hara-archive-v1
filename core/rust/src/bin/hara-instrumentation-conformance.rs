use std::{env, fs, path::PathBuf, process};

use hara_wasm::instrumentation::conformance::report;
use serde_json::Value;

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-instrumentation-conformance: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (corpus_path, output_path) = arguments()?;
    let corpus: Value = serde_json::from_str(
        &fs::read_to_string(&corpus_path)
            .map_err(|error| format!("cannot read {}: {error}", corpus_path.display()))?,
    )
    .map_err(|error| format!("invalid instrumentation corpus: {error}"))?;
    let report = report(&corpus, "rust")?;
    let encoded = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create report directory: {error}"))?;
        }
        fs::write(&path, format!("{encoded}\n"))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    } else {
        println!("{encoded}");
    }
    Ok(())
}

fn arguments() -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut arguments = env::args().skip(1);
    let mut corpus = None;
    let mut output = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--corpus" => {
                corpus = Some(PathBuf::from(
                    arguments.next().ok_or("missing --corpus path")?,
                ))
            }
            "--output" => {
                output = Some(PathBuf::from(
                    arguments.next().ok_or("missing --output path")?,
                ))
            }
            _ => {
                return Err(format!(
                    "unknown argument {argument}; use --corpus PATH [--output PATH]"
                ))
            }
        }
    }
    Ok((corpus.ok_or("missing --corpus PATH")?, output))
}
