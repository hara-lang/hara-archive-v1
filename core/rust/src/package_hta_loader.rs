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

use crate::extension::{ExtensionManifest, Value, WasmAbi, WasmExtension};
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
            return Err(
                "package/missing-require-artifact: package declares no HTA artifacts".into(),
            )
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
    host_handler: Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>,
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
        return Err(
            "package/abi-mismatch: extension manifest differs from selected variant".into(),
        );
    }
    let required_capabilities = extension_manifest
        .capabilities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if required_capabilities != variant.required_capabilities {
        return Err(
            "package/manifest-mismatch: required capabilities differ from extension".into(),
        );
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
    let artifact_path = variant.artifact.path.to_string_lossy();
    let library_path = extension_manifest
        .assets
        .iter()
        .find(|path| path.ends_with(".wasm") && path.as_str() != artifact_path)
        .cloned();
    let provider = if let Some(library_path) = library_path {
        let library_bytes = fs::read(package_root.join(&library_path)).map_err(|error| {
            format!(
                "package/missing-library: cannot read {}: {error}",
                library_path
            )
        })?;
        WasmtimeExtensionProvider::compile_hta_with_library(&bytes, &library_bytes, host_handler)?
    } else {
        WasmtimeExtensionProvider::compile_hta_with_host_handler(&bytes, host_handler)?
    };
    let extension = WasmExtension::new(extension_manifest, provider)?;
    Ok(LoadedPackageHta {
        identity: manifest.identity.clone(),
        entry_point: variant.artifact.entry_point.clone(),
        extension,
    })
}
