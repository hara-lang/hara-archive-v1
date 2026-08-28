use super::{declared_namespace, files_in, Project};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "resources/installed.rs"]
mod installed;

/// A source-only namespace catalog used by native runtimes.
///
/// The catalog deliberately stores paths rather than source text. Project
/// startup scans each file for its top-level namespace declaration (some
/// legacy library paths do not mirror their namespace); source is retained and
/// fully parsed/evaluated only when a namespace is actually required.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceCatalog {
    entries: BTreeMap<String, PathBuf>,
}

impl SourceCatalog {
    pub(crate) fn entries(&self) -> &BTreeMap<String, PathBuf> {
        &self.entries
    }

    pub fn path(&self, namespace: &str) -> Option<&Path> {
        self.entries.get(namespace).map(PathBuf::as_path)
    }

    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    fn add_project(&mut self, project: &Project, owner: &str) -> Result<(), String> {
        let project_root = project
            .root
            .canonicalize()
            .map_err(|error| format!("cannot resolve {}: {error}", project.root.display()))?;
        let mut owned = BTreeMap::<String, PathBuf>::new();
        for source_root in &project.source_paths {
            let source_root = project.root.join(source_root);
            if !source_root.exists() {
                continue;
            }
            let source_root = source_root.canonicalize().map_err(|error| {
                format!(
                    "cannot resolve source root {}: {error}",
                    source_root.display()
                )
            })?;
            if !source_root.starts_with(&project_root) {
                return Err(format!(
                    "source root escapes project root: {}",
                    source_root.display()
                ));
            }
            for path in files_in(
                &project.root,
                &[source_root_for_project(&project_root, &source_root)?],
            )? {
                let path = path
                    .canonicalize()
                    .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
                if !path.starts_with(&project_root) {
                    return Err(format!(
                        "source file escapes project root: {}",
                        path.display()
                    ));
                }
                let source = fs::read_to_string(&path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
                let namespace = declared_namespace_header(&source)
                    .map_err(|error| format!("{}: {error}", path.display()))?
                    .ok_or_else(|| {
                        format!(
                            "{} does not declare an ns or ns+ namespace",
                            path.display()
                        )
                    })?;
                if let Some(previous) = owned.insert(namespace.clone(), path.clone()) {
                    if previous != path {
                        return Err(format!(
                            "duplicate namespace {namespace} in {owner}: {} and {}",
                            previous.display(),
                            path.display()
                        ));
                    }
                }
            }
        }
        // Later project layers intentionally overlay earlier layers.  This
        // preserves the existing lite-project-then-application ordering while
        // keeping duplicate files within one project an error.
        self.entries.extend(owned);
        Ok(())
    }
}

fn source_root_for_project(project_root: &Path, source_root: &Path) -> Result<PathBuf, String> {
    source_root
        .strip_prefix(project_root)
        .map(Path::to_path_buf)
        .map_err(|_| {
            format!(
                "source root {} is outside project root {}",
                source_root.display(),
                project_root.display()
            )
        })
}

fn declared_namespace_header(source: &str) -> Result<Option<String>, String> {
    let mut depth = 0;
    let mut form_start = None;
    let mut in_comment = false;
    let mut in_string = false;
    let mut escaped = false;
    let mut skip_character = false;
    for (index, character) in source.char_indices() {
        if skip_character {
            skip_character = false;
            continue;
        }
        if in_comment {
            if character == '\n' {
                in_comment = false;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            ';' => in_comment = true,
            '"' => in_string = true,
            '\\' => skip_character = true,
            '(' | '[' | '{' => {
                if depth == 0 && character == '(' {
                    form_start = Some(index);
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = form_start.take() {
                        let end = index + character.len_utf8();
                        if let Some(namespace) = declared_namespace(&source[start..end])? {
                            return Ok(Some(namespace));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Builds a path-backed source catalog for one project and its installed Hara
/// dependencies.
pub fn source_catalog(project: &Project) -> Result<SourceCatalog, String> {
    source_catalogs(&[project])
}

/// Builds a path-backed source catalog for several ordered project layers.
/// Each project contributes its verified installed dependencies first,
/// followed by its own source paths; later project layers take precedence.
pub fn source_catalogs(projects: &[&Project]) -> Result<SourceCatalog, String> {
    let distribution_root = dist_root();
    let mut catalog = SourceCatalog::default();
    for project in projects {
        for dependency in installed::resolve(project, &distribution_root)? {
            catalog.add_project(
                &dependency.project,
                &format!("{}@{}", dependency.coordinate, dependency.version),
            )?;
        }
        catalog.add_project(project, &format!("{}@{}", project.id, project.version))?;
    }
    Ok(catalog)
}

/// Returns namespace resources from installed dependencies followed by the
/// automatically selected native Rust profile of the consuming project.
pub fn source_resources(project: &Project) -> Result<Vec<(String, String)>, String> {
    source_resources_at(project, &dist_root())
}

pub(crate) fn source_resources_at(
    project: &Project,
    distribution_root: &Path,
) -> Result<Vec<(String, String)>, String> {
    let mut resources = Vec::new();
    let mut declarations = BTreeMap::<String, (String, PathBuf)>::new();
    for dependency in installed::resolve(project, distribution_root)? {
        collect_project(
            &dependency.project,
            &format!("{}@{}", dependency.coordinate, dependency.version),
            &mut declarations,
            &mut resources,
        )?;
    }
    collect_project(
        project,
        &format!("{}@{}", project.id, project.version),
        &mut declarations,
        &mut resources,
    )?;
    Ok(resources)
}

fn collect_project(
    project: &Project,
    owner: &str,
    declarations: &mut BTreeMap<String, (String, PathBuf)>,
    resources: &mut Vec<(String, String)>,
) -> Result<(), String> {
    for path in files_in(&project.root, &project.source_paths)? {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let namespace = declared_namespace(&source)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .ok_or_else(|| format!("{} does not declare an ns or ns+ namespace", path.display()))?;
        if let Some((previous_owner, previous_path)) =
            declarations.insert(namespace.clone(), (owner.to_owned(), path.clone()))
        {
            return Err(format!(
                "duplicate namespace {namespace}: {previous_owner} ({}) and {owner} ({})",
                previous_path.display(),
                path.display()
            ));
        }
        resources.push((namespace, source));
    }
    Ok(())
}

fn dist_root() -> PathBuf {
    if let Some(root) = std::env::var_os("HARA_DIST_HOME") {
        return PathBuf::from(root);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hara/dist")
}
