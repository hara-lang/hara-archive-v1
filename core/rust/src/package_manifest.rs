//! Data-only validation and exact package-artifact resolution for generated
//! `package.edn` manifests.
//!
//! This module deliberately stops before class loading, Wasm instantiation, or
//! provider registration. It turns untrusted archive metadata into a verified,
//! deterministic selection that a host loader can consume. Wasm modules are
//! imports shared by all hosts; host artifacts live under named flavors.

use crate::kernel::Form;
use semver::Version;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

mod archive;
mod parse;
#[cfg(test)]
mod tests;

const PACKAGE_FORMAT: &str = "0.0.0-alpha";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackageRuntime {
    Jvm,
}

impl PackageRuntime {
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Jvm => "jvm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageArtifactType {
    Jar,
    Wasm,
    Hta,
}

impl PackageArtifactType {
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Jar => "jar",
            Self::Wasm => "wasm",
            Self::Hta => "hta",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifestError {
    pub code: &'static str,
    pub detail: String,
}

impl PackageManifestError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for PackageManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for PackageManifestError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageProvenance {
    pub repository: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFile {
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageArtifact {
    pub artifact_type: PackageArtifactType,
    pub path: PathBuf,
    pub sha256: String,
    pub target: String,
    pub abi: String,
    pub entry_point: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLifecycle {
    pub load_idempotent: bool,
    pub close_idempotent: bool,
    pub session_isolation: bool,
    pub asynchronous: bool,
    pub cancellation: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageVariant {
    pub artifact: PackageArtifact,
    pub required_capabilities: BTreeSet<String>,
    pub host_calls: BTreeSet<String>,
    pub exports: BTreeSet<String>,
    pub dependencies: Option<Form>,
    pub lifecycle: Option<PackageLifecycle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageRuntimeRequirements {
    pub supported_targets: BTreeSet<String>,
    pub supported_abis: BTreeSet<String>,
    pub available_capabilities: BTreeSet<String>,
    pub allowed_host_calls: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PackageSelection {
    Portable,
    Variant(PackageVariant),
}

/// A package archive whose complete declared file set has been digest-checked
/// before exact-runtime preflight.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedPackageSelection {
    pub manifest: PackageManifest,
    pub selection: PackageSelection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageManifest {
    pub format: String,
    pub identity: String,
    pub version: Version,
    pub provenance: Option<PackageProvenance>,
    pub files: BTreeMap<PathBuf, PackageFile>,
    pub wasm_imports: BTreeMap<String, PackageVariant>,
    pub flavors: BTreeMap<String, PackageVariant>,
    canonical_edn: String,
}

impl PackageManifest {
    pub fn read(path: &Path) -> Result<Self, PackageManifestError> {
        let source = fs::read_to_string(path).map_err(|error| {
            PackageManifestError::new(
                "package/invalid-manifest",
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
        Self::parse(&source)
    }

    /// Opens a `.harp`, parses its data-only `package.edn`, rejects unsafe,
    /// duplicate, or undeclared entries, and verifies every declared file
    /// digest before returning the manifest.
    pub fn read_archive(path: &Path) -> Result<Self, PackageManifestError> {
        archive::read_archive(path)
    }

    /// Verifies the complete archive and then resolves only the requested
    /// runtime. A loader must consume the installed content-addressed root or
    /// reverify any artifact bytes it reads after this preflight.
    pub fn select_archive(
        path: &Path,
        runtime: PackageRuntime,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<VerifiedPackageSelection, PackageManifestError> {
        let manifest = Self::read_archive(path)?;
        let selection = manifest.select_variant(runtime, requirements)?;
        Ok(VerifiedPackageSelection {
            manifest,
            selection,
        })
    }

    pub fn select_wasm_import_archive(
        path: &Path,
        module: &str,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<VerifiedPackageSelection, PackageManifestError> {
        let manifest = Self::read_archive(path)?;
        let selection = manifest.select_wasm_import(module, requirements)?;
        Ok(VerifiedPackageSelection { manifest, selection })
    }

    pub fn parse(source: &str) -> Result<Self, PackageManifestError> {
        parse::parse_manifest(source)
    }

    pub fn canonical_edn(&self) -> &str {
        &self.canonical_edn
    }

    pub fn select_variant(
        &self,
        runtime: PackageRuntime,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<PackageSelection, PackageManifestError> {
        let flavor = match runtime {
            PackageRuntime::Jvm => "jvm",
        };
        self.select_flavor(flavor, requirements)
    }

    pub fn select_flavor(
        &self,
        flavor: &str,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<PackageSelection, PackageManifestError> {
        if self.flavors.is_empty() {
            return Ok(PackageSelection::Portable);
        }
        let variant = self.flavors.get(flavor).ok_or_else(|| {
            PackageManifestError::new(
                "package/missing-flavor",
                format!(
                    "{} {} has no :{} host flavor",
                    self.identity, self.version, flavor
                ),
            )
        })?;
        if !requirements
            .supported_targets
            .contains(&variant.artifact.target)
        {
            return Err(PackageManifestError::new(
                "package/target-mismatch",
                format!(
                    ":{} artifact target {} is not supported",
                    flavor,
                    variant.artifact.target
                ),
            ));
        }
        if !requirements.supported_abis.contains(&variant.artifact.abi) {
            return Err(PackageManifestError::new(
                "package/abi-mismatch",
                format!(
                    ":{} artifact ABI {} is not supported",
                    flavor,
                    variant.artifact.abi
                ),
            ));
        }

        let missing_capabilities = difference(
            &variant.required_capabilities,
            &requirements.available_capabilities,
        );
        if !missing_capabilities.is_empty() {
            return Err(PackageManifestError::new(
                "package/capability-denied",
                format!("missing capabilities: {}", missing_capabilities.join(", ")),
            ));
        }
        let denied_host_calls = difference(&variant.host_calls, &requirements.allowed_host_calls);
        if !denied_host_calls.is_empty() {
            return Err(PackageManifestError::new(
                "package/host-call-denied",
                format!("denied host calls: {}", denied_host_calls.join(", ")),
            ));
        }
        Ok(PackageSelection::Variant(variant.clone()))
    }

    pub fn select_wasm_import(
        &self,
        module: &str,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<PackageSelection, PackageManifestError> {
        let variant = self.wasm_imports.get(module).ok_or_else(|| {
            PackageManifestError::new(
                "package/missing-wasm-import",
                format!("{} {} has no Wasm import {module}", self.identity, self.version),
            )
        })?;
        if !requirements.supported_targets.contains(&variant.artifact.target) {
            return Err(PackageManifestError::new(
                "package/target-mismatch",
                format!("Wasm import {module} target {} is not supported", variant.artifact.target),
            ));
        }
        if !requirements.supported_abis.contains(&variant.artifact.abi) {
            return Err(PackageManifestError::new(
                "package/abi-mismatch",
                format!("Wasm import {module} ABI {} is not supported", variant.artifact.abi),
            ));
        }
        let missing_capabilities =
            difference(&variant.required_capabilities, &requirements.available_capabilities);
        if !missing_capabilities.is_empty() {
            return Err(PackageManifestError::new(
                "package/capability-denied",
                format!("missing capabilities: {}", missing_capabilities.join(", ")),
            ));
        }
        let denied_host_calls = difference(&variant.host_calls, &requirements.allowed_host_calls);
        if !denied_host_calls.is_empty() {
            return Err(PackageManifestError::new(
                "package/host-call-denied",
                format!("denied host calls: {}", denied_host_calls.join(", ")),
            ));
        }
        Ok(PackageSelection::Variant(variant.clone()))
    }

    pub fn verify_artifact_bytes(
        &self,
        selection: &PackageSelection,
        bytes: &[u8],
    ) -> Result<(), PackageManifestError> {
        let PackageSelection::Variant(variant) = selection else {
            return Err(PackageManifestError::new(
                "package/missing-artifact",
                "portable package selection has no runtime artifact",
            ));
        };
        self.verify_file_bytes(&variant.artifact.path, bytes)
    }

    /// Verifies one declared archive-relative file without requiring the caller
    /// to retain the full payload in memory.
    pub fn verify_file_reader<R: Read>(
        &self,
        relative: &Path,
        reader: &mut R,
    ) -> Result<(), PackageManifestError> {
        let expected = self.files.get(relative).ok_or_else(|| {
            PackageManifestError::new(
                "package/missing-artifact",
                format!("file is not declared in :files: {}", relative.display()),
            )
        })?;
        verify_reader(relative, expected, reader)
    }

    pub fn verify_file_bytes(
        &self,
        relative: &Path,
        bytes: &[u8],
    ) -> Result<(), PackageManifestError> {
        self.verify_file_reader(relative, &mut std::io::Cursor::new(bytes))
    }

    pub fn verify_files_at(&self, root: &Path) -> Result<(), PackageManifestError> {
        for (relative, expected) in &self.files {
            let path = root.join(relative);
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                PackageManifestError::new(
                    "package/missing-artifact",
                    format!("cannot inspect {}: {error}", relative.display()),
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(PackageManifestError::new(
                    "package/missing-artifact",
                    format!(
                        "declared package file is not a regular file: {}",
                        relative.display()
                    ),
                ));
            }
            if metadata.len() != expected.size {
                return Err(PackageManifestError::new(
                    "package/size-mismatch",
                    format!(
                        "{} has {} bytes, expected {}",
                        relative.display(),
                        metadata.len(),
                        expected.size
                    ),
                ));
            }
            let mut file = fs::File::open(&path).map_err(|error| {
                PackageManifestError::new(
                    "package/missing-artifact",
                    format!("cannot read {}: {error}", relative.display()),
                )
            })?;
            verify_reader(relative, expected, &mut file)?;
        }
        Ok(())
    }

    /// Verifies an extracted package root and performs exact-runtime preflight.
    /// This is the handoff used by runtime loaders after installation.
    pub fn verify_selection_at(
        &self,
        root: &Path,
        runtime: PackageRuntime,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<PackageSelection, PackageManifestError> {
        self.verify_files_at(root)?;
        self.select_variant(runtime, requirements)
    }
}

fn verify_reader<R: Read>(
    relative: &Path,
    expected: &PackageFile,
    reader: &mut R,
) -> Result<(), PackageManifestError> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            PackageManifestError::new(
                "package/missing-artifact",
                format!("cannot read {}: {error}", relative.display()),
            )
        })?;
        if read == 0 {
            break;
        }
        size = size.checked_add(read as u64).ok_or_else(|| {
            PackageManifestError::new(
                "package/size-mismatch",
                format!("{} is too large to verify", relative.display()),
            )
        })?;
        hasher.update(&buffer[..read]);
    }
    if size != expected.size {
        return Err(PackageManifestError::new(
            "package/size-mismatch",
            format!(
                "{} has {} bytes, expected {}",
                relative.display(),
                size,
                expected.size
            ),
        ));
    }
    let actual = digest_string(&hasher.finalize());
    if actual != expected.sha256 {
        return Err(PackageManifestError::new(
            "package/digest-mismatch",
            format!(
                "{} has digest {}, expected {}",
                relative.display(),
                actual,
                expected.sha256
            ),
        ));
    }
    Ok(())
}

fn digest_string(bytes: &[u8]) -> String {
    let hexadecimal = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hexadecimal}")
}

fn difference(required: &BTreeSet<String>, available: &BTreeSet<String>) -> Vec<String> {
    required.difference(available).cloned().collect()
}
