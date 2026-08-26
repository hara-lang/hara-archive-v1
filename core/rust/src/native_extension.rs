#![cfg(not(target_arch = "wasm32"))]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::extension::{ExtensionManifest, WasmAbi};
use crate::kernel::{parse, Form};

use super::wasm_binding::{MemoryBindingPlan, WasmInterface, MEMORY_BINDING_SCHEMA};

const MAX_PROJECT_BYTES: u64 = 1024 * 1024;
const MAX_MODULE_BYTES: u64 = 64 * 1024 * 1024;

pub struct ExtensionPackage {
    pub root: PathBuf,
    pub descriptor: PathBuf,
    pub source: String,
    pub manifest: ExtensionManifest,
}

impl ExtensionPackage {
    #[cfg(test)]
    pub fn load(root: &Path) -> Result<Self, String> {
        let mut packages = packages_in_project(root)?;
        match packages.len() {
            1 => Ok(packages.remove(0)),
            0 => Err(format!(
                "extension/malformed: {} does not declare :project/extensions",
                root.display()
            )),
            count => Err(format!(
                "extension/ambiguous: {} declares {count} extension namespaces",
                root.display()
            )),
        }
    }

    pub fn discover(namespace: &str, roots: &[PathBuf]) -> Result<Option<Self>, String> {
        let mut candidates = Vec::new();
        for root in roots {
            for project in project_manifests(root)? {
                for package in packages_from_manifest(&project)? {
                    if package.manifest.namespace == namespace {
                        candidates.push(package);
                    }
                }
            }
        }
        candidates.sort_by(|left, right| left.descriptor.cmp(&right.descriptor));
        candidates.dedup_by(|left, right| left.descriptor == right.descriptor);
        match candidates.len() {
            0 => Ok(None),
            1 => Ok(candidates.pop()),
            _ => Err(format!(
                "extension/ambiguous: multiple projects export {namespace}: {:?}",
                candidates
                    .iter()
                    .map(|package| &package.descriptor)
                    .collect::<Vec<_>>()
            )),
        }
    }

    pub fn module_bytes(&self) -> Result<Vec<u8>, String> {
        let module =
            self.manifest.module.as_deref().ok_or_else(|| {
                format!("extension/module-unavailable: {}", self.manifest.namespace)
            })?;
        let path = self.resolve(module)?;
        let metadata = path
            .metadata()
            .map_err(|error| format!("extension/module-unavailable: {error}"))?;
        if metadata.len() > MAX_MODULE_BYTES {
            return Err(format!("extension/module-too-large: {}", path.display()));
        }
        fs::read(&path).map_err(|error| format!("extension/module-unavailable: {error}"))
    }

    pub fn memory_binding_plan(&self, module_bytes: &[u8]) -> Result<MemoryBindingPlan, String> {
        if self.manifest.abi != WasmAbi::MemoryV1 {
            return Err(format!(
                "extension/abi-invalid: {} is not a memory.v1 package",
                self.manifest.namespace
            ));
        }
        let interface_source = fs::read_to_string(self.resolve("interface.hal")?)
            .map_err(|error| format!("extension/asset-unavailable: {error}"))?;
        let interface = WasmInterface::parse(
            &interface_source,
            &format!("{}/interface.hal", self.manifest.namespace),
        )?;
        if interface.namespace != self.manifest.namespace
            || interface.module != self.manifest.module.as_deref().unwrap_or_default()
        {
            return Err(format!(
                "extension/manifest-mismatch: {} does not match interface.hal",
                self.manifest.namespace
            ));
        }
        let inspection = crate::wasm_binding::inspect_direct(module_bytes)?;
        let plan = interface.memory_plan()?;
        plan.verify(&inspection)
            .map_err(|error| format!("extension/binding-invalid: {error}"))?;

        let bindings_source = fs::read_to_string(self.resolve("bindings.edn")?)
            .map_err(|error| format!("extension/asset-unavailable: {error}"))?;
        let bindings = parse(&bindings_source)
            .map_err(|error| format!("extension/binding-invalid: {error}"))?;
        if bindings.to_string() != plan.canonical_source() {
            return Err(
                "extension/binding-drift: bindings.edn does not match interface.hal".into(),
            );
        }
        if field_string(&bindings, "schema")? != MEMORY_BINDING_SCHEMA
            || field_string(&bindings, "target")? != "memory.v1"
            || field_string(&bindings, "namespace")? != interface.namespace
            || field_string(&bindings, "module")? != interface.module
        {
            return Err("extension/binding-invalid: bindings.edn metadata mismatch".into());
        }

        let module_digest = digest(module_bytes);
        let interface_digest = digest(interface.canonical_source().as_bytes());
        let binding_digest = digest(bindings_source.as_bytes());
        let conformance_source = fs::read_to_string(self.resolve("conformance/bindings.edn")?)
            .map_err(|error| format!("extension/asset-unavailable: {error}"))?;
        let conformance = parse(&conformance_source)
            .map_err(|error| format!("extension/conformance-invalid: {error}"))?;
        verify_recorded_digests(
            &conformance,
            "memory.v1",
            &interface.namespace,
            &module_digest,
            &interface_digest,
            &binding_digest,
        )?;
        let product_source = fs::read_to_string(self.resolve("hara.build-product.edn")?)
            .map_err(|error| format!("extension/asset-unavailable: {error}"))?;
        let product = parse(&product_source)
            .map_err(|error| format!("extension/build-product-invalid: {error}"))?;
        if field_string(&product, "product/target")? != "memory.v1"
            || field_string(&product, "product/namespace")? != interface.namespace
            || field_string(&product, "product/binding-digest")? != binding_digest
            || field_string(field(&product, "product/inputs")?, "module-digest")? != module_digest
            || field_string(field(&product, "product/inputs")?, "interface-digest")?
                != interface_digest
        {
            return Err("extension/digest-mismatch: build product digest does not match".into());
        }
        Ok(plan)
    }

    pub fn declared_files(&self) -> Vec<String> {
        let mut paths = Vec::new();
        if let Some(module) = &self.manifest.module {
            paths.push(module.clone());
        }
        paths.extend(
            self.manifest
                .targets
                .values()
                .map(|target| target.provider.clone()),
        );
        paths.extend(self.manifest.assets.clone());
        paths.sort();
        paths.dedup();
        paths
    }

    fn validate_declared_files(&self) -> Result<(), String> {
        for relative in self.declared_files() {
            self.resolve(&relative)?;
        }
        Ok(())
    }

    pub fn resolve(&self, relative: &str) -> Result<PathBuf, String> {
        let root = self
            .root
            .canonicalize()
            .map_err(|error| format!("extension/asset-unavailable: {error}"))?;
        let declaration_root = self.manifest.root.as_deref().unwrap_or(".");
        let path = root
            .join(declaration_root)
            .join(relative)
            .canonicalize()
            .map_err(|error| {
                format!(
                    "extension/asset-unavailable: {}/{} ({error})",
                    self.manifest.namespace, relative
                )
            })?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(format!("extension/path-denied: {relative}"));
        }
        Ok(path)
    }
}

pub fn configured_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(current) = env::current_dir() {
        for directory in current.ancestors() {
            if directory.join("project.edn").is_file() {
                roots.push(directory.to_path_buf());
                roots.push(directory.join("extensions"));
                break;
            }
        }
    }
    if let Some(configured) = env::var_os("HARA_EXTENSION_PATH") {
        roots.extend(env::split_paths(&configured));
    }
    roots.extend(installed_package_roots());
    roots
}

/// Returns immutable installed package roots. Package loading still verifies
/// package.edn and every selected artifact before activation.
pub fn installed_package_roots() -> Vec<PathBuf> {
    let dist = env::var_os("HARA_DIST_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".hara/dist")))
        .unwrap_or_else(|| PathBuf::from(".hara/dist"));
    let roots = dist.join("roots/sha256");
    let Ok(entries) = fs::read_dir(&roots) else {
        return Vec::new();
    };
    let mut result = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    result.sort();
    result
}

pub fn package_exists(namespace: &str, roots: &[PathBuf]) -> bool {
    ExtensionPackage::discover(namespace, roots)
        .ok()
        .flatten()
        .is_some()
}

#[cfg(test)]
fn packages_in_project(root: &Path) -> Result<Vec<ExtensionPackage>, String> {
    let descriptor = if root.is_file() {
        root.to_path_buf()
    } else {
        root.join("project.edn")
    };
    packages_from_manifest(&descriptor)
}

fn packages_from_manifest(descriptor: &Path) -> Result<Vec<ExtensionPackage>, String> {
    let metadata = descriptor
        .metadata()
        .map_err(|error| format!("extension/asset-unavailable: {error}"))?;
    if metadata.len() > MAX_PROJECT_BYTES {
        return Err(format!(
            "extension/malformed {}: project manifest is too large",
            descriptor.display()
        ));
    }
    let project_source = fs::read_to_string(descriptor)
        .map_err(|error| format!("extension/malformed {}: {error}", descriptor.display()))?;
    let Form::Map(project) = parse(&project_source)
        .map_err(|error| format!("extension/malformed {}: {error}", descriptor.display()))?
    else {
        return Err("extension/malformed: project.edn must be a map".into());
    };
    let version = value(&project, "project/version")
        .ok_or("extension/malformed: project.edn is missing :project/version")?;
    let version = scalar(version, "project/version")?;
    let Some(Form::Map(extensions)) = value(&project, "project/extensions") else {
        return Ok(Vec::new());
    };
    let root = descriptor
        .parent()
        .ok_or("extension/root-invalid: project.edn has no parent")?
        .to_path_buf();
    extensions
        .iter()
        .map(|(namespace, declaration)| {
            let namespace = scalar(namespace, "extension namespace")?;
            let Form::Map(declaration) = declaration else {
                return Err(format!(
                    "extension/malformed {}: declaration for {namespace} must be a map",
                    descriptor.display()
                ));
            };
            let mut normalized = declaration.clone();
            normalized.push((
                Form::Keyword("namespace".into()),
                Form::String(namespace.clone()),
            ));
            normalized.push((
                Form::Keyword("version".into()),
                Form::String(version.clone()),
            ));
            let source = Form::Map(normalized).to_string();
            let package = ExtensionPackage {
                root: root.clone(),
                descriptor: descriptor.to_path_buf(),
                manifest: ExtensionManifest::parse(&source, &descriptor.display().to_string())?,
                source,
            };
            package.validate_declared_files()?;
            Ok(package)
        })
        .collect()
}

fn project_manifests(root: &Path) -> Result<Vec<PathBuf>, String> {
    let root = absolute(root)?;
    if root.is_file() {
        return Ok(
            (root.file_name().and_then(|name| name.to_str()) == Some("project.edn"))
                .then_some(root)
                .into_iter()
                .collect(),
        );
    }
    let mut pending = vec![root];
    let mut manifests = Vec::new();
    while let Some(directory) = pending.pop() {
        let manifest = directory.join("project.edn");
        if manifest.is_file() {
            manifests.push(manifest);
            continue;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        pending.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_dir()
                        && path.file_name().and_then(|name| name.to_str()) != Some("target")
                }),
        );
    }
    manifests.sort();
    Ok(manifests)
}

fn value<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
    })
}

fn scalar(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) | Form::Symbol(value) => Ok(value.clone()),
        _ => Err(format!(
            "extension/malformed: {label} must be a string or symbol"
        )),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn field<'a>(form: &'a Form, name: &str) -> Result<&'a Form, String> {
    let Form::Map(entries) = form else {
        return Err("extension/metadata-invalid: document must be a map".into());
    };
    entries
        .iter()
        .find_map(|(key, value)| {
            matches!(key, Form::Keyword(key) | Form::Symbol(key) if key == name).then_some(value)
        })
        .ok_or_else(|| format!("extension/metadata-invalid: missing :{name}"))
}

fn field_string(form: &Form, name: &str) -> Result<String, String> {
    match field(form, name)? {
        Form::String(value) | Form::Symbol(value) | Form::Keyword(value) => Ok(value.clone()),
        _ => Err(format!(
            "extension/metadata-invalid: :{name} must be a string, symbol, or keyword"
        )),
    }
}

fn verify_recorded_digests(
    form: &Form,
    target: &str,
    namespace: &str,
    module_digest: &str,
    interface_digest: &str,
    binding_digest: &str,
) -> Result<(), String> {
    if field_string(form, "target")? != target
        || field_string(form, "namespace")? != namespace
        || field_string(form, "module-digest")? != module_digest
        || field_string(form, "interface-digest")? != interface_digest
        || field_string(form, "binding-digest")? != binding_digest
    {
        return Err("extension/digest-mismatch: recorded package digest does not match".into());
    }
    Ok(())
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("extension/root-invalid: {error}"))?
            .join(path)
    };
    if path.exists() {
        path.canonicalize()
            .map_err(|error| format!("extension/root-invalid: {error}"))
    } else {
        Ok(path)
    }
}
