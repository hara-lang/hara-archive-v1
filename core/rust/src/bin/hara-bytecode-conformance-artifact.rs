use hara_wasm::{kernel::Form, spec_registry, Runtime};
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf, process};

const MAGIC: &[u8; 4] = b"HCC0";

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-bytecode-conformance-artifact: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = env::args().nth(1).unwrap_or_else(|| "check".into());
    let corpus_path =
        spec_registry::resolve("01-lang/010-bytecode/draft/conformance/bytecode-vm.edn")
            .filter(|candidate| candidate.is_file())
            .ok_or_else(|| "cannot locate bytecode-vm conformance corpus")?;
    let asset_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/bytecode-conformance.hcc");
    let corpus = fs::read_to_string(&corpus_path).map_err(|error| error.to_string())?;
    let generated = compile_corpus(&corpus)?;
    match command.as_str() {
        "generate" => {
            fs::create_dir_all(asset_path.parent().unwrap()).map_err(|error| error.to_string())?;
            fs::write(&asset_path, &generated).map_err(|error| error.to_string())?;
            println!(
                "wrote {} bytes to {}",
                generated.len(),
                asset_path.display()
            );
            Ok(())
        }
        "check" => {
            let tracked = fs::read(&asset_path).map_err(|error| error.to_string())?;
            if tracked != generated {
                return Err(format!(
                    "{} is stale; run with generate",
                    asset_path.display()
                ));
            }
            println!(
                "{} is current ({} bytes)",
                asset_path.display(),
                tracked.len()
            );
            Ok(())
        }
        _ => Err("usage: hara-bytecode-conformance-artifact [generate|check]".into()),
    }
}

fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find_map(|(candidate, value)| match candidate {
            Form::Keyword(name) if name == key => Some(value),
            _ => None,
        })
}

fn compile_corpus(source: &str) -> Result<Vec<u8>, String> {
    let Form::Map(manifest) = hara_wasm::kernel::parse_forms(source)?.remove(0) else {
        return Err("bytecode conformance corpus must be a map".into());
    };
    let Some(Form::Vector(cases)) = entry(&manifest, "cases") else {
        return Err("bytecode conformance corpus must contain :cases".into());
    };
    let mut compiled = Vec::new();
    let runtime = Runtime::new();
    for case in cases {
        let Form::Map(case) = case else { continue };
        let (Some(Form::Keyword(id)), Some(Form::String(source)), Some(Form::Map(expect))) = (
            entry(case, "id"),
            entry(case, "source"),
            entry(case, "expect"),
        ) else {
            continue;
        };
        let Some(Form::String(display)) = entry(expect, "display") else {
            continue;
        };
        let artifact = runtime
            .compile_bytecode_artifact(source)
            .map_err(|error| format!(":{id} failed to compile: {error}"))?;
        compiled.push((id.as_str(), display.as_str(), artifact));
    }
    let mut payload = Vec::new();
    put_u32(&mut payload, compiled.len())?;
    for (id, expected, artifact) in compiled {
        put_bytes(&mut payload, id.as_bytes())?;
        put_bytes(&mut payload, expected.as_bytes())?;
        put_bytes(&mut payload, &artifact)?;
    }
    let mut output = MAGIC.to_vec();
    output.extend_from_slice(&Sha256::digest(&payload));
    output.extend_from_slice(&payload);
    Ok(output)
}

fn put_u32(output: &mut Vec<u8>, value: usize) -> Result<(), String> {
    output.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| "conformance artifact exceeds u32 limits")?
            .to_le_bytes(),
    );
    Ok(())
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), String> {
    put_u32(output, value.len())?;
    output.extend_from_slice(value);
    Ok(())
}
