//! Project manifest discovery and editing for the native CLI.
//!
//! `project.edn` is data, never evaluator input.  Keeping this model separate
//! from `Runtime` makes command behaviour portable to other Hara hosts.

use crate::kernel::{parse, parse_forms, Form};
use crate::Runtime;
use semver::{Version, VersionReq};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[path = "project/npm.rs"]
mod npm;

const REQUIRED: &[&str] = &[
    "hara/type",
    "hara/version",
    "project/id",
    "project/version",
    "project/source-paths",
    "project/test-paths",
    "project/extension-paths",
    "project/capabilities",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub id: String,
    pub version: Version,
    /// Effective native-Rust paths (shared paths followed by :rust additions).
    pub source_paths: Vec<PathBuf>,
    pub test_paths: Vec<PathBuf>,
    pub extension_paths: Vec<PathBuf>,
    pub shared_source_paths: Vec<PathBuf>,
    pub shared_test_paths: Vec<PathBuf>,
    pub shared_extension_paths: Vec<PathBuf>,
    pub runtime_profiles: BTreeMap<String, RuntimeProfile>,
    pub active_runtime: String,
    pub native_source_paths: Vec<PathBuf>,
    pub runtime_target_path: Option<PathBuf>,
    pub maven_dependencies: BTreeMap<String, String>,
    pub npm_dependencies: BTreeMap<String, NpmWasmDependency>,
    pub native_imports: BTreeMap<String, WasmNativeImport>,
    pub capabilities: Vec<String>,
    pub artifact_paths: Vec<PathBuf>,
    pub archive_root: Option<PathBuf>,
    /// Whether the intentionally portable workspace declaration is a package
    /// resource.  This never includes a live Studio workspace or cache.
    pub package_workspace: bool,
    /// Semantic package coordinate selected from the project's package
    /// profile. This is independent from the namespace coordinates it owns.
    pub package_name: Option<String>,
    /// Optional Foundation-compatible package profile used to select source
    /// namespaces while building a package archive.
    pub package_profile: Option<PathBuf>,
    /// Optional explicit HAL files to include when building a package.
    ///
    /// This keeps package projects rooted next to canonical sources without
    /// recursively archiving runtime-specific siblings.
    pub source_files: Option<Vec<PathBuf>>,
    pub main: Option<String>,
    pub default_profile: Option<String>,
    pub profiles: BTreeMap<String, ProjectProfile>,
    /// Effective native-Rust Hara dependencies.
    pub dependencies: BTreeMap<String, String>,
    pub shared_dependencies: BTreeMap<String, String>,
    pub extensions: BTreeMap<String, Form>,
    /// Project-local command aliases.  Values are argv prefixes, never shell
    /// expressions; callers append their own arguments after expansion.
    pub aliases: BTreeMap<String, Vec<String>>,
    pub recipe: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectProfile {
    pub language: String,
    pub main: Option<String>,
    pub options: Form,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuntimeProfile {
    pub source_paths: Vec<PathBuf>,
    pub test_paths: Vec<PathBuf>,
    pub extension_paths: Vec<PathBuf>,
    pub native_source_paths: Vec<PathBuf>,
    pub target_path: Option<PathBuf>,
    pub hara_dependencies: BTreeMap<String, String>,
    pub maven_dependencies: BTreeMap<String, String>,
    pub npm_dependencies: BTreeMap<String, NpmWasmDependency>,
    pub native_imports: BTreeMap<String, WasmNativeImport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmWasmDependency {
    pub version: Version,
    pub integrity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmNativeImport {
    pub package: String,
    pub module: PathBuf,
    pub abi: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRuntimeProfile {
    pub runtime: String,
    pub source_paths: Vec<PathBuf>,
    pub test_paths: Vec<PathBuf>,
    pub extension_paths: Vec<PathBuf>,
    pub native_source_paths: Vec<PathBuf>,
    pub target_path: Option<PathBuf>,
    pub hara_dependencies: BTreeMap<String, String>,
    pub maven_dependencies: BTreeMap<String, String>,
    pub npm_dependencies: BTreeMap<String, NpmWasmDependency>,
    pub native_imports: BTreeMap<String, WasmNativeImport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProfile {
    pub name: String,
    pub language: String,
    pub main: String,
    pub options: Form,
}

impl Project {
    /// Resolves the shared project declaration with one host runtime overlay.
    pub fn resolve_runtime_profile(&self, runtime: &str) -> Result<ResolvedRuntimeProfile, String> {
        resolve_runtime_profile_values(
            runtime,
            &self.shared_source_paths,
            &self.shared_test_paths,
            &self.shared_extension_paths,
            &self.shared_dependencies,
            &self.runtime_profiles,
        )
    }

    /// Resolves a named runnable target without assigning any meaning to its
    /// language or options. Language hosts such as Hoplite own that policy.
    pub fn resolve_profile(
        &self,
        requested: Option<&str>,
    ) -> Result<Option<ResolvedProfile>, String> {
        if self.profiles.is_empty() {
            if requested.is_some() {
                return Err("project.edn does not declare :project/profiles".into());
            }
            return Ok(None);
        }
        let name = requested
            .map(str::to_owned)
            .or_else(|| self.default_profile.clone())
            .ok_or("project.edn requires :project/default-profile or an explicit profile")?;
        let profile = self
            .profiles
            .get(&name)
            .ok_or_else(|| format!("project.edn has no profile {name:?}"))?;
        let main = profile
            .main
            .clone()
            .or_else(|| self.main.clone())
            .ok_or_else(|| format!("project profile {name:?} has no main value"))?;
        Ok(Some(ResolvedProfile {
            name,
            language: profile.language.clone(),
            main,
            options: profile.options.clone(),
        }))
    }
}

pub fn discover(start: &Path) -> Result<Project, String> {
    let initial = if start.is_file() {
        start
            .parent()
            .ok_or_else(|| format!("cannot determine project root for {}", start.display()))?
    } else {
        start
    };
    let mut current = initial
        .canonicalize()
        .unwrap_or_else(|_| initial.to_path_buf());
    loop {
        let manifest = current.join("project.edn");
        if manifest.is_file() {
            return read(&manifest);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return Err(format!("no project.edn found above {}", initial.display())),
        }
    }
}

pub fn read(input: &Path) -> Result<Project, String> {
    let manifest_path = if input.is_dir() {
        input.join("project.edn")
    } else {
        input.to_path_buf()
    };
    let root = manifest_path
        .parent()
        .ok_or_else(|| {
            format!(
                "cannot determine project root for {}",
                manifest_path.display()
            )
        })?
        .to_path_buf();
    let source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let form = parse(&source).map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    let entries = map(&form, "project.edn must be an EDN map")?;
    reject_legacy_runtime_keys(entries)?;
    for key in REQUIRED {
        if lookup(entries, key).is_none() {
            return Err(format!("project.edn missing required key :{key}"));
        }
    }
    if !matches!(lookup(entries, "hara/type"), Some(Form::Keyword(value)) if value == "project") {
        return Err("project.edn :hara/type must be :project".into());
    }
    let id = scalar(
        lookup(entries, "project/id").unwrap(),
        "project.edn :project/id",
    )?;
    let version_text = string(
        lookup(entries, "project/version").unwrap(),
        "project.edn :project/version",
    )?;
    let version = Version::parse(&version_text)
        .map_err(|error| format!("project.edn :project/version is not SemVer: {error}"))?;
    let shared_source_paths = paths(
        lookup(entries, "project/source-paths").unwrap(),
        "project/source-paths",
    )?;
    let shared_test_paths = paths(
        lookup(entries, "project/test-paths").unwrap(),
        "project/test-paths",
    )?;
    let shared_extension_paths = paths(
        lookup(entries, "project/extension-paths").unwrap(),
        "project/extension-paths",
    )?;
    let capabilities = capability_set(
        lookup(entries, "project/capabilities").unwrap(),
        "project.edn :project/capabilities",
    )?;
    let artifact_paths = lookup(entries, "project/artifact-paths")
        .map(|value| paths(value, "project/artifact-paths"))
        .transpose()?
        .unwrap_or_default();
    let archive_root = lookup(entries, "project/archive-root")
        .map(|value| {
            relative_path(
                &string(value, "project/archive-root")?,
                "project/archive-root",
            )
        })
        .transpose()?;
    let package_config = lookup(entries, "project/package")
        .map(package_config)
        .transpose()?
        .unwrap_or_default();
    let source_files = lookup(entries, "project/source-files")
        .map(|value| paths(value, "project/source-files"))
        .transpose()?;
    let main = lookup(entries, "project/main")
        .map(|value| scalar(value, "project.edn :project/main"))
        .transpose()?;
    let default_profile = lookup(entries, "project/default-profile")
        .map(|value| identifier(value, "project.edn :project/default-profile"))
        .transpose()?;
    let profiles = lookup(entries, "project/profiles")
        .map(project_profiles)
        .transpose()?
        .unwrap_or_default();
    if let Some(default) = &default_profile {
        if !profiles.contains_key(default) {
            return Err(format!(
                "project.edn :project/default-profile {default:?} is not declared in :project/profiles"
            ));
        }
    }
    let shared_dependencies = lookup(entries, "project/dependencies")
        .map(dependencies)
        .transpose()?
        .unwrap_or_default();
    let runtime_profiles = lookup(entries, "project/runtime-profiles")
        .map(runtime_profiles)
        .transpose()?
        .unwrap_or_default();
    let active = resolve_runtime_profile_values(
        "rust",
        &shared_source_paths,
        &shared_test_paths,
        &shared_extension_paths,
        &shared_dependencies,
        &runtime_profiles,
    )?;
    let source_paths = active.source_paths.clone();
    let test_paths = active.test_paths.clone();
    let extension_paths = active.extension_paths.clone();
    let dependencies = active.hara_dependencies.clone();
    let native_source_paths = active.native_source_paths.clone();
    let runtime_target_path = active.target_path.clone();
    let maven_dependencies = active.maven_dependencies.clone();
    let npm_dependencies = active.npm_dependencies.clone();
    let native_imports = active.native_imports.clone();
    let extensions = lookup(entries, "project/extensions")
        .map(extension_declarations)
        .transpose()?
        .unwrap_or_default();
    let aliases = lookup(entries, "project/aliases")
        .map(project_aliases)
        .transpose()?
        .unwrap_or_default();
    let recipe = lookup(entries, "project/recipe")
        .map(|value| relative_path(&string(value, "project/recipe")?, "project/recipe"))
        .transpose()?;
    if let Some(path) = &recipe {
        if !root.join(path).is_file() {
            return Err(format!(
                "project.edn :project/recipe does not exist: {}",
                path.display()
            ));
        }
    }
    Ok(Project {
        root,
        manifest_path,
        id,
        version,
        source_paths,
        test_paths,
        extension_paths,
        shared_source_paths,
        shared_test_paths,
        shared_extension_paths,
        runtime_profiles,
        active_runtime: "rust".into(),
        native_source_paths,
        runtime_target_path,
        maven_dependencies,
        npm_dependencies,
        native_imports,
        capabilities,
        artifact_paths,
        archive_root,
        package_workspace: package_config.workspace,
        package_name: package_config.name,
        package_profile: package_config.profile,
        source_files,
        main,
        default_profile,
        profiles,
        dependencies,
        shared_dependencies,
        extensions,
        aliases,
        recipe,
    })
}

fn extension_declarations(form: &Form) -> Result<BTreeMap<String, Form>, String> {
    let Form::Map(entries) = form else {
        return Err("project.edn :project/extensions must be a map".into());
    };
    entries
        .iter()
        .map(|(namespace, declaration)| {
            let namespace = scalar(namespace, "project extension namespace")?;
            if !matches!(declaration, Form::Map(_)) {
                return Err(format!(
                    "project extension {namespace} declaration must be a map"
                ));
            }
            Ok((namespace, declaration.clone()))
        })
        .collect()
}

pub fn new_app(destination: &Path, name: &str) -> Result<Project, String> {
    if !valid_name(name) {
        return Err(
            "project name must contain only lowercase letters, numbers, and hyphens".into(),
        );
    }
    if destination.exists() {
        return Err(format!(
            "destination already exists: {}",
            destination.display()
        ));
    }
    let namespace = name.replace('-', "_");
    fs::create_dir_all(destination.join("src").join(&namespace)).map_err(io)?;
    fs::create_dir_all(destination.join("test").join(&namespace)).map_err(io)?;
    fs::create_dir_all(destination.join("extensions")).map_err(io)?;
    fs::write(destination.join("project.edn"), format!(
        "{{:hara/type :project\n :hara/version \"1.0.0\"\n :project/id {name}\n :project/version \"0.1.0\"\n :project/source-paths [\"src\"]\n :project/test-paths [\"test\"]\n :project/extension-paths [\"extensions\"]\n :project/main {namespace}.main\n :project/capabilities #{{}}\n :project/dependencies {{}}}}\n"
    )).map_err(io)?;
    fs::write(
        destination.join("workspace.edn"),
        "{:hara/type :workspace :hara/version \"1.0.0\"}\n",
    )
    .map_err(io)?;
    fs::write(
        destination.join("src").join(&namespace).join("main.hal"),
        format!("(ns {namespace}.main)\n\n(defn main []\n  \"Hello from {name}\")\n\n(main)\n"),
    )
    .map_err(io)?;
    fs::write(
        destination
            .join("test")
            .join(&namespace)
            .join("main_test.hal"),
        format!(
            "(ns {namespace}.main-test)\n\n[(test-check \"starter project runs\" true true)]\n"
        ),
    )
    .map_err(io)?;
    read(&destination.join("project.edn"))
}

pub fn set_dependency(
    project: &Project,
    coordinate: &str,
    version: Option<&str>,
) -> Result<(), String> {
    validate_coordinate(coordinate)?;
    if let Some(version) = version {
        VersionReq::parse(version)
            .map_err(|error| format!("invalid dependency range {version}: {error}"))?;
    }
    let source = fs::read_to_string(&project.manifest_path).map_err(io)?;
    let mut form =
        parse(&source).map_err(|error| format!("{}: {error}", project.manifest_path.display()))?;
    let entries = map_mut(&mut form, "project.edn must be an EDN map")?;
    let dependency_index = entries
        .iter()
        .position(|(key, _)| key_name(key).as_deref() == Some("project/dependencies"));
    let dependency_form = dependency_index.map(|index| &mut entries[index].1);
    let deps = match dependency_form {
        Some(Form::Map(entries)) => entries,
        Some(_) => return Err("project.edn :project/dependencies must be an EDN map".into()),
        None => {
            entries.push((
                Form::Keyword("project/dependencies".into()),
                Form::Map(Vec::new()),
            ));
            match &mut entries.last_mut().unwrap().1 {
                Form::Map(entries) => entries,
                _ => unreachable!(),
            }
        }
    };
    if let Some(index) = deps.iter().position(|(key, _)| {
        scalar(key, "dependency coordinate").ok().as_deref() == Some(coordinate)
    }) {
        if let Some(version) = version {
            deps[index].1 = Form::Map(vec![(
                Form::Keyword("version".into()),
                Form::String(version.into()),
            )]);
        } else {
            deps.remove(index);
        }
    } else if let Some(version) = version {
        deps.push((
            Form::String(coordinate.into()),
            Form::Map(vec![(
                Form::Keyword("version".into()),
                Form::String(version.into()),
            )]),
        ));
    }
    deps.sort_by(|left, right| left.0.to_string().cmp(&right.0.to_string()));
    fs::write(&project.manifest_path, format!("{form}\n")).map_err(io)
}

pub fn files_in(root: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut output = Vec::new();
    for relative in paths {
        collect_hal(&root.join(relative), &mut output)?;
    }
    output.sort();
    Ok(output)
}

#[path = "project/resources.rs"]
mod resources;
pub use resources::source_resources;
pub use resources::{source_catalog, source_catalogs, SourceCatalog};

/// Registers namespaces from the automatically selected native Rust profile.
pub fn register_sources(project: &Project, runtime: &mut Runtime) -> Result<(), String> {
    for (namespace, source) in source_resources(project)? {
        runtime.register_resource(&namespace, &source);
    }
    Ok(())
}

/// Installs direct WASM imports exclusively from the verified project lock and
/// content-addressed cache. Runtime evaluation never invokes npm or the network.
#[cfg(not(target_arch = "wasm32"))]
pub fn register_native_imports(project: &Project, runtime: &mut Runtime) -> Result<(), String> {
    if project.native_imports.is_empty() {
        Ok(())
    } else {
        npm::install(project, runtime)
    }
}

pub(crate) fn native_archive_entries(project: &Project) -> Result<Vec<PathBuf>, String> {
    if project.native_imports.is_empty() {
        Ok(Vec::new())
    } else {
        npm::archive_entries(project)
    }
}

pub fn main_file(project: &Project) -> Result<PathBuf, String> {
    let namespace = project
        .main
        .as_ref()
        .ok_or_else(|| "project.edn is missing :project/main".to_owned())?;
    let relative = format!("{}.hal", namespace.replace('.', "/").replace('-', "_"));
    for source in &project.source_paths {
        let candidate = project.root.join(source).join(&relative);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "cannot find :project/main {namespace} in :project/source-paths"
    ))
}

fn declared_namespace(source: &str) -> Result<Option<String>, String> {
    Ok(parse_forms(source)?
        .into_iter()
        .find_map(declared_namespace_form))
}

fn declared_namespace_form(form: Form) -> Option<String> {
    match form {
        Form::Metadata(_, value) => declared_namespace_form(*value),
        Form::List(values) if matches!(values.first(), Some(Form::Symbol(head)) if head == "ns" || head == "ns+") => {
            match values.get(1) {
                Some(Form::Symbol(namespace)) if !namespace.contains('/') => {
                    Some(namespace.clone())
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Creates or validates the lockfile for graphs that need no remote packages.
/// Remote graphs deliberately stop here until the reviewed registry and
/// identity clients can provide the required signed release metadata.
pub fn sync_lock(project: &Project, mode: LockMode) -> Result<PathBuf, String> {
    let lock = project.root.join("project.lock.edn");
    if !project.dependencies.is_empty() {
        return Err(format!(
            "project sync requires the reviewed registry client to resolve {} declared dependencies",
            project.dependencies.len()
        ));
    }
    if !project.npm_dependencies.is_empty() || !project.native_imports.is_empty() {
        return npm::sync(project, mode, &lock);
    }
    match mode {
        LockMode::Locked | LockMode::Frozen if !lock.is_file() => {
            return Err(format!(
                "{} requires an existing project.lock.edn",
                mode.flag()
            ));
        }
        LockMode::Locked | LockMode::Frozen => validate_empty_lock(&lock)?,
        LockMode::Default | LockMode::Offline => {
            fs::write(&lock, "{:lock/format \"0.0.0-alpha\" :packages {}}\n")
                .map_err(|error| format!("cannot write {}: {error}", lock.display()))?;
        }
    }
    Ok(lock)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Default,
    Offline,
    Locked,
    Frozen,
}

impl LockMode {
    pub fn flag(self) -> &'static str {
        match self {
            Self::Default => "sync",
            Self::Offline => "--offline",
            Self::Locked => "--locked",
            Self::Frozen => "--frozen",
        }
    }
}

fn collect_hal(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if editor_artifact(&path) {
            continue;
        }
        if path.is_dir() {
            collect_hal(&path, output)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("hal") {
            output.push(path);
        }
    }
    Ok(())
}

fn editor_artifact(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            name.starts_with(".#") || (name.starts_with('#') && name.ends_with('#'))
        })
}

fn validate_empty_lock(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let form = parse(&source).map_err(|error| format!("{}: {error}", path.display()))?;
    let entries = map(&form, "project.lock.edn must be an EDN map")?;
    if matches!(lookup(entries, "lock/format"), Some(Form::String(version)) if version == "0.0.0-alpha")
        && matches!(lookup(entries, "packages"), Some(Form::Map(entries)) if entries.is_empty())
    {
        Ok(())
    } else {
        Err(format!(
            "{} is not a lockfile written by this CLI",
            path.display()
        ))
    }
}

fn map<'a>(form: &'a Form, message: &str) -> Result<&'a Vec<(Form, Form)>, String> {
    if let Form::Map(entries) = form {
        Ok(entries)
    } else {
        Err(message.into())
    }
}
fn map_mut<'a>(form: &'a mut Form, message: &str) -> Result<&'a mut Vec<(Form, Form)>, String> {
    if let Form::Map(entries) = form {
        Ok(entries)
    } else {
        Err(message.into())
    }
}
fn key_name(key: &Form) -> Option<String> {
    match key {
        Form::Keyword(value) => Some(value.clone()),
        _ => None,
    }
}
fn lookup<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find(|(candidate, _)| key_name(candidate).as_deref() == Some(key))
        .map(|(_, value)| value)
}
fn scalar(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) | Form::Symbol(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a string or symbol")),
    }
}
fn identifier(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::Keyword(value) | Form::String(value) | Form::Symbol(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a keyword, string, or symbol")),
    }
}

fn capability_set(form: &Form, label: &str) -> Result<Vec<String>, String> {
    let Form::Set(values) = form else {
        return Err(format!("{label} must be an EDN set"));
    };
    let mut output = values
        .iter()
        .map(|value| identifier(value, label))
        .collect::<Result<Vec<_>, _>>()?;
    output.sort();
    output.dedup();
    Ok(output)
}

fn reject_legacy_runtime_keys(entries: &[(Form, Form)]) -> Result<(), String> {
    for (key, replacement) in [
        (
            "jvm/source-paths",
            ":project/runtime-profiles :jvm :runtime/native-source-paths",
        ),
        (
            "jvm/dependencies",
            ":project/runtime-profiles :jvm :runtime/dependencies :maven",
        ),
        (
            "jvm/target-path",
            ":project/runtime-profiles :jvm :runtime/target-path",
        ),
    ] {
        if lookup(entries, key).is_some() {
            return Err(format!(
                "project.edn :{key} is no longer supported; use {replacement}"
            ));
        }
    }
    Ok(())
}

fn runtime_profiles(form: &Form) -> Result<BTreeMap<String, RuntimeProfile>, String> {
    let mut output = BTreeMap::new();
    for (key, value) in map(
        form,
        "project.edn :project/runtime-profiles must be an EDN map",
    )? {
        let runtime = identifier(key, "runtime profile name")?;
        if runtime != "jvm" && runtime != "rust" {
            return Err(format!("unsupported project runtime profile {runtime:?}"));
        }
        let entries = map(value, "runtime profile must be an EDN map")?;
        let source_paths = lookup(entries, "runtime/source-paths")
            .map(|value| paths(value, "runtime/source-paths"))
            .transpose()?
            .unwrap_or_default();
        let test_paths = lookup(entries, "runtime/test-paths")
            .map(|value| paths(value, "runtime/test-paths"))
            .transpose()?
            .unwrap_or_default();
        let extension_paths = lookup(entries, "runtime/extension-paths")
            .map(|value| paths(value, "runtime/extension-paths"))
            .transpose()?
            .unwrap_or_default();
        let native_source_paths = lookup(entries, "runtime/native-source-paths")
            .map(|value| paths(value, "runtime/native-source-paths"))
            .transpose()?
            .unwrap_or_default();
        let target_path = lookup(entries, "runtime/target-path")
            .map(|value| {
                relative_path(
                    &string(value, "runtime/target-path")?,
                    "runtime/target-path",
                )
            })
            .transpose()?;
        let (hara_dependencies, maven_dependencies, npm_dependencies) =
            match lookup(entries, "runtime/dependencies") {
                None => (BTreeMap::new(), BTreeMap::new(), BTreeMap::new()),
                Some(value) => {
                    let groups = map(value, "runtime :runtime/dependencies must be an EDN map")?;
                    let hara = lookup(groups, "hara")
                        .map(dependencies)
                        .transpose()?
                        .unwrap_or_default();
                    let maven = lookup(groups, "maven")
                        .map(maven_dependencies)
                        .transpose()?
                        .unwrap_or_default();
                    let npm = lookup(groups, "npm")
                        .map(npm_wasm_dependencies)
                        .transpose()?
                        .unwrap_or_default();
                    (hara, maven, npm)
                }
            };
        let native_imports = lookup(entries, "runtime/imports")
            .map(|value| wasm_native_imports(value, &npm_dependencies))
            .transpose()?
            .unwrap_or_default();
        let profile = RuntimeProfile {
            source_paths,
            test_paths,
            extension_paths,
            native_source_paths,
            target_path,
            hara_dependencies,
            maven_dependencies,
            npm_dependencies,
            native_imports,
        };
        if output.insert(runtime.clone(), profile).is_some() {
            return Err(format!("duplicate project runtime profile {runtime:?}"));
        }
    }
    Ok(output)
}

fn resolve_runtime_profile_values(
    runtime: &str,
    shared_source_paths: &[PathBuf],
    shared_test_paths: &[PathBuf],
    shared_extension_paths: &[PathBuf],
    shared_dependencies: &BTreeMap<String, String>,
    runtime_profiles: &BTreeMap<String, RuntimeProfile>,
) -> Result<ResolvedRuntimeProfile, String> {
    if runtime != "jvm" && runtime != "rust" {
        return Err(format!("unsupported project runtime profile {runtime:?}"));
    }
    let profile = runtime_profiles.get(runtime).cloned().unwrap_or_default();
    let mut hara_dependencies = shared_dependencies.clone();
    for (coordinate, requirement) in &profile.hara_dependencies {
        if let Some(shared) = hara_dependencies.get(coordinate) {
            if shared != requirement {
                return Err(format!(
                    "conflicting Hara dependency requirements for {coordinate} in :{runtime}: {shared:?} and {requirement:?}"
                ));
            }
        }
        hara_dependencies.insert(coordinate.clone(), requirement.clone());
    }
    let mut source_paths = shared_source_paths.to_vec();
    source_paths.extend(profile.source_paths.iter().cloned());
    let mut test_paths = shared_test_paths.to_vec();
    test_paths.extend(profile.test_paths.iter().cloned());
    let mut extension_paths = shared_extension_paths.to_vec();
    extension_paths.extend(profile.extension_paths.iter().cloned());
    Ok(ResolvedRuntimeProfile {
        runtime: runtime.into(),
        source_paths,
        test_paths,
        extension_paths,
        native_source_paths: profile.native_source_paths,
        target_path: profile.target_path,
        hara_dependencies,
        maven_dependencies: profile.maven_dependencies,
        npm_dependencies: profile.npm_dependencies,
        native_imports: profile.native_imports,
    })
}

fn npm_wasm_dependencies(form: &Form) -> Result<BTreeMap<String, NpmWasmDependency>, String> {
    map(form, "runtime npm dependencies must be an EDN map")?
        .iter()
        .map(|(coordinate, declaration)| {
            let coordinate = string(coordinate, "npm package name")?;
            let entries = map(declaration, "npm dependency declaration must be an EDN map")?;
            for (key, _) in entries {
                let key = identifier(key, "npm dependency field")?;
                if key != "version" && key != "integrity" {
                    return Err(format!("unsupported npm dependency field :{key}"));
                }
            }
            let version = string(
                lookup(entries, "version").ok_or("npm dependency requires :version")?,
                "npm dependency :version",
            )?;
            let version = Version::parse(&version)
                .map_err(|_| "npm dependency :version must be an exact SemVer")?;
            let integrity = string(
                lookup(entries, "integrity").ok_or("npm dependency requires :integrity")?,
                "npm dependency :integrity",
            )?;
            let payload = integrity
                .strip_prefix("sha512-")
                .ok_or("npm dependency :integrity must use sha512 SRI")?;
            if payload.len() < 16
                || !payload
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
            {
                return Err("npm dependency :integrity contains invalid sha512 SRI data".into());
            }
            Ok((coordinate, NpmWasmDependency { version, integrity }))
        })
        .collect()
}

fn wasm_native_imports(
    form: &Form,
    dependencies: &BTreeMap<String, NpmWasmDependency>,
) -> Result<BTreeMap<String, WasmNativeImport>, String> {
    map(form, "runtime imports must be an EDN map")?
        .iter()
        .map(|(logical, declaration)| {
            let logical = identifier(logical, "runtime import name")?;
            let entries = map(declaration, "runtime import declaration must be an EDN map")?;
            for (key, _) in entries {
                let key = identifier(key, "runtime import field")?;
                if !matches!(key.as_str(), "package" | "module" | "abi") {
                    return Err(format!("unsupported runtime import field :{key}"));
                }
            }
            let package = string(
                lookup(entries, "package").ok_or("runtime import requires :package")?,
                "runtime import :package",
            )?;
            if !dependencies.contains_key(&package) {
                return Err(format!(
                    "runtime import {logical:?} uses undeclared npm package {package:?}"
                ));
            }
            let module = relative_path(
                &string(
                    lookup(entries, "module").ok_or("runtime import requires :module")?,
                    "runtime import :module",
                )?,
                "runtime import :module",
            )?;
            if module.extension().and_then(|value| value.to_str()) != Some("wasm") {
                return Err("runtime import :module must select a .wasm file".into());
            }
            let abi = identifier(
                lookup(entries, "abi").ok_or("runtime import requires :abi")?,
                "runtime import :abi",
            )?;
            if abi != "core.v1" {
                return Err(format!("runtime import uses unsupported ABI :{abi}"));
            }
            Ok((
                logical,
                WasmNativeImport {
                    package,
                    module,
                    abi,
                },
            ))
        })
        .collect()
}

fn maven_dependencies(form: &Form) -> Result<BTreeMap<String, String>, String> {
    let mut output = BTreeMap::new();
    for (key, value) in map(form, "runtime Maven dependencies must be an EDN map")? {
        let coordinate = scalar(key, "Maven dependency coordinate")?;
        let mut parts = coordinate.split('/');
        if !matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(group), Some(artifact), None) if !group.is_empty() && !artifact.is_empty()
        ) {
            return Err(format!(
                "invalid Maven dependency coordinate {coordinate:?}"
            ));
        }
        let declaration = map(value, "Maven dependency declaration must be an EDN map")?;
        let version = lookup(declaration, "version")
            .ok_or_else(|| format!("Maven dependency {coordinate} is missing :version"))
            .and_then(|value| string(value, "Maven dependency :version"))?;
        if version.is_empty()
            || version
                .chars()
                .any(|value| matches!(value, '[' | ']' | '(' | ')' | ',' | '*'))
        {
            return Err(format!(
                "Maven dependency {coordinate} requires an exact version"
            ));
        }
        if output.insert(coordinate.clone(), version).is_some() {
            return Err(format!("duplicate Maven dependency {coordinate}"));
        }
    }
    Ok(output)
}

fn project_profiles(form: &Form) -> Result<BTreeMap<String, ProjectProfile>, String> {
    let mut output = BTreeMap::new();
    for (key, value) in map(form, "project.edn :project/profiles must be an EDN map")? {
        let name = identifier(key, "project profile name")?;
        let entries = map(value, "project profile must be an EDN map")?;
        let language = lookup(entries, "profile/language")
            .ok_or_else(|| format!("project profile {name:?} is missing :profile/language"))
            .and_then(|value| identifier(value, "profile :profile/language"))?;
        let main = lookup(entries, "profile/main")
            .map(|value| scalar(value, "profile :profile/main"))
            .transpose()?;
        let options = lookup(entries, "profile/options")
            .cloned()
            .unwrap_or_else(|| Form::Map(Vec::new()));
        if !matches!(options, Form::Map(_)) {
            return Err(format!(
                "project profile {name:?} :profile/options must be an EDN map"
            ));
        }
        if output
            .insert(
                name.clone(),
                ProjectProfile {
                    language,
                    main,
                    options,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate project profile {name:?}"));
        }
    }
    Ok(output)
}

fn project_aliases(form: &Form) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut output = BTreeMap::new();
    for (key, value) in map(form, "project.edn :project/aliases must be an EDN map")? {
        let name = identifier(key, "project alias name")?;
        if name.is_empty() || name.contains('/') || name.starts_with('-') {
            return Err(format!("invalid project alias {name:?}"));
        }
        let Form::Vector(values) = value else {
            return Err(format!(
                "project alias {name:?} must be a vector of strings"
            ));
        };
        let argv = values
            .iter()
            .map(|value| string(value, &format!("project alias {name:?}")))
            .collect::<Result<Vec<_>, _>>()?;
        if argv.is_empty() || argv.iter().any(|value| value.is_empty()) {
            return Err(format!(
                "project alias {name:?} must contain command tokens"
            ));
        }
        if output.insert(name.clone(), argv).is_some() {
            return Err(format!("duplicate project alias {name:?}"));
        }
    }
    Ok(output)
}

/// Expands aliases without shell interpretation. Cycles are rejected rather
/// than silently consuming user arguments.
pub fn expand_aliases(project: &Project, argv: &[String]) -> Result<Vec<String>, String> {
    let mut output = argv.to_vec();
    let mut seen = BTreeMap::new();
    loop {
        let Some(name) = output.first().cloned() else {
            return Ok(output);
        };
        let Some(prefix) = project.aliases.get(&name) else {
            return Ok(output);
        };
        if seen.insert(name.clone(), true).is_some() {
            return Err(format!("project alias cycle detected at {name:?}"));
        }
        let mut expanded = prefix.clone();
        expanded.extend(output.into_iter().skip(1));
        output = expanded;
    }
}
fn string(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a string")),
    }
}
fn relative_path(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        Err(format!(
            "project.edn :{label} cannot escape the project root"
        ))
    } else {
        Ok(path)
    }
}
fn paths(form: &Form, label: &str) -> Result<Vec<PathBuf>, String> {
    match form {
        Form::Vector(values) => values
            .iter()
            .map(|value| relative_path(&string(value, &format!("project.edn :{label}"))?, label))
            .collect(),
        _ => Err(format!("project.edn :{label} must be a vector of strings")),
    }
}
#[derive(Default)]
struct PackageConfig {
    workspace: bool,
    name: Option<String>,
    profile: Option<PathBuf>,
}

fn package_config(form: &Form) -> Result<PackageConfig, String> {
    let entries = map(form, "project.edn :project/package must be an EDN map")?;
    let workspace = match lookup(entries, "workspace") {
        None | Some(Form::Bool(false)) => false,
        Some(Form::Bool(true)) => true,
        Some(_) => return Err("project.edn :project/package :workspace must be a boolean".into()),
    };
    let name = lookup(entries, "name")
        .map(|value| identifier(value, "project.edn :project/package :name"))
        .transpose()?;
    if name.as_deref().is_some_and(str::is_empty) {
        return Err("project.edn :project/package :name must be non-empty".into());
    }
    let profile = lookup(entries, "profile")
        .map(|value| {
            relative_path(
                &string(value, "project.edn :project/package :profile")?,
                "project/package/profile",
            )
        })
        .transpose()?;
    Ok(PackageConfig {
        workspace,
        name,
        profile,
    })
}
fn dependencies(form: &Form) -> Result<BTreeMap<String, String>, String> {
    let mut output = BTreeMap::new();
    for (key, value) in map(form, "project.edn :project/dependencies must be an EDN map")? {
        let coordinate = normalize_coordinate(&scalar(key, "dependency coordinate")?)?;
        let version = lookup(
            map(value, "dependency declaration must be an EDN map")?,
            "version",
        )
        .ok_or_else(|| format!("dependency {coordinate} is missing :version"))?;
        let version = string(version, "dependency :version")?;
        VersionReq::parse(&version)
            .map_err(|error| format!("invalid dependency range {version}: {error}"))?;
        output.insert(coordinate, version);
    }
    Ok(output)
}
pub fn normalize_coordinate(value: &str) -> Result<String, String> {
    let qualified = if let Some(package) = value.strip_prefix("official:") {
        format!("hara:{package}")
    } else if value.contains(':') {
        value.to_owned()
    } else {
        format!("hara:{value}")
    };
    let (tap, package) = qualified
        .split_once(':')
        .ok_or_else(|| format!("invalid package coordinate: {value}"))?;
    let mut parts = package.split('/');
    let valid = !tap.is_empty()
        && tap.chars().all(valid_coordinate_char)
        && matches!((parts.next(), parts.next(), parts.next()), (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() && owner.chars().all(valid_coordinate_char) && name.chars().all(valid_coordinate_char));
    if valid {
        Ok(qualified)
    } else {
        Err(format!("invalid package coordinate: {value}"))
    }
}
fn validate_coordinate(value: &str) -> Result<(), String> {
    normalize_coordinate(value).map(|_| ())
}
fn valid_coordinate_char(value: char) -> bool {
    value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, '-' | '_' | '.')
}
fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == '-')
}
fn io(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "project/tests.rs"]
mod tests;
