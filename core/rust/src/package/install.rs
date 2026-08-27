use super::{
    file_sha256, io_error, read_project, split_coordinate, validate_relative_path, zip_error,
};
use crate::kernel::{parse, Form};
use crate::package_manifest::PackageManifest;
use crate::project::{self, Project};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const UNIX_FILE_TYPE_MASK: u32 = 0o170000;
const UNIX_SYMLINK_TYPE: u32 = 0o120000;

pub(super) fn validate_recipe(project: &Project) -> Result<PathBuf, String> {
    let relative = project
        .recipe
        .as_ref()
        .ok_or("publication requires :project/recipe")?;
    let path = project.root.join(relative);
    let source = fs::read_to_string(&path).map_err(io_error)?;
    let Form::Map(entries) = parse(&source)? else {
        return Err("hara.recipe.edn must be an EDN map".into());
    };
    for key in [
        "recipe/format",
        "recipe/adapter",
        "recipe/toolchain",
        "recipe/inputs",
        "recipe/outputs",
    ] {
        if !entries
            .iter()
            .any(|(candidate, _)| matches!(candidate, Form::Keyword(name) if name == key))
        {
            return Err(format!("hara.recipe.edn is missing :{key}"));
        }
    }
    let adapter = entries
        .iter()
        .find(|(candidate, _)| matches!(candidate, Form::Keyword(name) if name == "recipe/adapter"))
        .map(|(_, value)| value);
    if !matches!(adapter, Some(Form::Keyword(name)) if matches!(name.as_str(), "rust-wasm" | "node-hta" | "hal"))
    {
        return Err(
            "hara.recipe.edn :recipe/adapter must be :rust-wasm, :node-hta, or :hal".into(),
        );
    }
    if source.contains(":command") || source.contains(":script") || source.contains(":shell") {
        return Err("official recipes cannot declare commands, scripts, or shell fragments".into());
    }
    Ok(path)
}

pub(super) fn dist_root() -> PathBuf {
    if let Some(root) = std::env::var_os("HARA_DIST_HOME") {
        return PathBuf::from(root);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hara/dist")
}

pub(super) fn install_archive(archive: &Path) -> Result<PathBuf, String> {
    install_archive_at(archive, &dist_root())
}

pub(super) fn install_archive_at(archive: &Path, root: &Path) -> Result<PathBuf, String> {
    let digest = file_sha256(archive)?;
    let archive_target = root.join("archives/sha256").join(format!("{digest}.harp"));
    let package_root = root.join("roots/sha256").join(&digest);
    fs::create_dir_all(
        archive_target
            .parent()
            .ok_or("package archive target has no parent")?,
    )
    .map_err(io_error)?;
    fs::create_dir_all(
        package_root
            .parent()
            .ok_or("package root target has no parent")?,
    )
    .map_err(io_error)?;

    let created_archive = install_archive_blob(archive, &archive_target, &digest)?;
    let manifest = match PackageManifest::read_archive(&archive_target) {
        Ok(manifest) => manifest,
        Err(error) => {
            if created_archive {
                let _ = fs::remove_file(&archive_target);
            }
            return Err(error.to_string());
        }
    };

    if package_root.exists() {
        validate_installed_root(&package_root, &manifest)?;
    } else {
        extract_package_root(&archive_target, &package_root, &manifest, &digest)?;
    }

    let project = validate_installed_root(&package_root, &manifest)?;
    let coordinate = if manifest.name.is_some() {
        project::normalize_coordinate(&manifest.identity).map_err(|error| {
            format!(
                "package/invalid-manifest: package identity {} is invalid: {error}",
                manifest.identity
            )
        })?
    } else {
        project::normalize_coordinate(&project.id)?
    };
    let (tap, package) = split_coordinate(&coordinate)?;
    let mut parts = package.split('/');
    let owner = parts
        .next()
        .ok_or_else(|| format!("invalid package coordinate: {coordinate}"))?;
    let name = parts
        .next()
        .ok_or_else(|| format!("invalid package coordinate: {coordinate}"))?;
    if parts.next().is_some() {
        return Err(format!("invalid package coordinate: {coordinate}"));
    }
    let registration = root
        .join("packages")
        .join(tap)
        .join(owner)
        .join(name)
        .join(format!("{}.edn", manifest.version));
    let registration_source = format!(
        "{{:coordinate {} :version {} :archive-sha256 {} :root {}}}\n",
        Form::String(coordinate).to_string(),
        Form::String(manifest.version.to_string()).to_string(),
        Form::String(format!("sha256:{digest}")).to_string(),
        Form::String(package_root.display().to_string()).to_string()
    );
    write_atomic(&registration, registration_source.as_bytes())?;
    Ok(package_root)
}

fn install_archive_blob(source: &Path, target: &Path, digest: &str) -> Result<bool, String> {
    if target.exists() {
        let actual = file_sha256(target)?;
        if actual != digest {
            return Err(format!(
                "package/digest-mismatch: cached archive {} has digest sha256:{actual}, expected sha256:{digest}",
                target.display()
            ));
        }
        return Ok(false);
    }

    let temporary = target.with_file_name(format!(
        ".{}.tmp-{}",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("archive"),
        std::process::id()
    ));
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(io_error)?;
    }
    fs::copy(source, &temporary).map_err(io_error)?;
    let copied_digest = file_sha256(&temporary)?;
    if copied_digest != digest {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "package/digest-mismatch: copied archive has digest sha256:{copied_digest}, expected sha256:{digest}"
        ));
    }
    match fs::rename(&temporary, target) {
        Ok(()) => Ok(true),
        Err(error) if target.exists() => {
            let _ = fs::remove_file(&temporary);
            let actual = file_sha256(target)?;
            if actual == digest {
                Ok(false)
            } else {
                Err(format!(
                    "cannot install archive {} after concurrent write: {error}",
                    target.display()
                ))
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.to_string())
        }
    }
}

fn extract_package_root(
    archive: &Path,
    package_root: &Path,
    manifest: &PackageManifest,
    digest: &str,
) -> Result<(), String> {
    let parent = package_root
        .parent()
        .ok_or("package root target has no parent")?;
    let scratch = parent.join(format!(".{digest}.tmp-{}", std::process::id()));
    if scratch.exists() {
        fs::remove_dir_all(&scratch).map_err(io_error)?;
    }
    fs::create_dir_all(&scratch).map_err(io_error)?;

    let result = (|| {
        let mut zip = ZipArchive::new(File::open(archive).map_err(io_error)?).map_err(zip_error)?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(zip_error)?;
            let raw = entry.name().to_owned();
            let canonical = if entry.is_dir() {
                raw.strip_suffix('/').unwrap_or(&raw)
            } else {
                &raw
            };
            let relative = entry
                .enclosed_name()
                .ok_or_else(|| format!("archive contains an unsafe path {raw}"))?
                .to_path_buf();
            validate_relative_path(&relative)?;
            if canonical.is_empty()
                || canonical.contains('\\')
                || canonical.split('/').any(str::is_empty)
                || relative
                    .components()
                    .any(|component| matches!(component, std::path::Component::CurDir))
            {
                return Err(format!(
                    "package/invalid-manifest: archive contains non-canonical path {raw}"
                ));
            }
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & UNIX_FILE_TYPE_MASK == UNIX_SYMLINK_TYPE)
            {
                return Err(format!(
                    "package/invalid-manifest: archive entry must not be a symbolic link: {}",
                    relative.display()
                ));
            }
            if entry.is_dir() {
                fs::create_dir_all(scratch.join(relative)).map_err(io_error)?;
                continue;
            }
            let output = scratch.join(relative);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            let mut file = File::create(output).map_err(io_error)?;
            std::io::copy(&mut entry, &mut file).map_err(io_error)?;
        }
        manifest
            .verify_files_at(&scratch)
            .map_err(|error| error.to_string())?;
        validate_installed_root(&scratch, manifest)?;
        Ok::<(), String>(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&scratch);
        return Err(error);
    }
    if let Err(error) = fs::rename(&scratch, package_root) {
        let _ = fs::remove_dir_all(&scratch);
        return Err(error.to_string());
    }
    Ok(())
}

fn validate_installed_root(
    package_root: &Path,
    manifest: &PackageManifest,
) -> Result<Project, String> {
    manifest
        .verify_files_at(package_root)
        .map_err(|error| error.to_string())?;
    verify_installed_entry_set(package_root, manifest)?;
    let installed_manifest = PackageManifest::read(&package_root.join("package.edn"))
        .map_err(|error| error.to_string())?;
    if installed_manifest.canonical_edn() != manifest.canonical_edn() {
        return Err(
            "package/invalid-manifest: installed package.edn differs from the verified archive index"
                .into(),
        );
    }
    let project = read_project(package_root)?;
    let project_coordinate = if manifest.name.is_none() {
        Some(project::normalize_coordinate(&project.id)?)
    } else {
        None
    };
    let manifest_coordinate =
        project::normalize_coordinate(&manifest.identity).map_err(|error| {
            format!(
                "package/invalid-manifest: package identity {} is invalid: {error}",
                manifest.identity
            )
        })?;
    if let Some(coordinate) = project_coordinate {
        if coordinate != manifest_coordinate {
            return Err(format!(
                "package/invalid-manifest: project identity {coordinate} does not match package identity {manifest_coordinate}"
            ));
        }
    }
    if project.version != manifest.version {
        return Err(format!(
            "package/invalid-manifest: project version {} does not match package version {}",
            project.version, manifest.version
        ));
    }
    Ok(project)
}

fn verify_installed_entry_set(
    package_root: &Path,
    manifest: &PackageManifest,
) -> Result<(), String> {
    let mut pending = vec![package_root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "package/invalid-manifest: installed package contains symbolic link {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(format!(
                    "package/invalid-manifest: installed package contains non-file entry {}",
                    path.display()
                ));
            }
            let relative = path
                .strip_prefix(package_root)
                .map_err(|_| "installed package path escapes its root".to_owned())?;
            validate_relative_path(relative)?;
            if relative != Path::new("package.edn") && !manifest.files.contains_key(relative) {
                return Err(format!(
                    "package/invalid-manifest: installed package contains undeclared file {}",
                    relative.display()
                ));
            }
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("package registration has no parent")?;
    fs::create_dir_all(parent).map_err(io_error)?;
    if path.exists() {
        let existing = fs::read(path).map_err(io_error)?;
        if existing == bytes {
            return Ok(());
        }
        return Err(format!(
            "package/registration-conflict: {} already records different package state",
            path.display()
        ));
    }
    let temporary = path.with_file_name(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("registration"),
        std::process::id()
    ));
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(io_error)?;
    }
    fs::write(&temporary, bytes).map_err(io_error)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) if path.exists() => {
            let _ = fs::remove_file(&temporary);
            if fs::read(path).map_err(io_error)? == bytes {
                Ok(())
            } else {
                Err(format!(
                    "package/registration-conflict: {} changed during registration: {error}",
                    path.display()
                ))
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.to_string())
        }
    }
}

pub(super) fn json_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}
