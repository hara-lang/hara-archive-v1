//! A bounded, data-only bridge for the WebAssembly Interface Types format.
//!
//! WIT is used to seed a canonical Hara interface and to project the exact
//! subset already represented by that interface. It never selects a fallback
//! loader route or invents a second runtime type system.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sha2::{Digest, Sha256};

use crate::kernel::Form;

use super::wit_format;
use super::wit_parser::{self, Document, Interface, Type, TypeDecl};
use super::{HaraValueType, Lifting, Lowering, WasmInterface};

pub const WIT_IR_SCHEMA: &str = "hara.wasm-wit/0-alpha";
pub const WIT_MANIFEST_SCHEMA: &str = "hara.wasm-wit-manifest/0-alpha";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WitDiagnosticSeverity {
    Lossy,
    Unsupported,
}

impl WitDiagnosticSeverity {
    pub(super) fn as_keyword(self) -> &'static str {
        match self {
            Self::Lossy => "lossy",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WitDiagnostic {
    pub severity: WitDiagnosticSeverity,
    pub code: String,
    pub path: String,
    pub message: String,
}

impl fmt::Display for WitDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} at {}: {}",
            self.severity.as_keyword(),
            self.code,
            self.path,
            self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WitRoute {
    DirectImport,
    HtaRequire,
}

impl WitRoute {
    pub fn as_keyword(self) -> &'static str {
        match self {
            Self::DirectImport => "import",
            Self::HtaRequire => "require",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WitImportOptions {
    pub namespace: Option<String>,
    pub module: Option<String>,
    pub world: Option<String>,
    pub interface: Option<String>,
    pub strict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitImportArtifact {
    pub namespace: String,
    pub world: Option<String>,
    pub interface: String,
    pub route: WitRoute,
    pub interface_source: String,
    pub normalized_ir: String,
    pub diagnostics: Vec<WitDiagnostic>,
    pub source_digest: String,
    pub interface_digest: String,
}

impl WitImportArtifact {
    pub fn manifest_source(&self, origin: &str) -> String {
        Form::Map(vec![
            (keyword("schema"), string(WIT_MANIFEST_SCHEMA)),
            (keyword("origin"), string(origin)),
            (keyword("source-digest"), string(&self.source_digest)),
            (keyword("interface-digest"), string(&self.interface_digest)),
            (keyword("route"), keyword(self.route.as_keyword())),
            (
                keyword("diagnostics"),
                Form::Vector(
                    self.diagnostics
                        .iter()
                        .map(wit_format::diagnostic_form)
                        .collect(),
                ),
            ),
        ])
        .to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WitProjectionOptions {
    pub package: Option<String>,
    pub interface: Option<String>,
    pub world: Option<String>,
    pub strict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitProjectionArtifact {
    pub source: String,
    pub diagnostics: Vec<WitDiagnostic>,
}

pub fn import_wit(
    source: &str,
    origin: &str,
    options: &WitImportOptions,
) -> Result<WitImportArtifact, String> {
    let document = wit_parser::parse(source)
        .map_err(|error| format!("wasm-wit/malformed {origin}: {error}"))?;
    let (interface_name, world_name, interface, mut diagnostics) =
        select_interface(&document, options, origin)?;
    let namespace = options
        .namespace
        .clone()
        .unwrap_or_else(|| default_namespace(document.package.as_deref(), &interface_name));
    if !valid_namespace(&namespace) {
        return Err(format!(
            "wasm-wit/malformed {origin}: namespace must be a qualified lower-case name"
        ));
    }
    let module = options
        .module
        .clone()
        .unwrap_or_else(|| "module.wasm".to_owned());
    if !valid_module(&module) {
        return Err(format!(
            "wasm-wit/malformed {origin}: module must be a safe relative .wasm path"
        ));
    }

    let mut context = MappingContext {
        types: &interface.types,
        diagnostics: &mut diagnostics,
        memory: false,
        hta: !interface.resources.is_empty(),
        reported: BTreeSet::new(),
    };
    for resource in &interface.resources {
        context.diagnostic(
            WitDiagnosticSeverity::Lossy,
            "resource",
            resource,
            "resources require explicit HTA handle ownership and release semantics",
        );
    }
    let mut exports = Vec::new();
    for function in &interface.functions {
        if function.async_ {
            context.diagnostic(
                WitDiagnosticSeverity::Unsupported,
                "async",
                &function.name,
                "async WIT functions require an HTA provider",
            );
            context.hta = true;
        }
        let arguments = function
            .arguments
            .iter()
            .enumerate()
            .map(|(index, (name, ty))| {
                let path = format!("{}.argument.{index}", function.name);
                let mapped = context.map_type(ty, &path, &function.name);
                (name.clone(), mapped)
            })
            .collect::<Vec<_>>();
        let result = function
            .result
            .as_ref()
            .map(|ty| context.map_type(ty, &format!("{}.result", function.name), &function.name));
        exports.push((function, arguments, result));
    }
    for (name, declaration) in &interface.types {
        if let TypeDecl::Unsupported(feature) = declaration {
            context.diagnostic(
                WitDiagnosticSeverity::Unsupported,
                feature,
                &format!("type.{name}"),
                "the declaration is not represented by the Hara binding IR",
            );
        }
    }
    if !document
        .worlds
        .get(world_name.as_deref().unwrap_or(""))
        .map_or(true, |world| world.imports.is_empty())
    {
        context.diagnostic(
            WitDiagnosticSeverity::Unsupported,
            "world-import",
            "world",
            "imported host interfaces need an explicit HTA capability contract",
        );
        context.hta = true;
    }
    if exports.is_empty() {
        context.diagnostic(
            WitDiagnosticSeverity::Unsupported,
            "empty-interface",
            "interface",
            "an empty interface cannot seed a bindable Hara export map",
        );
    }
    let memory = context.memory;
    let hta = context.hta;
    drop(context);
    diagnostics.sort();
    if options.strict && !diagnostics.is_empty() {
        return Err(strict_error(origin, &diagnostics));
    }

    let route = if hta {
        WitRoute::HtaRequire
    } else {
        WitRoute::DirectImport
    };
    let interface_source =
        wit_format::skeleton_source(&namespace, &module, &exports, memory, route);
    let normalized_ir = wit_format::normalized_source(
        &namespace,
        document.package.as_deref(),
        world_name.as_deref(),
        &interface_name,
        route,
        &interface,
        &exports,
        &diagnostics,
        source,
        origin,
    );
    Ok(WitImportArtifact {
        namespace,
        world: world_name,
        interface: interface_name,
        route,
        source_digest: digest(source.as_bytes()),
        interface_digest: digest(interface_source.as_bytes()),
        interface_source,
        normalized_ir,
        diagnostics,
    })
}

pub fn project_wit(
    interface: &WasmInterface,
    options: &WitProjectionOptions,
) -> Result<WitProjectionArtifact, String> {
    let mut diagnostics = Vec::new();
    let mut functions = Vec::new();
    for export in &interface.exports {
        let args = export
            .arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                wit_format::wit_projection_type(
                    &argument.hara_type,
                    argument.wasm_type,
                    argument.lowering,
                    argument.ownership,
                    &mut diagnostics,
                    &format!("{}.argument.{index}", export.name),
                )
                .map(|ty| (argument.name.clone(), ty))
            })
            .collect::<Option<Vec<_>>>();
        let result = wit_format::wit_projection_type(
            &export.returns.hara_type,
            export.returns.wasm_type,
            export.returns.lifting.map(|value| match value {
                Lifting::Direct => Lowering::Direct,
                Lifting::PointerLength => Lowering::PointerLength,
                Lifting::PackedI64 => Lowering::PointerLength,
            }),
            export.returns.ownership,
            &mut diagnostics,
            &format!("{}.result", export.name),
        );
        if let (Some(args), Some(result)) = (args, result) {
            functions.push((export.name.clone(), args, result));
        }
        if export.asynchronous {
            wit_format::diagnostic(
                &mut diagnostics,
                WitDiagnosticSeverity::Unsupported,
                "async",
                &export.name,
                "HTA asynchronous exports are not part of an exact WIT projection",
            );
        }
        if export.errors.is_some() {
            wit_format::diagnostic(
                &mut diagnostics,
                WitDiagnosticSeverity::Unsupported,
                "error-mapping",
                &export.name,
                "Hara error conventions need an explicit WIT result definition",
            );
        }
        if !export.capabilities.is_empty() || !interface.capabilities.is_empty() {
            wit_format::diagnostic(
                &mut diagnostics,
                WitDiagnosticSeverity::Unsupported,
                "capability",
                &export.name,
                "Hara capabilities are not representable in a WIT interface",
            );
        }
    }
    diagnostics.sort();
    if options.strict && !diagnostics.is_empty() {
        return Err(strict_error(&interface.namespace, &diagnostics));
    }
    let interface_name = options.interface.clone().unwrap_or_else(|| {
        interface
            .namespace
            .rsplit('.')
            .next()
            .unwrap_or("interface")
            .into()
    });
    let package = options
        .package
        .clone()
        .unwrap_or_else(|| default_package(&interface.namespace));
    let world = options
        .world
        .clone()
        .unwrap_or_else(|| format!("{interface_name}-world"));
    if !valid_wit_name(&interface_name) || !valid_wit_name(&world) {
        return Err("wasm-wit/malformed: projection names must be valid WIT identifiers".into());
    }
    let mut source = format!("package {package};\n\ninterface {interface_name} {{\n");
    for (name, args, result) in functions {
        source.push_str("  ");
        source.push_str(&name);
        source.push_str(": func(");
        source.push_str(
            &args
                .into_iter()
                .map(|(name, ty)| format!("{name}: {ty}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        source.push(')');
        if result != "unit" {
            source.push_str(" -> ");
            source.push_str(&result);
        }
        source.push_str(";\n");
    }
    source.push_str("}\n\nworld ");
    source.push_str(&world);
    source.push_str(" {\n  export ");
    source.push_str(&interface_name);
    source.push_str(";\n}\n");
    Ok(WitProjectionArtifact {
        source,
        diagnostics,
    })
}

fn select_interface(
    document: &Document,
    options: &WitImportOptions,
    origin: &str,
) -> Result<(String, Option<String>, Interface, Vec<WitDiagnostic>), String> {
    let mut diagnostics = Vec::new();
    let world_name = options.world.clone().or_else(|| {
        (document.worlds.len() == 1)
            .then(|| document.worlds.keys().next().cloned())
            .flatten()
    });
    if options.world.is_some()
        && !document
            .worlds
            .contains_key(options.world.as_ref().unwrap())
    {
        return Err(format!(
            "wasm-wit/malformed {origin}: requested world is not declared"
        ));
    }
    let interface_name = if let Some(name) = options.interface.clone() {
        name
    } else if let Some(world) = world_name
        .as_ref()
        .and_then(|name| document.worlds.get(name))
    {
        let matches = world
            .exports
            .iter()
            .filter(|name| document.interfaces.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            diagnostics.push(WitDiagnostic {
                severity: WitDiagnosticSeverity::Unsupported,
                code: "world-export-selection".into(),
                path: "world".into(),
                message: "a world must select exactly one declared interface".into(),
            });
        }
        matches
            .first()
            .cloned()
            .or_else(|| document.interfaces.keys().next().cloned())
            .ok_or_else(|| format!("wasm-wit/malformed {origin}: world has no interface export"))?
    } else if document.interfaces.len() == 1 {
        document.interfaces.keys().next().cloned().unwrap()
    } else {
        diagnostics.push(WitDiagnostic {
            severity: WitDiagnosticSeverity::Unsupported,
            code: "interface-selection".into(),
            path: "document".into(),
            message: "multiple interfaces require --interface or a world export".into(),
        });
        document
            .interfaces
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| format!("wasm-wit/malformed {origin}: no interface was declared"))?
    };
    let interface = document
        .interfaces
        .get(&interface_name)
        .cloned()
        .ok_or_else(|| {
            format!("wasm-wit/malformed {origin}: interface {interface_name} is not declared")
        })?;
    Ok((interface_name, world_name, interface, diagnostics))
}

struct MappingContext<'a> {
    types: &'a BTreeMap<String, TypeDecl>,
    diagnostics: &'a mut Vec<WitDiagnostic>,
    memory: bool,
    hta: bool,
    reported: BTreeSet<String>,
}

impl MappingContext<'_> {
    fn map_type(&mut self, ty: &Type, path: &str, context: &str) -> HaraValueType {
        match ty {
            Type::Atom(name) => match name.as_str() {
                "bool" => HaraValueType::Boolean,
                "u8" | "u16" | "u32" | "s8" | "s16" => {
                    self.diagnostic(
                        WitDiagnosticSeverity::Lossy,
                        "integer-width",
                        path,
                        &format!("WIT {name} is lowered to the existing Hara i32 scalar"),
                    );
                    HaraValueType::I32
                }
                "s32" => HaraValueType::I32,
                "u64" => {
                    self.diagnostic(
                        WitDiagnosticSeverity::Lossy,
                        "integer-width",
                        path,
                        "WIT u64 is lowered to the existing Hara i64 scalar",
                    );
                    HaraValueType::I64
                }
                "s64" => HaraValueType::I64,
                "f32" => HaraValueType::F32,
                "f64" => HaraValueType::F64,
                "char" => {
                    self.diagnostic(
                        WitDiagnosticSeverity::Lossy,
                        "char",
                        path,
                        "WIT char is lowered to the existing Hara i32 scalar",
                    );
                    HaraValueType::I32
                }
                "string" => {
                    self.memory = true;
                    HaraValueType::String
                }
                "unit" => HaraValueType::Void,
                name => match self.types.get(name) {
                    Some(TypeDecl::Alias(value)) => self.map_type(value, path, context),
                    Some(TypeDecl::Record(_)) => {
                        self.lossy_named("record", name, path);
                        HaraValueType::Record(name.into())
                    }
                    Some(TypeDecl::Variant(_)) => {
                        self.lossy_named("variant", name, path);
                        HaraValueType::Variant(name.into())
                    }
                    Some(TypeDecl::Resource) => {
                        self.hta = true;
                        self.diagnostic(
                            WitDiagnosticSeverity::Lossy,
                            "resource",
                            path,
                            "resource is represented as an HTA-owned Hara handle",
                        );
                        HaraValueType::Handle(name.into())
                    }
                    Some(TypeDecl::Unsupported(_)) | None => {
                        self.diagnostic(
                            WitDiagnosticSeverity::Unsupported,
                            "type",
                            path,
                            &format!("named type {name} is not represented"),
                        );
                        HaraValueType::Variant(name.into())
                    }
                },
            },
            Type::List(value) => {
                if matches!(value.as_ref(), Type::Atom(name) if name == "u8") {
                    self.memory = true;
                    HaraValueType::Bytes
                } else {
                    self.hta = true;
                    self.diagnostic(
                        WitDiagnosticSeverity::Lossy,
                        "list",
                        path,
                        "only list<u8> has an exact existing Hara bytes representation",
                    );
                    HaraValueType::Record(format!("{}-list", context))
                }
            }
            Type::Option(value) => {
                self.hta = true;
                self.diagnostic(
                    WitDiagnosticSeverity::Lossy,
                    "option",
                    path,
                    &format!(
                        "option<{}> is represented as an HTA variant until a canonical Hara option owner is authored",
                        wit_parser::type_label(value)
                    ),
                );
                HaraValueType::Variant(format!("{}-option", context))
            }
            Type::Result(ok, error) => {
                self.hta = true;
                self.diagnostic(
                    WitDiagnosticSeverity::Lossy,
                    "result",
                    path,
                    &format!(
                        "result<{}, {}> is represented as an HTA variant until Hara error ownership is authored",
                        ok.as_deref()
                            .map(wit_parser::type_label)
                            .unwrap_or_else(|| "unit".into()),
                        error
                            .as_deref()
                            .map(wit_parser::type_label)
                            .unwrap_or_else(|| "unit".into())
                    ),
                );
                HaraValueType::Variant(format!("{}-result", context))
            }
            Type::Tuple(values) => {
                self.hta = true;
                self.diagnostic(
                    WitDiagnosticSeverity::Lossy,
                    "tuple",
                    path,
                    &format!(
                        "tuple of {} values needs an authored Hara record",
                        values.len()
                    ),
                );
                HaraValueType::Record(format!("{}-tuple", context))
            }
        }
    }

    fn lossy_named(&mut self, kind: &str, name: &str, path: &str) {
        self.hta = true;
        self.diagnostic(
            WitDiagnosticSeverity::Lossy,
            kind,
            path,
            &format!("{kind} {name} is seeded by name; fields and ownership remain canonical .hal"),
        );
    }

    fn diagnostic(
        &mut self,
        severity: WitDiagnosticSeverity,
        code: &str,
        path: &str,
        message: &str,
    ) {
        let key = format!("{}:{code}:{path}:{message}", severity.as_keyword());
        if self.reported.insert(key) {
            self.diagnostics.push(WitDiagnostic {
                severity,
                code: code.into(),
                path: path.into(),
                message: message.into(),
            });
        }
    }
}

fn strict_error(origin: &str, diagnostics: &[WitDiagnostic]) -> String {
    let details = diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    format!("wasm-wit/strict {origin}: unsupported or lossy mappings:\n{details}")
}

fn default_namespace(package: Option<&str>, interface: &str) -> String {
    let package = package.unwrap_or("hara:wit");
    let package = package.split('@').next().unwrap_or(package);
    let package = package.replace(':', ".");
    let namespace = if package.rsplit('.').next() == Some(interface) {
        package
    } else {
        format!("{package}.{interface}")
    };
    if valid_namespace(&namespace) {
        namespace
    } else {
        format!("hara.wit.{interface}")
    }
}

fn default_package(namespace: &str) -> String {
    let mut parts = namespace.split('.');
    let namespace = parts.next().unwrap_or("hara");
    let name = parts.last().unwrap_or("interface");
    format!("{namespace}:{name}")
}

fn valid_namespace(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() > 1 && parts.iter().all(|part| valid_wit_name(part))
}

fn valid_wit_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase() || character == '_')
}

fn valid_module(value: &str) -> bool {
    !value.is_empty()
        && value.ends_with(".wasm")
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.contains(':')
        && !value.bytes().any(|byte| byte == 0)
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(super) fn keyword(value: &str) -> Form {
    Form::Keyword(value.into())
}

pub(super) fn symbol(value: &str) -> Form {
    Form::Symbol(value.into())
}

pub(super) fn string(value: &str) -> Form {
    Form::String(value.into())
}

pub(super) fn named_type(kind: &str, name: &str) -> Form {
    Form::Vector(vec![keyword(kind), symbol(name)])
}
