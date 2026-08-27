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
mod catalog;
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
pub struct PackageCatalogDescriptor {
    pub format: String,
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageBytecode {
    pub format: String,
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageCatalogAdmission {
    pub format: String,
    pub report: String,
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
    pub name: Option<String>,
    pub version: Version,
    pub provenance: Option<PackageProvenance>,
    pub files: BTreeMap<PathBuf, PackageFile>,
    pub bytecode: Option<PackageBytecode>,
    pub schema_catalog: Option<PackageCatalogDescriptor>,
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
        Ok(VerifiedPackageSelection {
            manifest,
            selection,
        })
    }

    pub fn select_hta_require_archive(
        path: &Path,
        module: &str,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<VerifiedPackageSelection, PackageManifestError> {
        let manifest = Self::read_archive(path)?;
        let selection = manifest.select_hta_require(module, requirements)?;
        Ok(VerifiedPackageSelection {
            manifest,
            selection,
        })
    }

    pub fn parse(source: &str) -> Result<Self, PackageManifestError> {
        parse::parse_manifest(source)
    }

    pub fn canonical_edn(&self) -> &str {
        &self.canonical_edn
    }

    /// Returns the diagnostic emitted by non-JVM loaders when a package also
    /// carries host-native flavor artifacts. Rust intentionally leaves those
    /// artifacts untouched; only portable and Wasm routes are loaded here.
    pub fn unsupported_host_flavors_warning(&self) -> Option<String> {
        if self.flavors.is_empty() {
            return None;
        }
        let flavors = self
            .flavors
            .keys()
            .map(|flavor| format!(":{flavor}"))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "package/host-flavors-ignored: {} {} ({flavors}) are unavailable on the Rust/Wasm runtime",
            self.identity, self.version
        ))
    }

    pub fn admit_catalog_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<PackageCatalogAdmission, PackageManifestError> {
        let descriptor = self.schema_catalog.as_ref().ok_or_else(|| {
            PackageManifestError::new(
                "package/catalog-missing",
                "package does not declare :schema/catalog",
            )
        })?;
        self.verify_file_bytes(&descriptor.path, bytes)?;
        let source = std::str::from_utf8(bytes).map_err(|error| {
            PackageManifestError::new(
                "package/catalog-invalid",
                format!(
                    "catalog {} is not UTF-8 JSON: {error}",
                    descriptor.path.display()
                ),
            )
        })?;
        catalog::admit(&descriptor.format, source)
    }

    pub fn admit_catalog_at(
        &self,
        root: &Path,
    ) -> Result<Option<PackageCatalogAdmission>, PackageManifestError> {
        let Some(descriptor) = &self.schema_catalog else {
            return Ok(None);
        };
        let bytes = fs::read(root.join(&descriptor.path)).map_err(|error| {
            PackageManifestError::new(
                "package/catalog-missing",
                format!("cannot read catalog {}: {error}", descriptor.path.display()),
            )
        })?;
        self.admit_catalog_bytes(&bytes).map(Some)
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
            if self.wasm_imports.is_empty() {
                return Ok(PackageSelection::Portable);
            }
            return Err(PackageManifestError::new(
                "package/missing-flavor",
                format!(
                    "{} {} has no :{} host flavor",
                    self.identity, self.version, flavor
                ),
            ));
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
        if variant.artifact.artifact_type != PackageArtifactType::Jar {
            return Err(PackageManifestError::new(
                "package/artifact-type-mismatch",
                format!(
                    ":{} flavor must select :jar, got :{}",
                    flavor,
                    variant.artifact.artifact_type.keyword()
                ),
            ));
        }
        self.preflight_variant(&format!(":{} flavor", flavor), variant, requirements)?;
        Ok(PackageSelection::Variant(variant.clone()))
    }

    pub fn select_hta_require(
        &self,
        module: &str,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<PackageSelection, PackageManifestError> {
        if self.wasm_imports.is_empty() && self.flavors.is_empty() {
            return Ok(PackageSelection::Portable);
        }
        let variant = self.wasm_imports.get(module).ok_or_else(|| {
            PackageManifestError::new(
                "package/missing-require-artifact",
                format!(
                    "{} {} has no HTA artifact for :require {module}",
                    self.identity, self.version
                ),
            )
        })?;
        if variant.artifact.artifact_type != PackageArtifactType::Hta {
            return Err(PackageManifestError::new(
                "package/artifact-type-mismatch",
                format!(
                    ":require {module} must select :hta, got :{}",
                    variant.artifact.artifact_type.keyword()
                ),
            ));
        }
        self.preflight_variant(&format!(":require {module}"), variant, requirements)?;
        Ok(PackageSelection::Variant(variant.clone()))
    }

    fn preflight_variant(
        &self,
        route: &str,
        variant: &PackageVariant,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<(), PackageManifestError> {
        if !requirements
            .supported_targets
            .contains(&variant.artifact.target)
        {
            return Err(PackageManifestError::new(
                "package/target-mismatch",
                format!(
                    "{route} artifact target {} is not supported",
                    variant.artifact.target
                ),
            ));
        }
        if !requirements.supported_abis.contains(&variant.artifact.abi) {
            return Err(PackageManifestError::new(
                "package/abi-mismatch",
                format!(
                    "{route} artifact ABI {} is not supported",
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
        Ok(())
    }

    pub fn select_wasm_import(
        &self,
        module: &str,
        requirements: &PackageRuntimeRequirements,
    ) -> Result<PackageSelection, PackageManifestError> {
        let variant = self.wasm_imports.get(module).ok_or_else(|| {
            PackageManifestError::new(
                "package/missing-wasm-import",
                format!(
                    "{} {} has no Wasm import {module}",
                    self.identity, self.version
                ),
            )
        })?;
        if variant.artifact.artifact_type != PackageArtifactType::Wasm {
            return Err(PackageManifestError::new(
                "package/artifact-type-mismatch",
                format!(
                    ":import {module} must select :wasm, got :{}",
                    variant.artifact.artifact_type.keyword()
                ),
            ));
        }
        self.preflight_variant(&format!(":import {module}"), variant, requirements)?;
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
