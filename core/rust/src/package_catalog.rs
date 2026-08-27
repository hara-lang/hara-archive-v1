use crate::kernel::{parse, Form};

/// A Foundation-compatible namespace selector. Package names are semantic
/// coordinates; selectors only describe which namespaces the coordinate owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSelectorMode {
    Base,
    Complete,
    Exclude(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSelector {
    pub namespace: String,
    pub mode: PackageSelectorMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageBundle {
    pub path: String,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDefinition {
    pub name: String,
    pub description: Option<String>,
    pub selectors: Vec<PackageSelector>,
    pub dependencies: Vec<String>,
    pub optional: Vec<String>,
    pub bundles: Vec<PackageBundle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    pub coordinate: String,
    pub name: Option<String>,
    pub version: String,
    pub tap: String,
    pub registry_commit: String,
    pub identity_revision: String,
    pub archive_sha256: String,
    pub namespaces: Vec<String>,
    pub dependencies: Vec<String>,
}

pub fn catalog_from_lock(source: &str) -> Result<Vec<LockedPackage>, String> {
    let document = parse(source)?;
    let root = map(&document, "project.lock.edn must be an EDN map")?;
    if !matches!(lookup(root, "lock/format"), Some(Form::String(version)) if version == "0.0.0-alpha")
    {
        return Err("project.lock.edn requires :lock/format \"0.0.0-alpha\"".into());
    }
    let packages = match lookup(root, "packages") {
        Some(value) => map(value, "project.lock.edn :packages must be a map")?,
        None => return Ok(Vec::new()),
    };
    let mut output = Vec::with_capacity(packages.len());
    for (coordinate, descriptor) in packages {
        let coordinate = scalar(coordinate, "locked package coordinate")?;
        let descriptor = map(descriptor, "locked package descriptor must be a map")?;
        let name = lookup_any(descriptor, &["name", "package/name"])
            .map(|value| scalar(value, "locked package :name"))
            .transpose()?;
        let version = string(required(descriptor, "version")?, "locked package :version")?;
        semver::Version::parse(&version)
            .map_err(|error| format!("locked package {coordinate} has invalid version: {error}"))?;
        let archive_sha256 = string(
            required(descriptor, "archive-sha256")?,
            "locked package :archive-sha256",
        )?;
        validate_sha256(&archive_sha256)?;
        let tap = string(required(descriptor, "tap")?, "locked package :tap")?;
        let registry_commit = string(
            required(descriptor, "registry-commit")?,
            "locked package :registry-commit",
        )?;
        validate_commit(&registry_commit, "registry-commit")?;
        let identity_revision = string(
            required(descriptor, "identity-revision")?,
            "locked package :identity-revision",
        )?;
        validate_commit(&identity_revision, "identity-revision")?;
        let namespaces = symbols(
            required(descriptor, "namespaces")?,
            "locked package :namespaces",
        )?;
        if namespaces.is_empty() {
            return Err(format!("locked package {coordinate} exports no namespaces"));
        }
        let dependencies = match lookup(descriptor, "dependencies") {
            Some(value) => map_keys(value, "locked package :dependencies")?,
            None => Vec::new(),
        };
        output.push(LockedPackage {
            coordinate,
            name,
            version,
            tap,
            registry_commit,
            identity_revision,
            archive_sha256,
            namespaces,
            dependencies,
        });
    }
    output.sort_by(|left, right| left.coordinate.cmp(&right.coordinate));
    let mut owners = std::collections::BTreeMap::new();
    let mut names = std::collections::BTreeMap::new();
    for package in &output {
        if let Some(name) = &package.name {
            if let Some(previous) = names.insert(name, &package.coordinate) {
                return Err(format!(
                    "package/name-conflict: {name} is used by {previous} and {}",
                    package.coordinate
                ));
            }
        }
        for namespace in &package.namespaces {
            if let Some(previous) = owners.insert(namespace, &package.coordinate) {
                return Err(format!(
                    "package/namespace-conflict: {namespace} is exported by {previous} and {}",
                    package.coordinate
                ));
            }
        }
    }
    let coordinates = output
        .iter()
        .map(|package| package.coordinate.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for package in &output {
        for dependency in &package.dependencies {
            if dependency == &package.coordinate {
                return Err(format!(
                    "package/dependency-cycle: {} depends on itself",
                    package.coordinate
                ));
            }
            if !coordinates.contains(dependency.as_str()) {
                return Err(format!(
                    "package/dependency-not-locked: {} requires {dependency}",
                    package.coordinate
                ));
            }
        }
    }
    Ok(output)
}

/// Reads the same direct package map used by Foundation's
/// `config/packages.edn`. A `:packages` wrapper is accepted as a convenience
/// for project-local profiles, but the selector and dependency semantics stay
/// identical.
pub fn definitions_from_packages_edn(source: &str) -> Result<Vec<PackageDefinition>, String> {
    let document = parse(source)?;
    let root = map(&document, "packages.edn must be an EDN map")?;
    let packages = match lookup(root, "packages") {
        Some(value) => map(value, "packages.edn :packages must be a map")?,
        None => root,
    };
    let mut definitions = Vec::with_capacity(packages.len());
    for (name, descriptor) in packages {
        let name = profile_scalar(name, "package coordinate")?;
        let descriptor = map(descriptor, "package descriptor must be a map")?;
        let description = lookup(descriptor, "description")
            .map(|value| string(value, "package :description"))
            .transpose()?;
        let selectors = parse_selectors(required(descriptor, "include")?, &name)?;
        let dependencies = optional_identifiers(descriptor, "dependencies")?;
        let optional = optional_identifiers(descriptor, "optional")?;
        let bundles = parse_bundles(descriptor)?;
        definitions.push(PackageDefinition {
            name,
            description,
            selectors,
            dependencies,
            optional,
            bundles,
        });
    }
    definitions.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in definitions.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(format!("package/duplicate-definition: {}", pair[0].name));
        }
    }
    Ok(definitions)
}

/// Expands one Foundation selector against the namespaces available in a
/// source tree. The result is sorted and contains no duplicate namespaces.
pub fn expand_selector(selector: &PackageSelector, available: &[String]) -> Vec<String> {
    let mut selected = available
        .iter()
        .filter(|namespace| selector_matches(selector, namespace))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort();
    selected.dedup();
    selected
}

/// Returns the exact namespace ownership implied by one semantic package.
pub fn package_namespaces(package: &PackageDefinition, available: &[String]) -> Vec<String> {
    let mut selected = package
        .selectors
        .iter()
        .flat_map(|selector| expand_selector(selector, available))
        .collect::<Vec<_>>();
    selected.sort();
    selected.dedup();
    selected
}

pub fn find_package_definition<'a>(
    definitions: &'a [PackageDefinition],
    target: &str,
) -> Result<&'a PackageDefinition, String> {
    let matches = definitions
        .iter()
        .filter(|definition| {
            definition.name == target
                || definition
                    .name
                    .rsplit_once('/')
                    .is_some_and(|(_, name)| name == target)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [definition] => Ok(definition),
        [] => Err(format!("package/not-defined: {target}")),
        _ => Err(format!("package/ambiguous-definition: {target}")),
    }
}

/// Validates that package definitions have deterministic, non-overlapping
/// namespace ownership and an acyclic internal dependency graph. Unselected
/// namespaces are allowed so that a profile can intentionally leave
/// host-specific families out; dependencies absent from the profile remain
/// external package dependencies.
pub fn validate_package_definitions(
    definitions: &[PackageDefinition],
    available: &[String],
) -> Result<(), String> {
    let mut owners = std::collections::BTreeMap::new();
    for package in definitions {
        for namespace in package_namespaces(package, available) {
            if let Some(previous) = owners.insert(namespace.clone(), package.name.clone()) {
                return Err(format!(
                    "package/namespace-overlap: {namespace} belongs to {previous} and {}",
                    package.name
                ));
            }
        }
    }
    let targets = definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect::<Vec<_>>();
    package_dependency_order(definitions, &targets).map(|_| ())
}

/// Computes a dependency-first order for selected semantic packages. Names
/// not present in the profile are treated as external dependencies, matching
/// Foundation's separation of internal and external dependency sets.
pub fn package_dependency_order(
    definitions: &[PackageDefinition],
    targets: &[String],
) -> Result<Vec<String>, String> {
    let definitions = definitions
        .iter()
        .map(|definition| (definition.name.clone(), definition))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut ordered = Vec::new();
    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    for target in targets {
        let target = resolve_definition_name(&definitions, target)?;
        visit_package_dependency(
            &target,
            &definitions,
            &mut visiting,
            &mut visited,
            &mut ordered,
        )?;
    }
    Ok(ordered)
}

fn visit_package_dependency(
    name: &str,
    definitions: &std::collections::BTreeMap<String, &PackageDefinition>,
    visiting: &mut std::collections::BTreeSet<String>,
    visited: &mut std::collections::BTreeSet<String>,
    ordered: &mut Vec<String>,
) -> Result<(), String> {
    if visited.contains(name) {
        return Ok(());
    }
    if !visiting.insert(name.to_owned()) {
        return Err(format!("package/dependency-cycle: {name}"));
    }
    let package = definitions
        .get(name)
        .ok_or_else(|| format!("package/not-defined: {name}"))?;
    let mut dependencies = package
        .dependencies
        .iter()
        .filter_map(|dependency| {
            if definitions.contains_key(dependency.as_str()) {
                return Some(Ok(dependency.clone()));
            }
            let suffix = dependency
                .rsplit_once('/')
                .map(|(_, name)| name)
                .unwrap_or(dependency);
            let matches = definitions
                .keys()
                .filter(|name| {
                    name.as_str() == suffix
                        || name.rsplit_once('/').map(|(_, name)| name) == Some(suffix)
                })
                .cloned()
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [] => None,
                [name] => Some(Ok(name.clone())),
                _ => Some(Err(format!(
                    "package/ambiguous-dependency: {dependency} ({})",
                    matches.join(",")
                ))),
            }
        })
        .collect::<Result<Vec<_>, _>>()
        ?;
    dependencies.sort();
    dependencies.dedup();
    for dependency in dependencies {
        visit_package_dependency(&dependency, definitions, visiting, visited, ordered)?;
    }
    visiting.remove(name);
    visited.insert(name.to_owned());
    ordered.push(name.to_owned());
    Ok(())
}

fn resolve_definition_name(
    definitions: &std::collections::BTreeMap<String, &PackageDefinition>,
    target: &str,
) -> Result<String, String> {
    resolve_optional_definition_name(definitions, target)
        .ok_or_else(|| format!("package/not-defined: {target}"))
}

fn resolve_optional_definition_name(
    definitions: &std::collections::BTreeMap<String, &PackageDefinition>,
    target: &str,
) -> Option<String> {
    if definitions.contains_key(target) {
        return Some(target.to_owned());
    }
    let suffix = target
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(target);
    let matches = definitions
        .keys()
        .filter(|name| {
            name.as_str() == suffix || name.rsplit_once('/').map(|(_, name)| name) == Some(suffix)
        })
        .cloned()
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
}

fn parse_selectors(form: &Form, package: &str) -> Result<Vec<PackageSelector>, String> {
    let values = sequence(form, &format!("package {package} :include"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let parts = sequence(value, &format!("package {package} selector {index}"))?;
            let namespace = profile_scalar(
                parts
                    .first()
                    .ok_or_else(|| format!("package {package} selector {index} is empty"))?,
                "package selector namespace",
            )?;
            let mode = parts
                .get(1)
                .ok_or_else(|| format!("package {package} selector {index} is missing a mode"))?;
            let mode = profile_scalar(mode, "package selector mode")?;
            match mode.as_str() {
                "base" if parts.len() == 2 => Ok(PackageSelector {
                    namespace,
                    mode: PackageSelectorMode::Base,
                }),
                "complete" if parts.len() == 2 => Ok(PackageSelector {
                    namespace,
                    mode: PackageSelectorMode::Complete,
                }),
                "exclude" if parts.len() == 3 => {
                    let excluded = sequence(&parts[2], "package selector exclusions")?
                        .iter()
                        .map(|value| profile_scalar(value, "package selector exclusion"))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(PackageSelector {
                        namespace,
                        mode: PackageSelectorMode::Exclude(excluded),
                    })
                }
                "base" | "complete" | "exclude" => Err(format!(
                    "package {package} selector {index} has invalid arity for :{mode}"
                )),
                _ => Err(format!(
                    "package {package} selector {index} has unsupported mode :{mode}"
                )),
            }
        })
        .collect()
}

fn parse_bundles(descriptor: &[(Form, Form)]) -> Result<Vec<PackageBundle>, String> {
    let Some(form) = lookup(descriptor, "bundle") else {
        return Ok(Vec::new());
    };
    sequence(form, "package :bundle")?
        .iter()
        .enumerate()
        .map(|(index, form)| {
            let entries = map(form, "package bundle must be a map")?;
            let path = string(required(entries, "path")?, "package bundle :path")?;
            let include = sequence(required(entries, "include")?, "package bundle :include")?
                .iter()
                .map(|value| string(value, "package bundle include path"))
                .collect::<Result<Vec<_>, _>>()?;
            if path.is_empty() {
                return Err(format!("package bundle {index} has an empty path"));
            }
            Ok(PackageBundle { path, include })
        })
        .collect()
}

fn optional_identifiers(descriptor: &[(Form, Form)], key: &str) -> Result<Vec<String>, String> {
    let Some(form) = lookup(descriptor, key) else {
        return Ok(Vec::new());
    };
    let mut values = sequence(form, &format!("package :{key}"))?
        .iter()
        .map(|value| profile_scalar(value, &format!("package :{key}")))
        .collect::<Result<Vec<_>, _>>()?;
    values.sort();
    values.dedup();
    Ok(values)
}

fn sequence<'a>(form: &'a Form, label: &str) -> Result<&'a Vec<Form>, String> {
    match form {
        Form::Vector(values) | Form::List(values) => Ok(values),
        _ => Err(format!("{label} must be a vector or list")),
    }
}

fn profile_scalar(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) | Form::Symbol(value) | Form::Keyword(value) if !value.is_empty() => {
            Ok(value.clone())
        }
        _ => Err(format!(
            "{label} must be a non-empty string, symbol, or keyword"
        )),
    }
}

fn selector_matches(selector: &PackageSelector, namespace: &str) -> bool {
    let within = |root: &str| namespace == root || namespace.starts_with(&format!("{root}."));
    match &selector.mode {
        PackageSelectorMode::Complete => within(&selector.namespace),
        PackageSelectorMode::Base => {
            namespace == selector.namespace
                || namespace.starts_with(&format!("{}.base.", selector.namespace))
        }
        PackageSelectorMode::Exclude(excluded) => {
            within(&selector.namespace) && !excluded.iter().any(|root| within(root))
        }
    }
}

fn map<'a>(form: &'a Form, message: &str) -> Result<&'a Vec<(Form, Form)>, String> {
    match form {
        Form::Map(entries) => Ok(entries),
        _ => Err(message.into()),
    }
}
fn lookup<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
    })
}
fn lookup_any<'a>(entries: &'a [(Form, Form)], keys: &[&str]) -> Option<&'a Form> {
    keys.iter().find_map(|key| lookup(entries, key))
}
fn required<'a>(entries: &'a [(Form, Form)], key: &str) -> Result<&'a Form, String> {
    lookup(entries, key).ok_or_else(|| format!("locked package is missing :{key}"))
}
fn string(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a string")),
    }
}
fn scalar(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) | Form::Symbol(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a string or symbol")),
    }
}
fn symbols(form: &Form, label: &str) -> Result<Vec<String>, String> {
    let Form::Vector(values) = form else {
        return Err(format!("{label} must be a vector"));
    };
    let mut output = values
        .iter()
        .map(|value| scalar(value, label))
        .collect::<Result<Vec<_>, _>>()?;
    output.sort();
    output.dedup();
    Ok(output)
}
fn map_keys(form: &Form, label: &str) -> Result<Vec<String>, String> {
    let entries = map(form, &format!("{label} must be a map"))?;
    let mut output = entries
        .iter()
        .map(|(key, _)| scalar(key, label))
        .collect::<Result<Vec<_>, _>>()?;
    output.sort();
    output.dedup();
    Ok(output)
}
fn validate_sha256(value: &str) -> Result<(), String> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    if value.len() == 64 && value.chars().all(|value| value.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("locked package :archive-sha256 must be SHA-256".into())
    }
}
fn validate_commit(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 40
        && value
            .chars()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(format!(
            "locked package :{label} must be a lowercase 40-character Git commit"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exact_lock_catalog_and_rejects_namespace_conflicts() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let commit = "b".repeat(40);
        let source = format!("{{:lock/format \"0.0.0-alpha\" :packages {{\"hara:demo/base\" {{:version \"1.0.0\" :tap \"hara\" :registry-commit \"{commit}\" :identity-revision \"{commit}\" :archive-sha256 \"{digest}\" :namespaces [demo.base]}} \"hara:demo/core\" {{:version \"1.2.3\" :tap \"hara\" :registry-commit \"{commit}\" :identity-revision \"{commit}\" :archive-sha256 \"{digest}\" :namespaces [demo.core demo.util] :dependencies {{\"hara:demo/base\" \"1.0.0\"}}}}}}}}");
        let catalog = catalog_from_lock(&source).unwrap();
        assert_eq!(catalog[1].namespaces, vec!["demo.core", "demo.util"]);
        assert_eq!(catalog[1].dependencies, vec!["hara:demo/base"]);
    }

    #[test]
    fn accepts_package_name_aliases_and_rejects_duplicate_semantic_names() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let commit = "b".repeat(40);
        let source = format!(
            "{{:lock/format \"0.0.0-alpha\" :packages {{\"hara:demo/one\" {{:package/name \"demo.shared\" :version \"1.0.0\" :tap \"hara\" :registry-commit \"{commit}\" :identity-revision \"{commit}\" :archive-sha256 \"{digest}\" :namespaces [demo.one]}} \"hara:demo/two\" {{:name \"demo.shared\" :version \"1.0.0\" :tap \"hara\" :registry-commit \"{commit}\" :identity-revision \"{commit}\" :archive-sha256 \"{digest}\" :namespaces [demo.two]}}}}}}"
        );
        let error = catalog_from_lock(&source).unwrap_err();
        assert!(error.contains("package/name-conflict"), "{error}");
    }

    #[test]
    fn reads_foundation_selectors_and_expands_semantic_packages() {
        let source = r#"{
          xyz.zcaudate/code.test {:description "tests" :include [[code.test :complete]]}
          lang.model.v1.postgres {:include [[lang.model.v1.spec-postgres :complete]
                                            [postgres.core :complete]
                                            [postgres.typed :complete]
                                            [postgres.gen :complete]]
                                  :dependencies [lang.base]}
          lang.base {:include [[lang.base :complete]]}
        }"#;
        let definitions = definitions_from_packages_edn(source).unwrap();
        assert_eq!(definitions[0].name, "lang.base");
        let postgres = definitions
            .iter()
            .find(|definition| definition.name == "lang.model.v1.postgres")
            .unwrap();
        let available = vec![
            "lang.model.v1.spec-postgres".into(),
            "lang.model.v1.spec-postgres.deftype.common".into(),
            "postgres.core".into(),
            "postgres.core.graph".into(),
            "postgres.typed".into(),
            "postgres.gen.rpc".into(),
            "db.postgres".into(),
        ];
        assert_eq!(
            package_namespaces(postgres, &available),
            vec![
                "lang.model.v1.spec-postgres".to_owned(),
                "lang.model.v1.spec-postgres.deftype.common".to_owned(),
                "postgres.core".to_owned(),
                "postgres.core.graph".to_owned(),
                "postgres.gen.rpc".to_owned(),
                "postgres.typed".to_owned(),
            ]
        );
        validate_package_definitions(&definitions, &available).unwrap();
        assert_eq!(
            package_dependency_order(&definitions, &["lang.model.v1.postgres".to_owned()]).unwrap(),
            vec!["lang.base", "lang.model.v1.postgres"]
        );
    }

    #[test]
    fn foundation_base_and_exclude_selectors_follow_namespace_boundaries() {
        let source = r#"{
          demo {:include [[demo :base]]}
          other {:include [[other :exclude [other.internal]]]}
        }"#;
        let definitions = definitions_from_packages_edn(source).unwrap();
        let available = vec![
            "demo".into(),
            "demo.base".into(),
            "demo.base.value".into(),
            "demo.extra".into(),
            "other".into(),
            "other.internal".into(),
            "other.internal.deep".into(),
            "other.public".into(),
        ];
        assert_eq!(
            package_namespaces(&definitions[0], &available),
            vec!["demo".to_owned(), "demo.base.value".to_owned()]
        );
        assert_eq!(
            package_namespaces(&definitions[1], &available),
            vec!["other".to_owned(), "other.public".to_owned()]
        );
    }

    #[test]
    fn rejects_overlapping_semantic_package_ownership_and_cycles() {
        let source = r#"{
          a {:include [[demo :complete]] :dependencies [b]}
          b {:include [[other :complete]] :dependencies [a]}
          c {:include [[demo.child :complete]]}
        }"#;
        let definitions = definitions_from_packages_edn(source).unwrap();
        let available = vec!["demo".into(), "demo.child".into(), "other".into()];
        let error = validate_package_definitions(&definitions, &available).unwrap_err();
        assert!(error.contains("package/namespace-overlap"), "{error}");
        let error = package_dependency_order(&definitions, &["a".into()]).unwrap_err();
        assert!(error.contains("package/dependency-cycle"), "{error}");
    }

    #[test]
    fn resolves_unique_tap_suffixes_but_rejects_ambiguous_dependencies() {
        let source = r#"{
          xyz.zcaudate/base {:include [[demo.base :complete]]}
          xyz.zcaudate/app {:include [[demo.app :complete]] :dependencies [base]}
        }"#;
        let definitions = definitions_from_packages_edn(source).unwrap();
        assert_eq!(
            package_dependency_order(&definitions, &["app".into()]).unwrap(),
            vec!["xyz.zcaudate/base", "xyz.zcaudate/app"]
        );

        let source = r#"{
          one/base {:include [[demo.one :complete]]}
          two/base {:include [[demo.two :complete]]}
          app {:include [[demo.app :complete]] :dependencies [base]}
        }"#;
        let definitions = definitions_from_packages_edn(source).unwrap();
        let error = package_dependency_order(&definitions, &["app".into()]).unwrap_err();
        assert!(error.contains("package/ambiguous-dependency"), "{error}");
    }
}
