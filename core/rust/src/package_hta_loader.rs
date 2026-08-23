//! Trusted native-host loader for resolver-selected generated HTA Wasm packages.
//!
//! HTA artifacts are selected only through the package `:require` route. The
//! loader verifies the complete package tree and the selected artifact before
//! Wasmtime sees any bytes.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use crate::extension::{ExtensionManifest, WasmAbi, WasmExtension, Value};
use crate::package_manifest::{
    PackageArtifactType, PackageManifest, PackageRuntimeRequirements, PackageSelection,
};
use crate::wasmtime_provider::WasmtimeExtensionProvider;

pub struct LoadedPackageHta {
    pub identity: String,
    pub entry_point: String,
    pub extension: WasmExtension,
}

pub fn load_hta_package(
    manifest: &PackageManifest,
    package_root: &Path,
    requirements: &PackageRuntimeRequirements,
    extension_manifest_source: &str,
) -> Result<LoadedPackageHta, String> {
    let module = match manifest.wasm_imports.len() {
        0 => {
            return Err("package/missing-require-artifact: package declares no HTA artifacts".into())
        }
        1 => manifest
            .wasm_imports
            .keys()
            .next()
            .cloned()
            .expect("one HTA artifact"),
        _ => {
            return Err(
                "package/ambiguous-require-artifact: package declares multiple HTA artifacts"
                    .into(),
            )
        }
    };
    load_hta_require_package(
        manifest,
        package_root,
        &module,
        requirements,
        extension_manifest_source,
        None,
    )
}

pub fn load_hta_require_package(
    manifest: &PackageManifest,
    package_root: &Path,
    module: &str,
    requirements: &PackageRuntimeRequirements,
    extension_manifest_source: &str,
    host_handler: Option<
        Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>,
    >,
) -> Result<LoadedPackageHta, String> {
    manifest
        .verify_files_at(package_root)
        .map_err(|error| error.to_string())?;
    let selection = manifest
        .select_hta_require(module, requirements)
        .map_err(|error| error.to_string())?;
    let PackageSelection::Variant(variant) = &selection else {
        return Err("package/missing-artifact: portable package has no HTA artifact".into());
    };
    if variant.artifact.artifact_type != PackageArtifactType::Hta {
        return Err(format!(
            "package/artifact-type-mismatch: expected :hta, got :{}",
            variant.artifact.artifact_type.keyword()
        ));
    }
    if variant.artifact.abi != "hta.v1" {
        return Err(format!(
            "package/abi-mismatch: HTA loader does not support {}",
            variant.artifact.abi
        ));
    }

    let artifact_path = package_root.join(&variant.artifact.path);
    let bytes = fs::read(&artifact_path).map_err(|error| {
        format!(
            "package/missing-artifact: cannot read {}: {error}",
            variant.artifact.path.display()
        )
    })?;
    manifest
        .verify_artifact_bytes(&selection, &bytes)
        .map_err(|error| error.to_string())?;

    let extension_manifest = ExtensionManifest::parse(extension_manifest_source, "package")?;
    if extension_manifest.identity.as_deref() != Some(manifest.identity.as_str()) {
        return Err("package/identity-mismatch: extension identity differs from package".into());
    }
    if extension_manifest.provider != "wasm" {
        return Err("package/provider-mismatch: HTA artifact requires :provider :wasm".into());
    }
    if extension_manifest.abi != WasmAbi::HtaV1 {
        return Err("package/abi-mismatch: extension manifest differs from selected variant".into());
    }
    let required_capabilities = extension_manifest
        .capabilities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if required_capabilities != variant.required_capabilities {
        return Err("package/manifest-mismatch: required capabilities differ from extension".into());
    }
    let declared_host_calls = extension_manifest
        .host_calls
        .iter()
        .flat_map(|(service, methods)| {
            methods
                .iter()
                .map(move |method| format!("{service}/{method}"))
        })
        .collect::<BTreeSet<_>>();
    if declared_host_calls != variant.host_calls {
        return Err("package/manifest-mismatch: declared host calls differ from extension".into());
    }
    if !variant.exports.iter().all(|export| {
        extension_manifest
            .exports
            .iter()
            .any(|(declared, _)| declared == export)
    }) {
        return Err(
            "package/manifest-mismatch: selected exports are not declared by extension".into(),
        );
    }

    let provider =
        WasmtimeExtensionProvider::compile_hta_with_host_handler(&bytes, host_handler)?;
    let extension = WasmExtension::new(extension_manifest, provider)?;
    Ok(LoadedPackageHta {
        identity: manifest.identity.clone(),
        entry_point: variant.artifact.entry_point.clone(),
        extension,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    const EXTENSION: &str = r#"
      {:namespace "fixture.package"
       :identity "example/provider"
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
            r#"
            {{:harp/format "0.0.0-alpha"
             :package {{:identity "example/provider" :version "1.0.0"}}
             :files {{"artifacts/provider.wasm" {{:sha256 "{digest}" :size {}}}}}
             :wasm-imports {{:fixture.package {{:variant/artifact
               {{:artifact/type :{artifact_type}
                :artifact/path "artifacts/provider.wasm"
                :artifact/sha256 "{digest}"
                :artifact/target "wasm32-wasi-preview1"
                :artifact/abi "hta.v1"
                :artifact/entry-point "hta_start"}}
               :variant/required-capabilities {{}}
               :variant/host-calls {{}}
               :variant/exports #{{"eval"}}}}}}
            "#,
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
        let expected = b"trusted";
        let manifest = manifest(expected, "hta");
        let root = root("tampered", b"tampered");
        let result = load_hta_require_package(
            &manifest,
            &root,
            "fixture.package",
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
            "fixture.package",
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
}
