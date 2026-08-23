//! Trusted native-host loader for resolver-selected prebuilt Wasm package variants.
//!
//! This module deliberately consumes only verified package metadata and package-root bytes. It does
//! not compile source code, discover ambient host services, or grant capabilities not admitted by
//! the package resolver.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;

use crate::extension::{ExtensionManifest, WasmAbi, WasmExtension};
use crate::package_manifest::{
    PackageArtifactType, PackageManifest, PackageRuntimeRequirements, PackageSelection,
};
use crate::wasmtime_provider::WasmtimeExtensionProvider;

pub struct LoadedPackageWasm {
    pub identity: String,
    pub entry_point: String,
    pub extension: WasmExtension,
}

pub fn load_wasm_package(
    manifest: &PackageManifest,
    package_root: &Path,
    requirements: &PackageRuntimeRequirements,
    extension_manifest_source: &str,
) -> Result<LoadedPackageWasm, String> {
    let module = manifest
        .wasm_imports
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| {
            "package/missing-wasm-import: package declares no Wasm imports".to_owned()
        })?;
    load_wasm_import_package(
        manifest,
        package_root,
        &module,
        requirements,
        extension_manifest_source,
    )
}

pub fn load_wasm_import_package(
    manifest: &PackageManifest,
    package_root: &Path,
    module: &str,
    requirements: &PackageRuntimeRequirements,
    extension_manifest_source: &str,
) -> Result<LoadedPackageWasm, String> {
    manifest
        .verify_files_at(package_root)
        .map_err(|error| error.to_string())?;
    let selection = manifest
        .select_wasm_import(module, requirements)
        .map_err(|error| error.to_string())?;
    let PackageSelection::Variant(variant) = &selection else {
        return Err("package/missing-artifact: portable package has no Wasm artifact".into());
    };
    if variant.artifact.artifact_type != PackageArtifactType::Wasm {
        return Err(format!(
            "package/artifact-type-mismatch: expected :wasm, got :{}",
            variant.artifact.artifact_type.keyword()
        ));
    }
    if variant.artifact.abi != "core.v1" && variant.artifact.abi != "memory.v1" {
        return Err(format!(
            "package/abi-mismatch: direct Wasm loader does not support {}",
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
    let expected_abi = match variant.artifact.abi.as_str() {
        "core.v1" => WasmAbi::CoreV1,
        "memory.v1" => WasmAbi::MemoryV1,
        _ => unreachable!(),
    };
    if extension_manifest.abi != expected_abi {
        return Err(
            "package/abi-mismatch: extension manifest differs from selected variant".into(),
        );
    }
    if extension_manifest.provider != "wasm" {
        return Err(
            "package/provider-mismatch: direct Wasm artifact requires :provider :wasm".into(),
        );
    }
    if !variant.required_capabilities.iter().all(|capability| {
        extension_manifest
            .capabilities
            .iter()
            .any(|declared| declared == capability)
    }) {
        return Err(
            "package/manifest-mismatch: required capabilities differ from extension".into(),
        );
    }
    if !variant.host_calls.is_empty() || !extension_manifest.host_calls.is_empty() {
        return Err(
            "package/host-call-denied: direct Wasm packages cannot request host calls".into(),
        );
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
    if !extension_manifest.exports.iter().any(|(name, spec)| {
        name == &variant.artifact.entry_point || spec.raw_name(name) == variant.artifact.entry_point
    }) {
        return Err("package/entry-point-mismatch: selected entry point is not exported".into());
    }

    let provider = match expected_abi {
        WasmAbi::CoreV1 => WasmtimeExtensionProvider::compile(&bytes)?,
        WasmAbi::MemoryV1 => {
            return Err(
                "package/binding-plan-required: memory.v1 package loading requires verified bindings.edn"
                    .into(),
            )
        }
        WasmAbi::HtaV1 => unreachable!(),
    };
    let extension = WasmExtension::new(extension_manifest, provider)?;
    Ok(LoadedPackageWasm {
        identity: manifest.identity.clone(),
        entry_point: variant.artifact.entry_point.clone(),
        extension,
    })
}
