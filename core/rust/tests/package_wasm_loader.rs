use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use hara_wasm::extension::Value;
use hara_wasm::package_manifest::{PackageManifest, PackageRuntimeRequirements};
use hara_wasm::package_wasm_loader::load_wasm_package;

const ADD: &[u8] =
    b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";

const PACKAGE_MANIFEST: &str = r#"
{:harp/format "0.0.0-alpha"
 :package {:identity "example/provider"
           :version "1.0.0"
           :provenance {:repository "https://github.com/example/provider"
                        :commit "0123456789abcdef0123456789abcdef01234567"}}
 :files {"artifacts/provider.wasm" {:sha256 "sha256:cf96c3351ea2afd66dd2cee4480ea44fd2e76f8009ca1df96edb9dc149749edc"
                                    :size 41}}
 :wasm-imports {:provider {:variant/artifact {:artifact/type :wasm
                                       :artifact/path "artifacts/provider.wasm"
                                       :artifact/sha256 "sha256:cf96c3351ea2afd66dd2cee4480ea44fd2e76f8009ca1df96edb9dc149749edc"
                                       :artifact/target "wasm32-wasi-preview1"
                                       :artifact/abi "core.v1"
                                       :artifact/entry-point "add"}
                    :variant/required-capabilities #{}
                    :variant/exports #{"add"}}}}
"#;

const EXTENSION_MANIFEST: &str = r#"
{:namespace "fixture.package"
 :identity "example/provider"
 :version "1.0.0"
 :provider :wasm
 :module "provider.wasm"
 :abi :core.v1
 :exports {"add" {:args [:i64 :i64] :returns :i64}}
 :capabilities []}
"#;

fn requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        supported_targets: BTreeSet::from(["wasm32-wasi-preview1".to_owned()]),
        supported_abis: BTreeSet::from(["core.v1".to_owned()]),
        ..PackageRuntimeRequirements::default()
    }
}

fn package_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "hara-package-wasm-loader-{name}-{}",
        std::process::id()
    ))
}

fn write_artifact(name: &str, bytes: &[u8]) -> (PackageManifest, PathBuf) {
    let root = package_root(name);
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("artifacts")).unwrap();
    fs::write(root.join("artifacts/provider.wasm"), bytes).unwrap();
    (PackageManifest::parse(PACKAGE_MANIFEST).unwrap(), root)
}

#[test]
fn loads_verified_core_wasm_package_artifact() {
    let (manifest, root) = write_artifact("success", ADD);
    let mut loaded =
        load_wasm_package(&manifest, &root, &requirements(), EXTENSION_MANIFEST).unwrap();
    let bindings = loaded.extension.require().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0]
            .invoke(&[Value::Number(19), Value::Number(23)])
            .unwrap(),
        Value::Number(42)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_tampered_wasm_package_artifact_before_loading() {
    let mut tampered = ADD.to_vec();
    tampered[10] ^= 1;
    let (manifest, root) = write_artifact("tampered", &tampered);
    let error = match load_wasm_package(&manifest, &root, &requirements(), EXTENSION_MANIFEST) {
        Ok(_) => panic!("tampered package artifact was loaded"),
        Err(error) => error,
    };
    assert!(error.starts_with("package/digest-mismatch:"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_package_extension_identity_mismatch() {
    let (manifest, root) = write_artifact("identity", ADD);
    let extension_manifest = EXTENSION_MANIFEST.replace("example/provider", "other/provider");
    let error = match load_wasm_package(&manifest, &root, &requirements(), &extension_manifest) {
        Ok(_) => panic!("mismatched extension identity was loaded"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "package/identity-mismatch: extension identity differs from package"
    );
    fs::remove_dir_all(root).unwrap();
}
