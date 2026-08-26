use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=HARA_HTA_PROVIDER_SOURCE");

    if env::var_os("CARGO_FEATURE_RICH_HTA").is_none() {
        return;
    }

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .expect("Cargo must provide CARGO_MANIFEST_DIR to rich-hta builds"),
    );
    let source = env::var_os("HARA_HTA_PROVIDER_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("test-fixtures/rich/provider.hal"));
    let source = source.canonicalize().unwrap_or_else(|error| {
        panic!(
            "cannot resolve HARA_HTA_PROVIDER_SOURCE {}: {error}",
            source.display()
        )
    });
    if !source.is_file() {
        panic!(
            "HARA_HTA_PROVIDER_SOURCE is not a file: {}",
            source.display()
        );
    }
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rustc-env=HARA_HTA_PROVIDER_SOURCE={}", source.display());
}
