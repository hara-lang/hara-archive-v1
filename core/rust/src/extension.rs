//! Strict metadata and host boundary for runtime-generated WASM namespaces.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub use crate::core::{Promise, PromiseState, Value};
use crate::kernel::{parse, Form};

const MANIFEST_FIELDS: &[&str] = &[
    "root",
    "namespace",
    "identity",
    "version",
    "provider",
    "module",
    "abi",
    "exports",
    "capabilities",
    "host-calls",
    "callbacks",
    "handles",
    "targets",
    "assets",
];
const EXPORT_FIELDS: &[&str] = &["args", "returns", "async", "wasm/export", "operation"];
const HANDLE_FIELDS: &[&str] = &["tag", "release"];
const TARGET_FIELDS: &[&str] = &["provider", "runtime"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmAbi {
    CoreV1,
    HtaV1,
    MemoryV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionExport {
    pub arguments: Vec<String>,
    pub returns: String,
    pub asynchronous: bool,
    pub raw_export: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionCallback {
    pub arguments: Vec<String>,
    pub returns: String,
}

impl ExtensionExport {
    pub fn raw_name<'a>(&'a self, public_name: &'a str) -> &'a str {
        self.raw_export.as_deref().unwrap_or(public_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionTarget {
    pub provider: String,
    pub runtime: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifest {
    pub namespace: String,
    pub root: Option<String>,
    pub identity: Option<String>,
    pub version: String,
    pub provider: String,
    pub module: Option<String>,
    pub abi: WasmAbi,
    pub targets: HashMap<String, ExtensionTarget>,
    pub assets: Vec<String>,
    pub exports: Vec<(String, ExtensionExport)>,
    pub operations: HashMap<String, String>,
    pub capabilities: Vec<String>,
    pub host_calls: HashMap<String, Vec<String>>,
    pub host_call_capabilities: HashMap<String, Vec<String>>,
    pub callbacks: HashMap<String, ExtensionCallback>,
    pub handle_tags: HashMap<String, String>,
    pub handle_releases: HashMap<String, String>,
}

impl ExtensionManifest {
    pub fn parse(source: &str, origin: &str) -> Result<Self, String> {
        let form = parse(source)
            .map_err(|error| malformed(origin, format!("cannot parse manifest: {error}")))?;
        let entries = map(&form, origin, "manifest")?;
        reject_unknown(entries, MANIFEST_FIELDS, origin, "manifest")?;
        let namespace = named_string(entries, "namespace", origin)?;
        if !valid_namespace(&namespace) {
            return Err(malformed(
                origin,
                "namespace must be a qualified lower-case symbol",
            ));
        }
        let root = optional(entries, "root")
            .map(|form| string(form, origin, "root").map(str::to_owned))
            .transpose()?;
        if let Some(root) = root.as_deref() {
            safe_relative(root, None, origin, "root")?;
        }
        let identity = optional(entries, "identity")
            .map(|form| string(form, origin, "identity").map(str::to_owned))
            .transpose()?;
        if identity
            .as_deref()
            .is_some_and(|value| !valid_identity(value))
        {
            return Err(malformed(
                origin,
                "identity must be an owner/name coordinate",
            ));
        }
        let version = named_string(entries, "version", origin)?;
        let provider = named_keyword(entries, "provider", origin)?;
        let module = optional(entries, "module")
            .map(|form| string(form, origin, "module").map(str::to_owned))
            .transpose()?;
        let targets = optional(entries, "targets")
            .map_or_else(|| Ok(HashMap::new()), |form| parse_targets(form, origin))?;
        let assets = optional(entries, "assets")
            .map_or_else(|| Ok(Vec::new()), |form| parse_assets(form, origin))?;
        match provider.as_str() {
            "wasm" if module.is_some() && targets.is_empty() => {
                safe_relative(module.as_deref().unwrap(), Some(".wasm"), origin, "module")?;
            }
            "hta" if module.is_none() && !targets.is_empty() => {}
            "wasm" => {
                return Err(malformed(
                    origin,
                    "WASM providers require :module and cannot declare :targets",
                ))
            }
            "hta" => {
                return Err(malformed(
                    origin,
                    "HTA providers require :targets and cannot declare :module",
                ))
            }
            _ => {
                return Err(malformed(
                    origin,
                    format!("unsupported provider :{provider}"),
                ))
            }
        }
        let abi = match named_keyword(entries, "abi", origin)?.as_str() {
            "core.v1" => WasmAbi::CoreV1,
            "hta.v1" => WasmAbi::HtaV1,
            "memory.v1" => WasmAbi::MemoryV1,
            value => return Err(malformed(origin, format!("unsupported WASM ABI :{value}"))),
        };
        if provider == "hta" && abi != WasmAbi::HtaV1 {
            return Err(malformed(origin, "HTA providers require :abi :hta.v1"));
        }
        let (exports, operations) = parse_exports(required(entries, "exports", origin)?, origin)?;
        let capabilities = keyword_vector(
            required(entries, "capabilities", origin)?,
            origin,
            "capabilities",
        )?;
        let (host_calls, host_call_capabilities) = optional(entries, "host-calls").map_or_else(
            || Ok((HashMap::new(), HashMap::new())),
            |form| parse_host_calls(form, origin),
        )?;
        let callbacks = optional(entries, "callbacks")
            .map_or_else(|| Ok(HashMap::new()), |form| parse_callbacks(form, origin))?;
        let (handle_tags, handle_releases) = optional(entries, "handles").map_or_else(
            || Ok((HashMap::new(), HashMap::new())),
            |form| parse_handles(form, origin),
        )?;
        Ok(Self {
            namespace,
            root,
            identity,
            version,
            provider,
            module,
            targets,
            assets,
            abi,
            exports,
            operations,
            capabilities,
            host_calls,
            host_call_capabilities,
            callbacks,
            handle_tags,
            handle_releases,
        })
    }

    pub fn permits_host_call(&self, service: &str, method: &str) -> bool {
        self.host_calls.get(service).map_or(false, |methods| {
            methods.iter().any(|candidate| candidate == method)
        })
    }

    pub fn host_call_capabilities(&self, service: &str, method: &str) -> &[String] {
        self.host_call_capabilities
            .get(&format!("{service}/{method}"))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

pub trait WasmExtensionProvider {
    fn supports(&self, abi: WasmAbi) -> bool;

    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }

    fn start(&self, manifest: &ExtensionManifest) -> Result<(), String>;

    fn invoke(
        &self,
        manifest: &ExtensionManifest,
        export: &str,
        arguments: &[Value],
    ) -> Result<Value, String>;

    fn cancel(&self, manifest: &ExtensionManifest, request: u64) -> Result<(), String>;

    fn release(&self, _manifest: &ExtensionManifest, _handle: &Value) -> Result<(), String> {
        Err("extension/release-unsupported: provider has no HTA handle boundary".into())
    }

    fn shutdown(&self, manifest: &ExtensionManifest);
}

struct ExtensionSession {
    manifest: ExtensionManifest,
    provider: Rc<dyn WasmExtensionProvider>,
}

impl Drop for ExtensionSession {
    fn drop(&mut self) {
        self.provider.shutdown(&self.manifest);
    }
}

#[derive(Clone)]
pub struct ExtensionBinding {
    pub namespace: String,
    pub name: String,
    pub specification: ExtensionExport,
    session: Rc<ExtensionSession>,
}

impl ExtensionBinding {
    pub fn invoke(&self, arguments: &[Value]) -> Result<Value, String> {
        if arguments.len() != self.specification.arguments.len() {
            return Err(format!(
                "extension/arity: {}/{} expects {} arguments, got {}",
                self.namespace,
                self.name,
                self.specification.arguments.len(),
                arguments.len()
            ));
        }
        let result = self
            .session
            .provider
            .invoke(&self.session.manifest, &self.name, arguments)?;
        if self.specification.asynchronous && !matches!(result, Value::Promise(_)) {
            return Err(format!(
                "extension/protocol: asynchronous export {}/{} must return a promise",
                self.namespace, self.name
            ));
        }
        Ok(result)
    }

    pub fn release(&self, handle: &Value) -> Result<(), String> {
        self.session
            .provider
            .release(&self.session.manifest, handle)
            .map_err(|error| format!("extension/release: {error}"))
    }
}

pub struct WasmExtension {
    manifest: ExtensionManifest,
    provider: Rc<dyn WasmExtensionProvider>,
    session: Option<Rc<ExtensionSession>>,
}

impl WasmExtension {
    pub fn new<P: WasmExtensionProvider + 'static>(
        manifest: ExtensionManifest,
        provider: P,
    ) -> Result<Self, String> {
        if !provider.supports(manifest.abi) {
            return Err(format!(
                "extension/unsupported: provider does not support {:?}",
                manifest.abi
            ));
        }
        Ok(Self {
            manifest,
            provider: Rc::new(provider),
            session: None,
        })
    }

    pub fn namespace(&self) -> &str {
        &self.manifest.namespace
    }

    pub fn require(&mut self) -> Result<Vec<ExtensionBinding>, String> {
        if self.session.is_none() {
            let available = self
                .provider
                .capabilities()
                .into_iter()
                .collect::<HashSet<_>>();
            let missing = self
                .manifest
                .capabilities
                .iter()
                .chain(self.manifest.host_call_capabilities.values().flatten())
                .find(|capability| !available.contains(*capability));
            if let Some(capability) = missing {
                return Err(format!(
                    "extension/denied: {} requires capability :{}",
                    self.manifest.namespace, capability
                ));
            }
            self.provider
                .start(&self.manifest)
                .map_err(|error| format!("extension/start: {error}"))?;
            self.session = Some(Rc::new(ExtensionSession {
                manifest: self.manifest.clone(),
                provider: self.provider.clone(),
            }));
        }
        let session = self.session.as_ref().expect("session started").clone();
        Ok(self
            .manifest
            .exports
            .iter()
            .map(|(name, specification)| ExtensionBinding {
                namespace: self.manifest.namespace.clone(),
                name: name.clone(),
                specification: specification.clone(),
                session: session.clone(),
            })
            .collect())
    }

    pub fn cancel(&self, request: u64) -> Result<(), String> {
        let session = self.session.as_ref().ok_or_else(|| {
            format!(
                "extension/not-started: namespace has not been required: {}",
                self.manifest.namespace
            )
        })?;
        session
            .provider
            .cancel(&session.manifest, request)
            .map_err(|error| format!("extension/cancel: {error}"))
    }
}

fn parse_targets(form: &Form, origin: &str) -> Result<HashMap<String, ExtensionTarget>, String> {
    map(form, origin, "targets")?
        .iter()
        .map(|(host, specification)| {
            let host = keyword(host, origin, "target host")?.to_owned();
            if host != "node" && host != "browser" {
                return Err(malformed(origin, format!("unsupported target :{host}")));
            }
            let entries = map(specification, origin, "target")?;
            reject_unknown(entries, TARGET_FIELDS, origin, &format!("target {host}"))?;
            let provider = named_string(entries, "provider", origin)?;
            safe_relative(&provider, Some(".mjs"), origin, "target provider")?;
            let runtime = named_keyword(entries, "runtime", origin)?;
            let compatible = (host == "node" && runtime == "process")
                || (host == "browser" && runtime == "web-worker");
            if !compatible {
                return Err(malformed(
                    origin,
                    format!("target {host} has incompatible runtime :{runtime}"),
                ));
            }
            Ok((host, ExtensionTarget { provider, runtime }))
        })
        .collect()
}

fn parse_assets(form: &Form, origin: &str) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    vector(form, origin, "assets")?
        .iter()
        .map(|value| {
            let value = string(value, origin, "asset")?.to_owned();
            safe_relative(&value, None, origin, "asset")?;
            if !seen.insert(value.clone()) {
                return Err(malformed(origin, format!("duplicate asset {value}")));
            }
            Ok(value)
        })
        .collect()
}

fn safe_relative(
    value: &str,
    suffix: Option<&str>,
    origin: &str,
    field: &str,
) -> Result<(), String> {
    let unsafe_path = value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.bytes().any(|byte| byte == 0)
        || value.contains(':')
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if unsafe_path || suffix.is_some_and(|suffix| !value.ends_with(suffix)) {
        return Err(malformed(
            origin,
            format!("{field} must be a safe relative package file"),
        ));
    }
    Ok(())
}

fn parse_exports(
    form: &Form,
    origin: &str,
) -> Result<(Vec<(String, ExtensionExport)>, HashMap<String, String>), String> {
    let entries = map(form, origin, "exports")?;
    if entries.is_empty() {
        return Err(malformed(origin, "exports cannot be empty"));
    }
    let mut operations = HashMap::new();
    let exports = entries
        .iter()
        .map(|(name, specification)| {
            let name = string(name, origin, "export name")?.to_owned();
            let specification = map(specification, origin, "export specification")?;
            reject_unknown(
                specification,
                EXPORT_FIELDS,
                origin,
                &format!("export {name}"),
            )?;
            let arguments = wire_vector(
                required(specification, "args", origin)?,
                origin,
                "export args",
            )?;
            let returns = wire_type(
                required(specification, "returns", origin)?,
                origin,
                "export returns",
            )?;
            let asynchronous = match optional(specification, "async") {
                None => false,
                Some(Form::Bool(value)) => *value,
                Some(_) => return Err(malformed(origin, "export async must be boolean")),
            };
            let raw_export = optional(specification, "wasm/export")
                .map(|form| string(form, origin, "export wasm/export").map(str::to_owned))
                .transpose()?;
            if let Some(operation) = optional(specification, "operation") {
                let operation = string(operation, origin, "export operation")?.to_owned();
                if operations.insert(name.clone(), operation).is_some() {
                    return Err(malformed(origin, format!("duplicate export {name}")));
                }
            }
            Ok((
                name,
                ExtensionExport {
                    arguments,
                    returns,
                    asynchronous,
                    raw_export,
                },
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((exports, operations))
}

fn parse_host_calls(
    form: &Form,
    origin: &str,
) -> Result<(HashMap<String, Vec<String>>, HashMap<String, Vec<String>>), String> {
    let mut host_calls = HashMap::new();
    let mut host_call_capabilities = HashMap::new();
    for (service, specification) in map(form, origin, "host-calls")? {
        let service = string(service, origin, "host-call service")?.to_owned();
        let (methods, capabilities) = match specification {
            Form::Vector(_) => (
                vector(specification, origin, "host-call methods")?
                    .iter()
                    .map(|method| string(method, origin, "host-call method").map(str::to_owned))
                    .collect::<Result<Vec<_>, _>>()?,
                Vec::new(),
            ),
            Form::Map(entries) => {
                reject_unknown(entries, &["methods", "capabilities"], origin, "host-call")?;
                let methods = vector(
                    required(entries, "methods", origin)?,
                    origin,
                    "host-call methods",
                )?
                .iter()
                .map(|method| string(method, origin, "host-call method").map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()?;
                let capabilities = optional(entries, "capabilities")
                    .map(|form| keyword_vector(form, origin, "host-call capabilities"))
                    .transpose()?
                    .unwrap_or_default();
                (methods, capabilities)
            }
            _ => {
                return Err(malformed(
                    origin,
                    "host-call specification must be a vector or map",
                ))
            }
        };
        if host_calls
            .insert(service.clone(), methods.clone())
            .is_some()
        {
            return Err(malformed(
                origin,
                format!("duplicate host-call service {service}"),
            ));
        }
        for method in methods {
            host_call_capabilities.insert(format!("{service}/{method}"), capabilities.clone());
        }
    }
    Ok((host_calls, host_call_capabilities))
}

fn parse_handles(
    form: &Form,
    origin: &str,
) -> Result<(HashMap<String, String>, HashMap<String, String>), String> {
    let mut tags = HashMap::new();
    let mut releases = HashMap::new();
    for (type_name, specification) in map(form, origin, "handles")? {
        let type_name = string(type_name, origin, "handle type")?.to_owned();
        let specification = map(specification, origin, "handle specification")?;
        reject_unknown(
            specification,
            HANDLE_FIELDS,
            origin,
            &format!("handle {type_name}"),
        )?;
        let tag = match required(specification, "tag", origin)? {
            Form::Symbol(tag) if valid_tag(tag) => tag.clone(),
            _ => return Err(malformed(origin, "handle tag must be a lower-case symbol")),
        };
        tags.insert(type_name.clone(), tag);
        if let Some(release) = optional(specification, "release") {
            releases.insert(
                type_name,
                string(release, origin, "handle release")?.to_owned(),
            );
        }
    }
    Ok((tags, releases))
}

fn parse_callbacks(
    form: &Form,
    origin: &str,
) -> Result<HashMap<String, ExtensionCallback>, String> {
    let mut callbacks = HashMap::new();
    for (name, specification) in map(form, origin, "callbacks")? {
        let name = string(name, origin, "callback name")?.to_owned();
        let entries = map(specification, origin, "callback specification")?;
        reject_unknown(entries, &["args", "returns", "reentrant"], origin, "callback")?;
        let arguments = wire_vector(required(entries, "args", origin)?, origin, "callback args")?;
        let returns = wire_type(
            required(entries, "returns", origin)?,
            origin,
            "callback returns",
        )?;
        if matches!(optional(entries, "reentrant"), Some(Form::Bool(true))) {
            return Err(malformed(
                origin,
                format!("callback {name} cannot be reentrant"),
            ));
        }
        if optional(entries, "reentrant").is_some_and(|form| !matches!(form, Form::Bool(_))) {
            return Err(malformed(origin, "callback reentrant must be boolean"));
        }
        if callbacks
            .insert(name.clone(), ExtensionCallback { arguments, returns })
            .is_some()
        {
            return Err(malformed(origin, format!("duplicate callback {name}")));
        }
    }
    Ok(callbacks)
}

fn valid_identity(value: &str) -> bool {
    value
        .split_once("/")
        .is_some_and(|(owner, name)| valid_component(owner) && valid_tag(name))
}

fn valid_namespace(value: &str) -> bool {
    value.contains('.') && value.split('.').all(valid_component)
}

fn valid_tag(value: &str) -> bool {
    value.split('.').all(valid_component)
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn map<'a>(form: &'a Form, origin: &str, field: &str) -> Result<&'a [(Form, Form)], String> {
    match form {
        Form::Map(entries) => Ok(entries),
        _ => Err(malformed(origin, format!("{field} must be a map"))),
    }
}

fn vector<'a>(form: &'a Form, origin: &str, field: &str) -> Result<&'a [Form], String> {
    match form {
        Form::Vector(values) => Ok(values),
        _ => Err(malformed(origin, format!("{field} must be a vector"))),
    }
}

fn string<'a>(form: &'a Form, origin: &str, field: &str) -> Result<&'a str, String> {
    match form {
        Form::String(value) if !value.is_empty() => Ok(value),
        _ => Err(malformed(
            origin,
            format!("{field} must be a non-empty string"),
        )),
    }
}

fn keyword<'a>(form: &'a Form, origin: &str, field: &str) -> Result<&'a str, String> {
    match form {
        Form::Keyword(value) => Ok(value),
        _ => Err(malformed(origin, format!("{field} must be a keyword"))),
    }
}

fn keyword_vector(form: &Form, origin: &str, field: &str) -> Result<Vec<String>, String> {
    vector(form, origin, field)?
        .iter()
        .map(|form| keyword(form, origin, field).map(str::to_owned))
        .collect()
}

fn wire_vector(form: &Form, origin: &str, field: &str) -> Result<Vec<String>, String> {
    vector(form, origin, field)?
        .iter()
        .map(|form| wire_type(form, origin, field))
        .collect()
}

fn wire_type(form: &Form, origin: &str, field: &str) -> Result<String, String> {
    match form {
        Form::Keyword(value) => Ok(value.clone()),
        Form::Vector(_) => Ok(form.to_string()),
        _ => Err(malformed(origin, format!("{field} must be a type form"))),
    }
}

fn key(form: &Form) -> Option<&str> {
    match form {
        Form::Keyword(value) | Form::Symbol(value) | Form::String(value) => Some(value),
        _ => None,
    }
}

fn required<'a>(entries: &'a [(Form, Form)], name: &str, origin: &str) -> Result<&'a Form, String> {
    optional(entries, name)
        .ok_or_else(|| malformed(origin, format!("missing required field {name}")))
}

fn optional<'a>(entries: &'a [(Form, Form)], name: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find(|(candidate, _)| key(candidate) == Some(name))
        .map(|(_, value)| value)
}

fn named_string(entries: &[(Form, Form)], name: &str, origin: &str) -> Result<String, String> {
    string(required(entries, name, origin)?, origin, name).map(str::to_owned)
}

fn named_keyword(entries: &[(Form, Form)], name: &str, origin: &str) -> Result<String, String> {
    keyword(required(entries, name, origin)?, origin, name).map(str::to_owned)
}

fn reject_unknown(
    entries: &[(Form, Form)],
    allowed: &[&str],
    origin: &str,
    scope: &str,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (candidate, _) in entries {
        let Some(name) = key(candidate) else {
            return Err(malformed(origin, format!("{scope} keys must be named")));
        };
        if !allowed.contains(&name) {
            return Err(malformed(origin, format!("unknown {scope} field: {name}")));
        }
        if !seen.insert(name) {
            return Err(malformed(
                origin,
                format!("duplicate {scope} field: {name}"),
            ));
        }
    }
    Ok(())
}

fn malformed(origin: &str, message: impl AsRef<str>) -> String {
    format!("extension/malformed {origin}: {}", message.as_ref())
}

#[cfg(test)]
mod tests {
    use super::{ExtensionManifest, WasmAbi};

    const MANIFEST: &str = r#"
      {:namespace "crypto.hash"
       :identity "hara/crypto.hash"
       :version "0.1.0"
       :provider :wasm
       :module "hash.wasm"
       :abi :hta.v1
       :exports {"digest" {:args [:bytes] :returns :bytes :async true}}
       :host-calls {"crypto.random" ["fill"]}
       :handles {"digest" {:tag crypto}}
       :capabilities [:random]}"#;

    const ALIASED_MANIFEST: &str = r#"
      {:namespace "math.scalar"
       :version "0.1.0"
       :provider :wasm
       :module "math.wasm"
       :abi :core.v1
       :exports {"sum" {:wasm/export "add_i64"
                         :args [:i64 :i64]
                         :returns :i64}}
       :capabilities []}"#;

    const MEMORY_MANIFEST: &str = r#"
      {:namespace "codec.echo"
       :version "0.1.0"
       :provider :wasm
       :module "echo.wasm"
       :abi :memory.v1
       :exports {"echo" {:wasm/export "echo_bytes"
                          :args [:bytes]
                          :returns :bytes}}
       :capabilities []}"#;

    #[test]
    fn parses_the_wasm_manifest_contract() {
        let manifest = ExtensionManifest::parse(MANIFEST, "fixture").unwrap();
        assert_eq!(manifest.namespace, "crypto.hash");
        assert_eq!(manifest.identity.as_deref(), Some("hara/crypto.hash"));
        assert_eq!(manifest.abi, WasmAbi::HtaV1);
        assert!(manifest.exports[0].1.asynchronous);
        assert_eq!(manifest.exports[0].1.raw_name("digest"), "digest");
        assert_eq!(manifest.handle_tags["digest"], "crypto");
        assert!(manifest.permits_host_call("crypto.random", "fill"));
    }

    #[test]
    fn parses_memory_v1_manifests_without_downgrading_the_abi() {
        let manifest = ExtensionManifest::parse(MEMORY_MANIFEST, "fixture").unwrap();
        assert_eq!(manifest.abi, WasmAbi::MemoryV1);
        assert_eq!(manifest.exports[0].1.raw_name("echo"), "echo_bytes");
    }

    #[test]
    fn preserves_public_and_raw_export_names() {
        let manifest = ExtensionManifest::parse(ALIASED_MANIFEST, "fixture").unwrap();
        assert_eq!(manifest.exports[0].0, "sum");
        assert_eq!(manifest.exports[0].1.raw_name("sum"), "add_i64");
    }

    #[test]
    fn rejects_non_wasm_unknown_duplicate_unsafe_and_unsupported_manifests() {
        for source in [
            MANIFEST.replace(":wasm", ":pod"),
            MANIFEST.replace(":capabilities [:random]", ":capabilities [] :extra true"),
            MANIFEST.replace(":version \"0.1.0\"", ":version \"0.1.0\" :version \"2\""),
            MANIFEST.replace("hash.wasm", "../hash.wasm"),
            MANIFEST.replace(":hta.v1", ":unknown-v1"),
        ] {
            assert!(ExtensionManifest::parse(&source, "fixture")
                .unwrap_err()
                .starts_with("extension/malformed fixture:"));
        }
    }

    #[test]
    fn hta_targets_name_provider_implementations() {
        let source = r#"
{:namespace "demo.hta"
 :version "1"
 :provider :hta
 :abi :hta.v1
 :targets {:node {:provider "node/provider.mjs" :runtime :process}
           :browser {:provider "browser/provider.mjs" :runtime :web-worker}}
 :exports {"open" {:args [] :returns :value}}
 :capabilities []}"#;
        let manifest = ExtensionManifest::parse(source, "fixture").unwrap();
        assert_eq!(manifest.targets["browser"].provider, "browser/provider.mjs");
        assert!(ExtensionManifest::parse(
            &source.replace(":provider \"browser/provider.mjs\"", ":module \"browser/worker.mjs\""),
            "fixture"
        )
        .is_err());
    }
}
