use std::{env, fs, path::PathBuf, process};

fn main() {
    let result = std::thread::Builder::new()
        .name("foundation-bytecode-artifact".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .expect("spawn foundation bytecode artifact worker")
        .join()
        .expect("foundation bytecode artifact worker must not panic");
    if let Err(error) = result {
        eprintln!("hara-foundation-artifact: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = env::args().nth(1).unwrap_or_else(|| "check".into());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/std.foundation.hbx");
    let generated = hara_wasm::vm::compile_embedded_foundation_bootstrap_bundle()?;
    match command.as_str() {
        "generate" => {
            fs::create_dir_all(path.parent().expect("asset parent"))
                .map_err(|error| error.to_string())?;
            fs::write(&path, &generated).map_err(|error| error.to_string())?;
            println!("wrote {} bytes to {}", generated.len(), path.display());
            Ok(())
        }
        "check" => {
            let tracked = fs::read(&path).map_err(|error| {
                format!("cannot read {}: {error}; run with generate", path.display())
            })?;
            if tracked != generated {
                return Err(format!(
                    "{} is stale; run `cargo run --manifest-path core/rust/Cargo.toml --features bytecode-vm --bin hara-foundation-artifact -- generate`",
                    path.display()
                ));
            }
            println!("{} is current ({} bytes)", path.display(), tracked.len());
            Ok(())
        }
        _ => Err("usage: hara-foundation-artifact [generate|check]".into()),
    }
}
