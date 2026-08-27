use crate::kernel::{parse_forms, Form};
use crate::package_manifest::PackageManifest;
use crate::project::Project;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub(super) fn build_archive(project: &Project, output: &Path) -> Result<(), String> {
    let mut entries = Vec::new();
    if let Some(source_files) = &project.source_files {
        for source_file in source_files {
            collect_source_file(source_file, &project.root, &mut entries)?;
        }
    } else {
        for source_path in &project.source_paths {
            let base = project.root.join(source_path);
            collect_files(&base, &project.root, false, false, &mut entries)?;
        }
    }
    for artifact_path in &project.artifact_paths {
        let base = project.root.join(artifact_path);
        collect_files(&base, &project.root, true, true, &mut entries)?;
    }
    // A release archive must be self-describing. These entries intentionally
    // stay at its root even when :project/archive-root relocates artifacts.
    let manifest_source = project
        .manifest_path
        .strip_prefix(&project.root)
        .map(PathBuf::from)
        .map_err(|_| "project manifest must be inside its project root".to_owned())?;
    entries.push(manifest_source.clone());
    if let Some(profile) = &project.package_profile {
        collect_declared_file(profile, &project.root, &mut entries)?;
    }
    if let Some(recipe) = &project.recipe {
        entries.push(recipe.clone());
    }
    let lock = project.root.join("project.lock.edn");
    if lock.is_file() {
        entries.push(PathBuf::from("project.lock.edn"));
    } else if !project.dependencies.is_empty()
        || !project.npm_dependencies.is_empty()
        || !project.native_imports.is_empty()
    {
        return Err(
            "package build requires project.lock.edn when package dependencies or native imports are non-empty"
                .into(),
        );
    }
    entries.extend(crate::project::native_archive_entries(project)?);
    if project.package_workspace {
        let workspace = project.root.join("workspace.edn");
        if !workspace.is_file() {
            return Err("project.edn declares :project/package {:workspace true}, but workspace.edn is missing".into());
        }
        entries.push(PathBuf::from("workspace.edn"));
    }
    let mut archive_entries = Vec::new();
    for source in entries {
        let archive = if source == manifest_source
            || project
                .package_profile
                .as_ref()
                .is_some_and(|profile| source == *profile)
            || matches!(source.as_path(), path if path == Path::new("project.lock.edn") || path == Path::new("workspace.edn"))
        {
            if source == manifest_source {
                PathBuf::from("project.edn")
            } else {
                source.clone()
            }
        } else {
            match &project.archive_root {
                Some(root) => source
                    .strip_prefix(root)
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| source.clone()),
                None => source.clone(),
            }
        };
        validate_relative_path(&archive)?;
        if archive.as_os_str().is_empty() {
            return Err("package archive path must name a file".into());
        }
        archive_entries.push((archive, source));
    }
    archive_entries.sort_by(|left, right| left.0.cmp(&right.0));
    for pair in archive_entries.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(format!(
                "duplicate package archive path: {}",
                pair[0].0.display()
            ));
        }
    }
    if archive_entries.is_empty() {
        return Err(
            "package build found no files in :project/source-paths, :project/source-files, or :project/artifact-paths"
                .into(),
        );
    }
    let mut contents = Vec::new();
    for (archive, source) in &archive_entries {
        let bytes = fs::read(project.root.join(source))
            .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
        contents.push((archive.clone(), bytes));
    }
    #[cfg(feature = "bytecode-vm")]
    let compilation_context = contents
        .iter()
        .filter(|(path, _)| path.extension().and_then(|value| value.to_str()) == Some("hal"))
        .map(|(path, bytes)| {
            let source = std::str::from_utf8(bytes)
                .map_err(|_| format!("HAL package resource is not UTF-8: {}", path.display()))?;
            let namespace = hal_namespace(source)
                .map_err(|error| {
                    format!("cannot parse package resource {}: {error}", path.display())
                })?
                .ok_or_else(|| {
                    format!("HAL package resource has no namespace: {}", path.display())
                })?;
            Ok((namespace, source.to_owned()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let selected_package = if project.package_name.is_some() {
        let profile_path = project
            .package_profile
            .as_ref()
            .ok_or("package selection requires a package profile")?;
        let package_name = project
            .package_name
            .as_deref()
            .expect("package_name is present when selecting a package");
        let profile_source =
            fs::read_to_string(project.root.join(profile_path)).map_err(|error| {
                format!(
                    "cannot read package profile {}: {error}",
                    profile_path.display()
                )
            })?;
        let definitions = crate::package_catalog::definitions_from_packages_edn(&profile_source)?;
        let mut available = Vec::new();
        for (path, bytes) in &contents {
            if path.extension().and_then(|value| value.to_str()) != Some("hal") {
                continue;
            }
            let source = std::str::from_utf8(bytes)
                .map_err(|_| format!("HAL package resource is not UTF-8: {}", path.display()))?;
            if let Some(namespace) = hal_namespace(source)? {
                available.push(namespace);
            }
        }
        available.sort();
        available.dedup();
        crate::package_catalog::validate_package_definitions(&definitions, &available)?;
        let package = crate::package_catalog::find_package_definition(&definitions, package_name)?;
        let selected = crate::package_catalog::package_namespaces(package, &available);
        if selected.is_empty() {
            return Err(format!(
                "package {package_name} selects no namespaces from the project source tree"
            ));
        }
        Some((
            package.clone(),
            selected
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
        ))
    } else {
        None
    };
    if let Some((_, selected)) = &selected_package {
        let mut filtered = Vec::with_capacity(contents.len());
        for (path, bytes) in contents {
            if path.extension().and_then(|value| value.to_str()) == Some("hal") {
                let source = std::str::from_utf8(&bytes).map_err(|_| {
                    format!("HAL package resource is not UTF-8: {}", path.display())
                })?;
                let namespace = hal_namespace(source)?.ok_or_else(|| {
                    format!("HAL package resource has no namespace: {}", path.display())
                })?;
                if !selected.contains(&namespace) {
                    continue;
                }
            }
            filtered.push((path, bytes));
        }
        contents = filtered;
    }
    if let Some((package, _)) = &selected_package {
        for bundle in &package.bundles {
            collect_bundle_files(bundle, &project.root, &mut contents)?;
        }
    }
    #[cfg(feature = "bytecode-vm")]
    let hal_modules = contents
        .iter()
        .filter(|(path, _)| path.extension().and_then(|value| value.to_str()) == Some("hal"))
        .map(|(path, bytes)| {
            let source = std::str::from_utf8(bytes)
                .map_err(|_| format!("HAL package resource is not UTF-8: {}", path.display()))?;
            let namespace = hal_namespace(source)
                .map_err(|error| {
                    format!("cannot parse package resource {}: {error}", path.display())
                })?
                .ok_or_else(|| {
                    format!("HAL package resource has no namespace: {}", path.display())
                })?;
            Ok((namespace, source.to_owned()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    #[cfg(feature = "bytecode-vm")]
    if !hal_modules.is_empty() {
        let sources = hal_modules
            .iter()
            .map(|(namespace, source)| crate::vm::ModuleSource {
                resource: namespace,
                source,
            })
            .collect::<Vec<_>>();
        let context = compilation_context
            .iter()
            .map(|(namespace, source)| crate::vm::ModuleSource {
                resource: namespace,
                source,
            })
            .collect::<Vec<_>>();
        let bundle = crate::vm::compile_package_bytecode_bundle(&context, &sources)?;
        contents.push((PathBuf::from("bytecode/package.hbx"), bundle));
        contents.sort_by(|left, right| left.0.cmp(&right.0));
    }
    contents.sort_by(|left, right| left.0.cmp(&right.0));
    for pair in contents.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(format!(
                "duplicate package archive path: {}",
                pair[0].0.display()
            ));
        }
    }
    let generated_manifest = package_manifest(project, &contents)?;
    let package_edn = PackageManifest::parse(&generated_manifest)
        .map_err(|error| error.to_string())?
        .canonical_edn()
        .to_owned();
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let file = File::create(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    writer
        .start_file("package.edn", options)
        .map_err(zip_error)?;
    writer.write_all(package_edn.as_bytes()).map_err(io_error)?;
    for (path, bytes) in contents {
        let archive_path = path_to_slash(&path)?;
        writer
            .start_file(archive_path, options)
            .map_err(zip_error)?;
        writer.write_all(&bytes).map_err(io_error)?;
    }
    writer.finish().map_err(zip_error)?;
    PackageManifest::read_archive(output).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn inspect_archive(path: &Path) -> Result<String, String> {
    PackageManifest::read_archive(path)
        .map(|manifest| manifest.canonical_edn().to_owned())
        .map_err(|error| error.to_string())
}

pub(super) fn package_manifest(
    project: &Project,
    contents: &[(PathBuf, Vec<u8>)],
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut files = String::new();
    let mut resources = Vec::new();
    for (path, bytes) in contents {
        let path = path_to_slash(path).expect("validated project-relative path");
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        files.push_str(&format!(
            "  {} {{:sha256 \"sha256:{}\" :size {}}}\n",
            edn_string(&path),
            hex(&Sha256::digest(bytes)),
            bytes.len()
        ));
        if path.ends_with(".hal") {
            let source = std::str::from_utf8(bytes)
                .map_err(|_| format!("HAL package resource is not UTF-8: {path}"))?;
            if let Some(namespace) = hal_namespace(source)
                .map_err(|error| format!("cannot parse package resource {path}: {error}"))?
            {
                resources.push((namespace, path.clone()));
            }
        } else if path.ends_with(".halc") || path.ends_with(".hir") {
            let module = crate::kernel::halc::decode_halc(bytes)
                .map_err(|error| format!("cannot decode package resource {path}: {error}"))?;
            resources.push((module.namespace, path.clone()));
        }
    }
    resources.sort();
    for pair in resources.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(format!("duplicate package namespace: {}", pair[0].0));
        }
    }
    let resources = resources
        .iter()
        .map(|(namespace, path)| format!("  {} {}\n", edn_string(namespace), edn_string(path)))
        .collect::<String>();
    let extensions = Form::Map(
        project
            .extensions
            .iter()
            .map(|(namespace, declaration)| (Form::Symbol(namespace.clone()), declaration.clone()))
            .collect(),
    )
    .to_string();
    let bytecode = contents
        .iter()
        .find(|(path, _)| path == Path::new("bytecode/package.hbx"))
        .map(|(_, bytes)| {
            format!(
                "\n :bytecode {{:format \"0.0.0-alpha\" :path \"bytecode/package.hbx\" :sha256 \"sha256:{}\"}}",
                hex(&Sha256::digest(bytes))
            )
        })
        .unwrap_or_default();
    let package_name = project
        .package_name
        .as_deref()
        .map(|name| format!(" :name {}", edn_string(name)))
        .unwrap_or_default();
    let schema_catalog = contents
        .iter()
        .find(|(path, _)| path == Path::new("catalog/std-typed-catalog.json"))
        .map(|(_, bytes)| {
            format!(
                "\n :schema/catalog {{:format \"std.typed.catalog/2\" :path \"catalog/std-typed-catalog.json\" :sha256 \"sha256:{}\"}}",
                hex(&Sha256::digest(bytes))
            )
        })
        .unwrap_or_default();
    let identity = match project.package_name.as_deref() {
        Some(name) => super::semantic_package_identity(name)?,
        None => crate::project::normalize_coordinate(&project.id)?,
    };
    Ok(format!(
        "{{:harp/format \"0.0.0-alpha\"\n :package {{:identity {}{} :version {}}}\n :files {{\n{}}} :resources {{\n{}}} :extensions {}{}{}\n :integrity {{:tree-sha256 \"sha256:{}\"}}}}\n",
        edn_string(&identity),
        package_name,
        edn_string(&project.version.to_string()),
        files,
        resources,
        extensions,
        bytecode,
        schema_catalog,
        hex(&hasher.finalize())
    ))
}

pub(super) fn hal_namespace(source: &str) -> Result<Option<String>, String> {
    for form in parse_forms(source)? {
        let Form::List(forms) = form else { continue };
        let [Form::Symbol(head), Form::Symbol(namespace), ..] = forms.as_slice() else {
            continue;
        };
        if head == "ns" || head == "ns+" {
            return Ok(Some(namespace.clone()));
        }
    }
    Ok(None)
}

pub(super) fn edn_string(value: &str) -> String {
    Form::String(value.to_owned()).to_string()
}

pub(super) fn collect_files(
    directory: &Path,
    root: &Path,
    include_all: bool,
    required: bool,
    entries: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if !directory.exists() {
        return if required {
            Err(format!(
                "declared package path does not exist: {}",
                directory.display()
            ))
        } else {
            Ok(())
        };
    }
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "package entries must not be symbolic links: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            collect_files(&path, root, include_all, true, entries)?;
        } else if metadata.is_file()
            && (include_all
                || path.extension().and_then(|extension| extension.to_str()) == Some("hal"))
        {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "package path escapes project root".to_owned())?;
            validate_relative_path(relative)?;
            entries.push(relative.to_path_buf());
        }
    }
    Ok(())
}

fn collect_declared_file(
    source: &Path,
    root: &Path,
    entries: &mut Vec<PathBuf>,
) -> Result<(), String> {
    validate_relative_path(source)?;
    let path = root.join(source);
    reject_source_symlinks(&path, root)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "declared package file is not a file: {}",
            source.display()
        ));
    }
    entries.push(source.to_path_buf());
    Ok(())
}

fn collect_bundle_files(
    bundle: &crate::package_catalog::PackageBundle,
    root: &Path,
    contents: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), String> {
    let bundle_path = Path::new(&bundle.path);
    validate_relative_path(bundle_path)?;
    if bundle_path.as_os_str().is_empty() {
        return Err("package bundle path must name a directory".into());
    }
    let base = root.join(bundle_path);
    reject_source_symlinks(&base, root)?;
    let metadata = fs::symlink_metadata(&base)
        .map_err(|error| format!("cannot read package bundle {}: {error}", base.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "package bundle path is not a directory: {}",
            bundle.path
        ));
    }

    for include in &bundle.include {
        let include_path = Path::new(include);
        validate_relative_path(include_path)?;
        if include_path.as_os_str().is_empty() {
            return Err(format!(
                "package bundle {} has an empty include path",
                bundle.path
            ));
        }
        let selected = base.join(include_path);
        reject_source_symlinks(&selected, root)?;
        let metadata = fs::symlink_metadata(&selected).map_err(|error| {
            format!(
                "cannot read package bundle include {}: {error}",
                selected.display()
            )
        })?;
        if metadata.is_dir() {
            collect_bundle_directory(&selected, &base, root, contents)?;
        } else if metadata.is_file() {
            collect_bundle_file(&selected, &base, root, contents)?;
        } else {
            return Err(format!(
                "package bundle include is not a regular file or directory: {}",
                selected.display()
            ));
        }
    }
    Ok(())
}

fn collect_bundle_directory(
    directory: &Path,
    base: &Path,
    root: &Path,
    contents: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), String> {
    reject_source_symlinks(directory, root)?;
    let mut children = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "cannot read package bundle {}: {error}",
                directory.display()
            )
        })?
        .map(|entry| entry.map(|entry| entry.path()).map_err(io_error))
        .collect::<Result<Vec<_>, _>>()?;
    children.sort();
    for child in children {
        let metadata = fs::symlink_metadata(&child).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "package entries must not be symbolic links: {}",
                child.display()
            ));
        }
        if metadata.is_dir() {
            collect_bundle_directory(&child, base, root, contents)?;
        } else if metadata.is_file() {
            collect_bundle_file(&child, base, root, contents)?;
        }
    }
    Ok(())
}

fn collect_bundle_file(
    file: &Path,
    base: &Path,
    root: &Path,
    contents: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), String> {
    reject_source_symlinks(file, root)?;
    let archive = file
        .strip_prefix(base)
        .map_err(|_| format!("package bundle file escapes its bundle: {}", file.display()))?
        .to_path_buf();
    validate_relative_path(&archive)?;
    let source = file
        .strip_prefix(root)
        .map_err(|_| "package bundle file escapes project root".to_owned())?
        .to_path_buf();
    validate_relative_path(&source)?;
    let bytes =
        fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    contents.push((archive, bytes));
    Ok(())
}

fn reject_source_symlinks(path: &Path, root: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("package path escapes project root: {}", path.display()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "package entries must not be symbolic links: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn collect_source_file(
    source: &Path,
    root: &Path,
    entries: &mut Vec<PathBuf>,
) -> Result<(), String> {
    validate_relative_path(source)?;
    let path = root.join(source);
    let mut current = root.to_path_buf();
    for component in source.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "package entries must not be symbolic links: {}",
                    source.display()
                ));
            }
        }
    }
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "package entries must not be symbolic links: {}",
            source.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "declared package source file is not a file: {}",
            source.display()
        ));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("hal") {
        return Err(format!(
            "declared package source file must be a .hal file: {}",
            source.display()
        ));
    }
    entries.push(source.to_path_buf());
    Ok(())
}

pub(super) fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!("unsafe package path: {}", path.display()));
    }
    Ok(())
}

pub(super) fn path_to_slash(path: &Path) -> Result<String, String> {
    validate_relative_path(path)?;
    path.to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| format!("package path is not UTF-8: {}", path.display()))
}

pub(super) fn archive_name(id: &str) -> String {
    id.replace('/', "-")
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) fn io_error(error: std::io::Error) -> String {
    error.to_string()
}
pub(super) fn zip_error(error: zip::result::ZipError) -> String {
    error.to_string()
}
