#![cfg(not(target_arch = "wasm32"))]

use hara_wasm::package_hta_loader::load_hta_require_package;
use hara_wasm::package_manifest::{PackageManifest, PackageRuntimeRequirements};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const EXTENSION: &str = r#"
{:namespace "fixture.package"
 :identity "hara:example/provider"
 :version "1.0.0"
 :provider :wasm
 :module "provider.wasm"
 :abi :hta.v1
 :exports {"eval" {:args [:value] :returns :value :async true}}
 :capabilities []}
"#;

fn requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        supported_targets: BTreeSet::from(["wasm32-wasi-preview1".to_owned()]),
        supported_abis: BTreeSet::from(["hta.v1".to_owned()]),
        ..PackageRuntimeRequirements::default()
    }
}

fn manifest(bytes: &[u8], artifact_type: &str) -> PackageManifest {
    let digest = format!("sha256:{:x}", Sha256::digest(bytes));
    PackageManifest::parse(&format!(
        r#"{{:harp/format "0.0.0-alpha"
 :package {{:identity "hara:example/provider"
           :version "1.0.0"
           :provenance {{:repository "https://github.com/example/provider"
                         :commit "0123456789abcdef0123456789abcdef01234567"}}}}
 :files {{"artifacts/provider.wasm" {{:sha256 "{digest}" :size {}}}}}
 :wasm-imports {{:provider {{:variant/artifact
   {{:artifact/type :{artifact_type}
    :artifact/path "artifacts/provider.wasm"
    :artifact/sha256 "{digest}"
    :artifact/target "wasm32-wasi-preview1"
    :artifact/abi "hta.v1"
    :artifact/entry-point "hta_start"}}
   :variant/required-capabilities #{{}}
   :variant/host-calls #{{}}
   :variant/exports #{{"eval"}}}}}}}}"#,
        bytes.len()
    ))
    .unwrap()
}

fn root(name: &str, bytes: &[u8]) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hara-package-hta-loader-{name}-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("artifacts")).unwrap();
    fs::write(root.join("artifacts/provider.wasm"), bytes).unwrap();
    root
}

#[test]
fn rejects_tampered_hta_artifact_before_wasmtime() {
    let manifest = manifest(b"trusted", "hta");
    let root = root("tampered", b"changed");
    let result = load_hta_require_package(
        &manifest,
        &root,
        "provider",
        &requirements(),
        EXTENSION,
        None,
    );
    let error = match result {
        Ok(_) => panic!("tampered artifact was loaded"),
        Err(error) => error,
    };
    assert!(error.starts_with("package/digest-mismatch:"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_cross_route_wasm_artifacts() {
    let bytes = b"not wasm";
    let manifest = manifest(bytes, "wasm");
    let root = root("route", bytes);
    let result = load_hta_require_package(
        &manifest,
        &root,
        "provider",
        &requirements(),
        EXTENSION,
        None,
    );
    let error = match result {
        Ok(_) => panic!("cross-route artifact was loaded"),
        Err(error) => error,
    };
    assert!(error.starts_with("package/artifact-type-mismatch:"));
    fs::remove_dir_all(root).unwrap();
}
