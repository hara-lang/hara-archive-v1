#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::kernel::Form;

use super::{
    direct_inspection_source, direct_interface_skeleton, inspect_direct, HaraValueType,
    MemoryBindingPlan, WasmInterface,
};

#[cfg(test)]
mod tests;

pub const DIRECT_WASM_BINDING_SCHEMA: &str = "hara.wasm-binding/0-alpha";
pub const DIRECT_WASM_CONFORMANCE_SCHEMA: &str = "hara.wasm-conformance/0-alpha";
pub const DIRECT_WASM_BUILD_PRODUCT_SCHEMA: &str = "hara.wasm-build-product/0-alpha";

const PACKAGE_FILE: &str = "package.edn";
const INTERFACE_FILE: &str = "interface.hal";
const BINDINGS_FILE: &str = "bindings.edn";
const BUILD_PRODUCT_FILE: &str = "hara.build-product.edn";
const CONFORMANCE_FILE: &str = "conformance/bindings.edn";
const GENERATED_VERSION: &str = "0.1.0";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingTarget {
    CoreV1,
    MemoryV1,
}

impl BindingTarget {
    pub fn as_keyword(self) -> &'static str {
        match self {
            Self::CoreV1 => "core.v1",
            Self::MemoryV1 => "memory.v1",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionArtifact {
    pub namespace: String,
    pub module: String,
    pub interface_source: String,
    pub inspection_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundPackage {
    pub root: PathBuf,
    pub namespace: String,
    pub module: String,
    pub target: BindingTarget,
    pub module_digest: String,
    pub interface_digest: String,
    pub binding_digest: String,
    pub files: Vec<String>,
}

pub fn inspect_module(
    module_path: &Path,
    namespace: Option<&str>,
) -> Result<InspectionArtifact, String> {
    let bytes = read_bytes(module_path, "module")?;
    let inspection = inspect_direct(&bytes)?;
    let module = file_name(module_path, "module")?;
    let namespace = namespace
        .map(str::to_owned)
        .unwrap_or_else(|| generated_namespace(&module));
    let interface_source = direct_interface_skeleton(&namespace, &module, &inspection)?;
    let inspection_source = direct_inspection_source(&inspection);
    Ok(InspectionArtifact {
        namespace,
        module,
        interface_source,
        inspection_source,
    })
}

pub fn write_interface_skeleton(
    module_path: &Path,
    output_path: &Path,
    namespace: Option<&str>,
) -> Result<InspectionArtifact, String> {
    let artifact = inspect_module(module_path, namespace)?;
    write_new_file(output_path, artifact.interface_source.as_bytes())?;
    Ok(artifact)
}

pub fn bind_package(
    interface_path: &Path,
    module_path: &Path,
    output_root: &Path,
) -> Result<BoundPackage, String> {
    if output_root.exists() {
        return Err(format!(
            "wasm-binding/output-exists: {}",
            output_root.display()
        ));
    }

    let interface_input = read_text(interface_path, "interface")?;
    let interface = WasmInterface::parse(&interface_input, &interface_path.display().to_string())?;
    let module_bytes = read_bytes(module_path, "module")?;
    let inspection = inspect_direct(&module_bytes)?;
    let (target, memory_plan) = binding_target(&interface, &inspection)?;

    let canonical_interface = interface.canonical_source();
    let module_digest = digest(&module_bytes);
    let interface_digest = digest(canonical_interface.as_bytes());
    let bindings = match memory_plan.as_ref() {
        Some(plan) => plan.canonical_source(),
        None => direct_binding_document(&interface, &module_digest, &interface_digest),
    };
    let binding_digest = digest(bindings.as_bytes());
    let project = project_document(&interface, target)?;
    let conformance = conformance_document(
        &interface,
        target,
        &module_digest,
        &interface_digest,
        &binding_digest,
    )?;
    let package_identity = package_identity(&interface.namespace);
    let build_product = build_product_document(
        &interface,
        target,
        &module_digest,
        &interface_digest,
        &binding_digest,
    );

    let mut files = BTreeMap::<String, Vec<u8>>::new();
    files.insert(interface.module.clone(), module_bytes);
    files.insert(INTERFACE_FILE.into(), canonical_interface.into_bytes());
    files.insert(BINDINGS_FILE.into(), bindings.into_bytes());
    files.insert(BUILD_PRODUCT_FILE.into(), build_product.into_bytes());
    files.insert(CONFORMANCE_FILE.into(), conformance.into_bytes());
    files.insert("project.edn".into(), project.into_bytes());
    let package = package_document(&interface, target, &package_identity, &files)?;
    files.insert(PACKAGE_FILE.into(), package.into_bytes());
    write_atomic_tree(output_root, &files)?;

    Ok(BoundPackage {
        root: output_root.to_path_buf(),
        namespace: interface.namespace,
        module: interface.module,
        target,
        module_digest,
        interface_digest,
        binding_digest,
        files: files.keys().cloned().collect(),
    })
}

fn binding_target(
    interface: &WasmInterface,
    inspection: &super::DirectWasmInspection,
) -> Result<(BindingTarget, Option<MemoryBindingPlan>), String> {
    if interface.memory.is_some() {
        let plan = interface.memory_plan()?;
        plan.verify(inspection)?;
        Ok((BindingTarget::MemoryV1, Some(plan)))
    } else {
        interface.verify_direct(inspection)?;
        Ok((BindingTarget::CoreV1, None))
    }
}

fn project_document(interface: &WasmInterface, target: BindingTarget) -> Result<String, String> {
    let exports = interface
        .exports
        .iter()
        .map(|export| {
            let arguments = export
                .arguments
                .iter()
                .map(|argument| manifest_type(&argument.hara_type))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                string_form(&export.name),
                Form::Map(vec![
                    (
                        keyword_form("wasm/export"),
                        string_form(&export.wasm_export),
                    ),
                    (keyword_form("args"), Form::Vector(arguments)),
                    (
                        keyword_form("returns"),
                        manifest_type(&export.returns.hara_type)?,
                    ),
                    (keyword_form("async"), Form::Bool(false)),
                ]),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let assets = [
        INTERFACE_FILE,
        BINDINGS_FILE,
        BUILD_PRODUCT_FILE,
        CONFORMANCE_FILE,
    ]
    .into_iter()
    .map(string_form)
    .collect();
    let extension = Form::Map(vec![
        (
            keyword_form("identity"),
            string_form(&package_identity(&interface.namespace)),
        ),
        (keyword_form("provider"), keyword_form("wasm")),
        (keyword_form("module"), string_form(&interface.module)),
        (keyword_form("abi"), keyword_form(target.as_keyword())),
        (keyword_form("exports"), Form::Map(exports)),
        (keyword_form("capabilities"), Form::Vector(Vec::new())),
        (keyword_form("assets"), Form::Vector(assets)),
    ]);
    let project_id = format!("generated/{}", interface.namespace.replace('.', "-"));
    Ok(document(Form::Map(vec![
        (keyword_form("hara/type"), keyword_form("project")),
        (keyword_form("hara/version"), string_form("1.0.0")),
        (keyword_form("project/id"), symbol_form(&project_id)),
        (
            keyword_form("project/version"),
            string_form(GENERATED_VERSION),
        ),
        (
            keyword_form("project/source-paths"),
            Form::Vector(Vec::new()),
        ),
        (keyword_form("project/test-paths"), Form::Vector(Vec::new())),
        (
            keyword_form("project/extension-paths"),
            Form::Vector(Vec::new()),
        ),
        (keyword_form("project/capabilities"), Form::Set(Vec::new())),
        (
            keyword_form("project/extensions"),
            Form::Map(vec![(symbol_form(&interface.namespace), extension)]),
        ),
    ])))
}

fn package_document(
    interface: &WasmInterface,
    target: BindingTarget,
    identity: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<String, String> {
    let file_entries = files
        .iter()
        .map(|(path, bytes)| {
            (
                string_form(path),
                Form::Map(vec![
                    (keyword_form("sha256"), string_form(&digest(bytes))),
                    (keyword_form("size"), Form::Number(bytes.len() as i64)),
                ]),
            )
        })
        .collect();
    let exports = interface
        .exports
        .iter()
        .map(|export| keyword_form(&export.name))
        .collect();
    let entry_point = interface
        .exports
        .first()
        .map(|export| export.wasm_export.clone())
        .ok_or_else(|| "wasm-binding/package-invalid: package requires an export".to_owned())?;
    let artifact = Form::Map(vec![
        (
            keyword_form("variant/artifact"),
            Form::Map(vec![
                (keyword_form("artifact/type"), keyword_form("wasm")),
                (
                    keyword_form("artifact/path"),
                    string_form(&interface.module),
                ),
                (
                    keyword_form("artifact/sha256"),
                    string_form(
                        files
                            .get(&interface.module)
                            .map(|bytes| digest(bytes))
                            .ok_or_else(|| {
                                format!(
                                    "wasm-binding/package-invalid: missing module {}",
                                    interface.module
                                )
                            })?
                            .as_str(),
                    ),
                ),
                (
                    keyword_form("artifact/target"),
                    string_form("wasm32-wasi-preview1"),
                ),
                (
                    keyword_form("artifact/abi"),
                    string_form(target.as_keyword()),
                ),
                (
                    keyword_form("artifact/entry-point"),
                    string_form(&entry_point),
                ),
            ]),
        ),
        (
            keyword_form("variant/required-capabilities"),
            Form::Set(Vec::new()),
        ),
        (keyword_form("variant/host-calls"), Form::Set(Vec::new())),
        (keyword_form("variant/exports"), Form::Set(exports)),
    ]);
    Ok(document(Form::Map(vec![
        (keyword_form("harp/format"), string_form("0.0.0-alpha")),
        (
            keyword_form("package"),
            Form::Map(vec![
                (keyword_form("identity"), string_form(identity)),
                (keyword_form("version"), string_form(GENERATED_VERSION)),
                (
                    keyword_form("provenance"),
                    Form::Map(vec![
                        (
                            keyword_form("repository"),
                            string_form("generated/hara-wasm-bindgen"),
                        ),
                        (
                            keyword_form("commit"),
                            string_form(
                                digest(
                                    files
                                        .get("project.edn")
                                        .map(Vec::as_slice)
                                        .unwrap_or_default(),
                                )
                                .trim_start_matches("sha256:"),
                            ),
                        ),
                    ]),
                ),
            ]),
        ),
        (keyword_form("files"), Form::Map(file_entries)),
        (
            keyword_form("wasm-imports"),
            Form::Map(vec![(keyword_form(&interface.namespace), artifact)]),
        ),
    ])))
}

fn direct_binding_document(
    interface: &WasmInterface,
    module_digest: &str,
    interface_digest: &str,
) -> String {
    document(Form::Map(vec![
        (
            keyword_form("schema"),
            string_form(DIRECT_WASM_BINDING_SCHEMA),
        ),
        (keyword_form("target"), keyword_form("core.v1")),
        (keyword_form("namespace"), symbol_form(&interface.namespace)),
        (
            keyword_form("module"),
            Form::Map(vec![
                (keyword_form("path"), string_form(&interface.module)),
                (keyword_form("digest"), string_form(module_digest)),
            ]),
        ),
        (
            keyword_form("interface"),
            Form::Map(vec![
                (keyword_form("path"), string_form(INTERFACE_FILE)),
                (keyword_form("digest"), string_form(interface_digest)),
            ]),
        ),
        (
            keyword_form("exports"),
            Form::Vector(
                interface
                    .exports
                    .iter()
                    .map(direct_export_contract)
                    .collect(),
            ),
        ),
    ]))
}

fn conformance_document(
    interface: &WasmInterface,
    target: BindingTarget,
    module_digest: &str,
    interface_digest: &str,
    binding_digest: &str,
) -> Result<String, String> {
    let exports = interface
        .exports
        .iter()
        .map(public_export_contract)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(document(Form::Map(vec![
        (
            keyword_form("schema"),
            string_form(DIRECT_WASM_CONFORMANCE_SCHEMA),
        ),
        (keyword_form("target"), keyword_form(target.as_keyword())),
        (keyword_form("namespace"), symbol_form(&interface.namespace)),
        (keyword_form("module-digest"), string_form(module_digest)),
        (
            keyword_form("interface-digest"),
            string_form(interface_digest),
        ),
        (keyword_form("binding-digest"), string_form(binding_digest)),
        (keyword_form("exports"), Form::Vector(exports)),
    ])))
}

fn build_product_document(
    interface: &WasmInterface,
    target: BindingTarget,
    module_digest: &str,
    interface_digest: &str,
    binding_digest: &str,
) -> String {
    document(Form::Map(vec![
        (
            keyword_form("schema"),
            string_form(DIRECT_WASM_BUILD_PRODUCT_SCHEMA),
        ),
        (
            keyword_form("product/type"),
            keyword_form("extension-wasm-module"),
        ),
        (
            keyword_form("product/namespace"),
            symbol_form(&interface.namespace),
        ),
        (
            keyword_form("product/target"),
            keyword_form(target.as_keyword()),
        ),
        (
            keyword_form("product/tool"),
            Form::Map(vec![
                (keyword_form("name"), string_form("hara-wasm-bindgen")),
                (
                    keyword_form("version"),
                    string_form(env!("CARGO_PKG_VERSION")),
                ),
            ]),
        ),
        (
            keyword_form("product/inputs"),
            Form::Map(vec![
                (keyword_form("module-digest"), string_form(module_digest)),
                (
                    keyword_form("interface-digest"),
                    string_form(interface_digest),
                ),
            ]),
        ),
        (
            keyword_form("product/binding-digest"),
            string_form(binding_digest),
        ),
        (
            keyword_form("product/files"),
            Form::Vector(
                [
                    "project.edn",
                    interface.module.as_str(),
                    INTERFACE_FILE,
                    BINDINGS_FILE,
                    BUILD_PRODUCT_FILE,
                    CONFORMANCE_FILE,
                ]
                .into_iter()
                .map(string_form)
                .collect(),
            ),
        ),
    ]))
}

fn direct_export_contract(export: &super::BindingFunction) -> Form {
    Form::Map(vec![
        (keyword_form("hara/name"), symbol_form(&export.name)),
        (
            keyword_form("wasm/export"),
            string_form(&export.wasm_export),
        ),
        (
            keyword_form("arguments"),
            Form::Vector(
                export
                    .arguments
                    .iter()
                    .map(|argument| keyword_form(argument.wasm_type.as_keyword()))
                    .collect(),
            ),
        ),
        (
            keyword_form("returns"),
            keyword_form(export.returns.wasm_type.as_keyword()),
        ),
    ])
}

fn public_export_contract(export: &super::BindingFunction) -> Result<Form, String> {
    let arguments = export
        .arguments
        .iter()
        .map(|argument| manifest_type(&argument.hara_type))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Form::Map(vec![
        (keyword_form("hara/name"), symbol_form(&export.name)),
        (
            keyword_form("wasm/export"),
            string_form(&export.wasm_export),
        ),
        (keyword_form("arguments"), Form::Vector(arguments)),
        (
            keyword_form("returns"),
            manifest_type(&export.returns.hara_type)?,
        ),
    ]))
}

fn manifest_type(value: &HaraValueType) -> Result<Form, String> {
    let name = match value {
        HaraValueType::I32 => "i32",
        HaraValueType::I64 => "i64",
        HaraValueType::F32 => "f32",
        HaraValueType::F64 => "f64",
        HaraValueType::Boolean => "boolean",
        HaraValueType::String => "string",
        HaraValueType::Bytes => "bytes",
        HaraValueType::Void => "void",
        HaraValueType::Record(name)
        | HaraValueType::Variant(name)
        | HaraValueType::Handle(name)
        | HaraValueType::Callback(name) => {
            return Err(format!(
                "wasm-binding/feature-unsupported: manifest type {name} is not available in this package target"
            ))
        }
    };
    Ok(keyword_form(name))
}

fn write_atomic_tree(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let parent = root.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "wasm-binding/output-unavailable: {} ({error})",
            parent.display()
        )
    })?;
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("package");
    let temp = parent.join(format!(
        ".{name}.hara-bind-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).map_err(|error| {
            format!(
                "wasm-binding/output-unavailable: {} ({error})",
                temp.display()
            )
        })?;
    }

    let result = (|| {
        fs::create_dir(&temp).map_err(|error| {
            format!(
                "wasm-binding/output-unavailable: {} ({error})",
                temp.display()
            )
        })?;
        for (relative, bytes) in files {
            let target = temp.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "wasm-binding/output-unavailable: {} ({error})",
                        parent.display()
                    )
                })?;
            }
            fs::write(&target, bytes).map_err(|error| {
                format!(
                    "wasm-binding/output-unavailable: {} ({error})",
                    target.display()
                )
            })?;
        }
        fs::rename(&temp, root).map_err(|error| {
            format!(
                "wasm-binding/output-unavailable: {} ({error})",
                root.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp);
    }
    result
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return Err(format!("wasm-binding/output-exists: {}", path.display()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "wasm-binding/output-unavailable: {} ({error})",
                parent.display()
            )
        })?;
    }
    let temp = path.with_extension(format!(
        "hara-bind-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp, bytes).map_err(|error| {
        format!(
            "wasm-binding/output-unavailable: {} ({error})",
            temp.display()
        )
    })?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "wasm-binding/output-unavailable: {} ({error})",
            path.display()
        ));
    }
    Ok(())
}

fn read_bytes(path: &Path, subject: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| {
        format!(
            "wasm-binding/input-unavailable: {subject} {} ({error})",
            path.display()
        )
    })
}

fn read_text(path: &Path, subject: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| {
        format!(
            "wasm-binding/input-unavailable: {subject} {} ({error})",
            path.display()
        )
    })
}

fn file_name(path: &Path, subject: &str) -> Result<String, String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("wasm-binding/input-unavailable: {subject} path has no UTF-8 file name")
        })
}

fn generated_namespace(module: &str) -> String {
    let stem = module.strip_suffix(".wasm").unwrap_or(module);
    let mut component = String::new();
    let mut separated = false;
    for character in stem.chars() {
        if character.is_ascii_alphanumeric() {
            if separated && !component.is_empty() {
                component.push('-');
            }
            component.push(character.to_ascii_lowercase());
            separated = false;
        } else {
            separated = true;
        }
    }
    if component.is_empty() {
        component.push_str("module");
    }
    format!("generated.{component}")
}

fn package_identity(namespace: &str) -> String {
    format!("generated/{}", namespace.replace('.', "-"))
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn document(form: Form) -> String {
    format!("{form}\n")
}

fn keyword_form(value: &str) -> Form {
    Form::Keyword(value.to_owned())
}

fn symbol_form(value: &str) -> Form {
    Form::Symbol(value.to_owned())
}

fn string_form(value: &str) -> Form {
    Form::String(value.to_owned())
}
