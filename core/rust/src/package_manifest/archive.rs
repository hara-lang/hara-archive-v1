use super::*;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use zip::ZipArchive;

const MAX_PACKAGE_MANIFEST_BYTES: u64 = 1024 * 1024;
const UNIX_FILE_TYPE_MASK: u32 = 0o170000;
const UNIX_SYMLINK_TYPE: u32 = 0o120000;

pub(super) fn read_archive(path: &Path) -> Result<PackageManifest, PackageManifestError> {
    let file = File::open(path).map_err(|error| {
        PackageManifestError::new(
            "package/invalid-manifest",
            format!("cannot open {}: {error}", path.display()),
        )
    })?;
    let mut archive = ZipArchive::new(file).map_err(|error| {
        PackageManifestError::new(
            "package/invalid-manifest",
            format!("cannot read {} as a .harp archive: {error}", path.display()),
        )
    })?;
    let source = read_manifest_source(&mut archive)?;
    let manifest = PackageManifest::parse(&source)?;
    verify_archive_files(&mut archive, &manifest)?;
    verify_archive_catalog(&mut archive, &manifest)?;
    Ok(manifest)
}

fn verify_archive_catalog(
    archive: &mut ZipArchive<File>,
    manifest: &PackageManifest,
) -> Result<(), PackageManifestError> {
    let Some(descriptor) = &manifest.schema_catalog else {
        return Ok(());
    };
    let name = descriptor.path.to_str().ok_or_else(|| {
        PackageManifestError::new("package/catalog-missing", "catalog path is not UTF-8")
    })?;
    let mut entry = archive.by_name(name).map_err(|error| {
        PackageManifestError::new(
            "package/catalog-missing",
            format!("archive is missing declared catalog {name}: {error}"),
        )
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|error| {
        PackageManifestError::new(
            "package/catalog-invalid",
            format!("cannot read catalog {name}: {error}"),
        )
    })?;
    manifest.admit_catalog_bytes(&bytes).map(|_| ())
}

fn read_manifest_source(archive: &mut ZipArchive<File>) -> Result<String, PackageManifestError> {
    let mut seen = BTreeSet::new();
    let mut manifest = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            PackageManifestError::new(
                "package/invalid-manifest",
                format!("cannot read archive entry {index}: {error}"),
            )
        })?;
        let relative = safe_entry_path(&entry)?;
        if !seen.insert(relative.clone()) {
            return Err(PackageManifestError::new(
                "package/invalid-manifest",
                format!("archive contains duplicate path {}", relative.display()),
            ));
        }
        reject_symlink(&entry, &relative)?;
        if relative == Path::new("package.edn") {
            if entry.is_dir() {
                return Err(PackageManifestError::new(
                    "package/invalid-manifest",
                    "archive package.edn must be a regular file",
                ));
            }
            if entry.size() > MAX_PACKAGE_MANIFEST_BYTES {
                return Err(PackageManifestError::new(
                    "package/invalid-manifest",
                    format!(
                        "archive package.edn is {} bytes; maximum is {}",
                        entry.size(),
                        MAX_PACKAGE_MANIFEST_BYTES
                    ),
                ));
            }
            let mut source = String::new();
            entry.read_to_string(&mut source).map_err(|error| {
                PackageManifestError::new(
                    "package/invalid-manifest",
                    format!("archive package.edn is not UTF-8 text: {error}"),
                )
            })?;
            manifest = Some(source);
        }
    }
    manifest.ok_or_else(|| {
        PackageManifestError::new("package/invalid-manifest", "archive is missing package.edn")
    })
}

fn verify_archive_files(
    archive: &mut ZipArchive<File>,
    manifest: &PackageManifest,
) -> Result<(), PackageManifestError> {
    if manifest.files.contains_key(Path::new("package.edn")) {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            ":files must not declare the self-referential package.edn index",
        ));
    }

    let mut seen = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            PackageManifestError::new(
                "package/invalid-manifest",
                format!("cannot read archive entry {index}: {error}"),
            )
        })?;
        let relative = safe_entry_path(&entry)?;
        reject_symlink(&entry, &relative)?;
        if entry.is_dir() || relative == Path::new("package.edn") {
            continue;
        }
        if !manifest.files.contains_key(&relative) {
            return Err(PackageManifestError::new(
                "package/invalid-manifest",
                format!("archive contains undeclared file {}", relative.display()),
            ));
        }
        manifest.verify_file_reader(&relative, &mut entry)?;
        seen.insert(relative);
    }

    for relative in manifest.files.keys() {
        if !seen.contains(relative) {
            return Err(PackageManifestError::new(
                "package/missing-artifact",
                format!("archive is missing declared file {}", relative.display()),
            ));
        }
    }
    Ok(())
}

fn safe_entry_path(entry: &zip::read::ZipFile<'_>) -> Result<PathBuf, PackageManifestError> {
    let raw = entry.name();
    let canonical = if entry.is_dir() {
        raw.strip_suffix('/').unwrap_or(raw)
    } else {
        raw
    };
    let relative = entry.enclosed_name().ok_or_else(|| {
        PackageManifestError::new(
            "package/invalid-manifest",
            format!("archive contains unsafe path {raw}"),
        )
    })?;
    if canonical.is_empty()
        || canonical.contains('\\')
        || canonical.split('/').any(str::is_empty)
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir
                    | std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("archive contains non-canonical path {raw}"),
        ));
    }
    Ok(relative.to_path_buf())
}

fn reject_symlink(
    entry: &zip::read::ZipFile<'_>,
    relative: &Path,
) -> Result<(), PackageManifestError> {
    if entry
        .unix_mode()
        .is_some_and(|mode| mode & UNIX_FILE_TYPE_MASK == UNIX_SYMLINK_TYPE)
    {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!(
                "archive entry must not be a symbolic link: {}",
                relative.display()
            ),
        ));
    }
    Ok(())
}
