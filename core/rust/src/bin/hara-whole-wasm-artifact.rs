use std::{env, fs, path::PathBuf, process};

use hara_wasm::whole_wasm::{compile_artifact_from_hbc, NativeModule};

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-whole-wasm-artifact: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("compile") => {
            let source_path = required(&mut arguments, "source path")?;
            let output_path = required(&mut arguments, "output path")?;
            reject_extra(arguments)?;
            compile(&source_path, &output_path)
        }
        Some("run") => {
            let artifact_path = required(&mut arguments, "artifact path")?;
            reject_extra(arguments)?;
            execute(&artifact_path)
        }
        _ => Err(usage()),
    }
}

fn required(arguments: &mut impl Iterator<Item = String>, name: &str) -> Result<PathBuf, String> {
    arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {name}; {}", usage()))
}

fn reject_extra(mut arguments: impl Iterator<Item = String>) -> Result<(), String> {
    if let Some(argument) = arguments.next() {
        return Err(format!("unexpected argument {argument:?}; {}", usage()));
    }
    Ok(())
}

fn compile(source_path: &PathBuf, output_path: &PathBuf) -> Result<(), String> {
    let source = fs::read_to_string(source_path)
        .map_err(|error| format!("cannot read {}: {error}", source_path.display()))?;
    let program = hara_wasm::vm::compile_source(&source).map_err(|error| error.to_string())?;
    let hbc = hara_wasm::vm::encode_program(&program)?;
    let artifact = compile_artifact_from_hbc(&hbc)?;
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::write(output_path, &artifact)
        .map_err(|error| format!("cannot write {}: {error}", output_path.display()))?;
    println!(
        "wrote {} bytes to {}",
        artifact.len(),
        output_path.display()
    );
    Ok(())
}

fn execute(artifact_path: &PathBuf) -> Result<(), String> {
    let artifact = fs::read(artifact_path)
        .map_err(|error| format!("cannot read {}: {error}", artifact_path.display()))?;
    let mut module = NativeModule::load(&artifact)?;
    println!("{}", module.call_entry_i64()?);
    Ok(())
}

fn usage() -> String {
    "usage: hara-whole-wasm-artifact compile <source.hal> <artifact.hnw> | run <artifact.hnw>"
        .into()
}
