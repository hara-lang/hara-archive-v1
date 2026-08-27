use super::*;
use crate::kernel::{parse, Form};
use std::path::Component;

pub(super) fn parse_manifest(source: &str) -> Result<PackageManifest, PackageManifestError> {
    let form = parse(source).map_err(|error| {
        PackageManifestError::new(
            "package/invalid-manifest",
            format!("package.edn is not valid EDN: {error}"),
        )
    })?;
    let root = expect_map(&form, "package.edn")?;
    let format = required_string(root, "harp/format", "package.edn")?;
    if format != PACKAGE_FORMAT {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("unsupported :harp/format {format}"),
        ));
    }

    let package = expect_map(required_value(root, "package", "package.edn")?, ":package")?;
    let identity = required_string(package, "identity", ":package")?;
    let name = optional_value(package, "name", ":package")?
        .map(|value| {
            let name = identifier(value, ":package :name")?;
            if name.is_empty() {
                return Err(PackageManifestError::new(
                    "package/invalid-manifest",
                    ":package :name must be non-empty",
                ));
            }
            Ok(name)
        })
        .transpose()?;
    let version_source = required_string(package, "version", ":package")?;
    let version = Version::parse(&version_source).map_err(|error| {
        PackageManifestError::new(
            "package/invalid-manifest",
            format!("invalid :package :version {version_source}: {error}"),
        )
    })?;
    let provenance = optional_value(package, "provenance", ":package")?
        .map(parse_provenance)
        .transpose()?;
    let files = parse_files(required_value(root, "files", "package.edn")?)?;

    let bytecode = optional_value(root, "bytecode", "package.edn")?
        .map(|value| parse_bytecode(value, &files))
        .transpose()?;

    let schema_catalog = optional_value(root, "schema/catalog", "package.edn")?
        .map(|value| parse_schema_catalog(value, &files))
        .transpose()?;

    if let Some(descriptor) = optional_value(root, "descriptor", "package.edn")? {
        validate_descriptor(descriptor)?;
    }

    let wasm_imports = optional_value(root, "wasm-imports", "package.edn")?
        .map(|value| parse_wasm_imports(value, &files))
        .transpose()?
        .unwrap_or_default();
    let flavors = optional_value(root, "flavors", "package.edn")?
        .map(|value| parse_flavors(value, &files))
        .transpose()?
        .unwrap_or_default();
    if (!wasm_imports.is_empty() || !flavors.is_empty()) && provenance.is_none() {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            "packages with Wasm imports or host flavors require :package :provenance",
        ));
    }

    Ok(PackageManifest {
        format,
        identity,
        name,
        version,
        provenance,
        files,
        bytecode,
        schema_catalog,
        wasm_imports,
        flavors,
        canonical_edn: canonical_form(&form).to_string(),
    })
}

fn parse_bytecode(
    value: &Form,
    files: &BTreeMap<PathBuf, PackageFile>,
) -> Result<PackageBytecode, PackageManifestError> {
    let entries = expect_map(value, ":bytecode")?;
    let format = required_string(entries, "format", ":bytecode")?;
    if format != PACKAGE_FORMAT {
        return Err(PackageManifestError::new(
            "package/invalid-bytecode",
            format!("unsupported :bytecode :format {format}"),
        ));
    }
    let path = parse_relative_path(&required_string(entries, "path", ":bytecode")?)?;
    let sha256 = required_string(entries, "sha256", ":bytecode")?;
    validate_sha256(&sha256)?;
    let file = files.get(&path).ok_or_else(|| {
        PackageManifestError::new(
            "package/bytecode-missing",
            format!(
                ":bytecode :path is not declared in :files: {}",
                path.display()
            ),
        )
    })?;
    if file.sha256 != sha256 {
        return Err(PackageManifestError::new(
            "package/bytecode-digest-mismatch",
            format!(
                "{} declares {}, but :files declares {}",
                path.display(),
                sha256,
                file.sha256
            ),
        ));
    }
    Ok(PackageBytecode {
        format,
        path,
        sha256,
    })
}

fn parse_schema_catalog(
    value: &Form,
    files: &BTreeMap<PathBuf, PackageFile>,
) -> Result<PackageCatalogDescriptor, PackageManifestError> {
    let entries = expect_map(value, ":schema/catalog")?;
    let format = required_string(entries, "format", ":schema/catalog")?;
    let path = parse_relative_path(&required_string(entries, "path", ":schema/catalog")?)?;
    let sha256 = required_string(entries, "sha256", ":schema/catalog")?;
    validate_sha256(&sha256)?;
    let file = files.get(&path).ok_or_else(|| {
        PackageManifestError::new(
            "package/catalog-missing",
            format!(
                ":schema/catalog :path is not declared in :files: {}",
                path.display()
            ),
        )
    })?;
    if file.sha256 != sha256 {
        return Err(PackageManifestError::new(
            "package/catalog-digest-mismatch",
            format!(
                "{} declares {}, but :files declares {}",
                path.display(),
                sha256,
                file.sha256
            ),
        ));
    }
    Ok(PackageCatalogDescriptor {
        format,
        path,
        sha256,
    })
}

fn parse_provenance(value: &Form) -> Result<PackageProvenance, PackageManifestError> {
    let entries = expect_map(value, ":package :provenance")?;
    let repository = required_string(entries, "repository", ":package :provenance")?;
    let commit = required_string(entries, "commit", ":package :provenance")?;
    if !matches!(commit.len(), 40 | 64)
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            ":package :provenance :commit must be a canonical 40- or 64-character hexadecimal digest",
        ));
    }
    Ok(PackageProvenance { repository, commit })
}

fn parse_files(value: &Form) -> Result<BTreeMap<PathBuf, PackageFile>, PackageManifestError> {
    let entries = expect_map(value, ":files")?;
    let mut files = BTreeMap::new();
    for (path, declaration) in entries {
        let Form::String(path) = path else {
            return Err(PackageManifestError::new(
                "package/invalid-manifest",
                ":files keys must be archive-relative strings",
            ));
        };
        let path = parse_relative_path(path)?;
        let declaration = expect_map(declaration, ":files entry")?;
        let sha256 = required_string(declaration, "sha256", ":files entry")?;
        validate_sha256(&sha256)?;
        let size = match required_value(declaration, "size", ":files entry")? {
            Form::Number(value) if *value >= 0 => *value as u64,
            _ => {
                return Err(PackageManifestError::new(
                    "package/invalid-manifest",
                    ":files entry :size must be a non-negative integer",
                ));
            }
        };
        if files
            .insert(path.clone(), PackageFile { sha256, size })
            .is_some()
        {
            return Err(PackageManifestError::new(
                "package/invalid-manifest",
                format!("duplicate :files path {}", path.display()),
            ));
        }
    }
    Ok(files)
}

fn parse_wasm_imports(
    value: &Form,
    files: &BTreeMap<PathBuf, PackageFile>,
) -> Result<BTreeMap<String, PackageVariant>, PackageManifestError> {
    let entries = expect_map(value, ":wasm-imports")?;
    let mut imports = BTreeMap::new();
    for (module, declaration) in entries {
        let module = parse_module_name(module)?;
        let variant = parse_artifact(declaration, files)?;
        if variant.artifact.artifact_type != PackageArtifactType::Wasm
            && variant.artifact.artifact_type != PackageArtifactType::Hta
        {
            return Err(PackageManifestError::new(
                "package/invalid-manifest",
                format!("Wasm import {module} must use :artifact/type :wasm or :hta"),
            ));
        }
        if imports.insert(module.clone(), variant).is_some() {
            return Err(PackageManifestError::new(
                "package/duplicate-wasm-import",
                format!("duplicate Wasm import {module}"),
            ));
        }
    }
    Ok(imports)
}

fn parse_flavors(
    value: &Form,
    files: &BTreeMap<PathBuf, PackageFile>,
) -> Result<BTreeMap<String, PackageVariant>, PackageManifestError> {
    let entries = expect_map(value, ":flavors")?;
    let mut flavors = BTreeMap::new();
    for (flavor, declaration) in entries {
        let flavor = match flavor {
            Form::Keyword(value) if value != "wasm" && !value.contains('/') => value.clone(),
            _ => {
                return Err(PackageManifestError::new(
                    "package/invalid-manifest",
                    ":flavors keys must be unqualified host keywords; :wasm is reserved",
                ));
            }
        };
        let variant = parse_artifact(declaration, files)?;
        if variant.artifact.artifact_type != PackageArtifactType::Jar {
            return Err(PackageManifestError::new(
                "package/invalid-manifest",
                format!(":{flavor} host flavor must use :artifact/type :jar"),
            ));
        }
        if flavors.insert(flavor.clone(), variant).is_some() {
            return Err(PackageManifestError::new(
                "package/duplicate-flavor",
                format!("duplicate :{flavor} host flavor"),
            ));
        }
    }
    Ok(flavors)
}

fn parse_module_name(value: &Form) -> Result<String, PackageManifestError> {
    match value {
        Form::Keyword(value) if !value.contains('/') && !value.is_empty() => Ok(value.clone()),
        Form::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(PackageManifestError::new(
            "package/invalid-manifest",
            ":wasm-imports keys must be non-empty module keywords or strings",
        )),
    }
}

fn parse_artifact(
    value: &Form,
    files: &BTreeMap<PathBuf, PackageFile>,
) -> Result<PackageVariant, PackageManifestError> {
    let entries = expect_map(value, "package artifact")?;
    let artifact_entries = expect_map(
        required_value(entries, "variant/artifact", "package artifact")?,
        ":variant/artifact",
    )?;
    let artifact_type =
        match required_identifier(artifact_entries, "artifact/type", ":variant/artifact")?.as_str()
        {
            "jar" => PackageArtifactType::Jar,
            "wasm" => PackageArtifactType::Wasm,
            "hta" => PackageArtifactType::Hta,
            other => {
                return Err(PackageManifestError::new(
                    "package/invalid-manifest",
                    format!("unsupported :artifact/type :{other}"),
                ));
            }
        };
    let artifact_path = parse_relative_path(&required_string(
        artifact_entries,
        "artifact/path",
        ":variant/artifact",
    )?)?;
    let sha256 = required_string(artifact_entries, "artifact/sha256", ":variant/artifact")?;
    validate_sha256(&sha256)?;
    let target = required_string(artifact_entries, "artifact/target", ":variant/artifact")?;
    let abi = required_string(artifact_entries, "artifact/abi", ":variant/artifact")?;
    let entry_point = required_string(
        artifact_entries,
        "artifact/entry-point",
        ":variant/artifact",
    )?;
    let file = files.get(&artifact_path).ok_or_else(|| {
        PackageManifestError::new(
            "package/missing-artifact",
            format!(
                ":artifact/path is not declared in :files: {}",
                artifact_path.display()
            ),
        )
    })?;
    if file.sha256 != sha256 {
        return Err(PackageManifestError::new(
            "package/digest-mismatch",
            format!(
                "{} declares {}, but :files declares {}",
                artifact_path.display(),
                sha256,
                file.sha256
            ),
        ));
    }

    let required_capabilities = parse_identifier_set(
        required_value(entries, "variant/required-capabilities", "package artifact")?,
        ":variant/required-capabilities",
    )?;
    let host_calls = optional_value(entries, "variant/host-calls", "package artifact")?
        .map(|value| parse_identifier_set(value, ":variant/host-calls"))
        .transpose()?
        .unwrap_or_default();
    let exports = optional_value(entries, "variant/exports", "package artifact")?
        .map(|value| parse_identifier_set(value, ":variant/exports"))
        .transpose()?
        .unwrap_or_default();
    let dependencies = optional_value(entries, "variant/dependencies", "package artifact")?
        .map(|value| {
            expect_map(value, ":variant/dependencies")?;
            Ok::<Form, PackageManifestError>(value.clone())
        })
        .transpose()?;
    let lifecycle = optional_value(entries, "variant/lifecycle", "package artifact")?
        .map(parse_lifecycle)
        .transpose()?;

    Ok(PackageVariant {
        artifact: PackageArtifact {
            artifact_type,
            path: artifact_path,
            sha256,
            target,
            abi,
            entry_point,
        },
        required_capabilities,
        host_calls,
        exports,
        dependencies,
        lifecycle,
    })
}

fn parse_lifecycle(value: &Form) -> Result<PackageLifecycle, PackageManifestError> {
    let entries = expect_map(value, ":variant/lifecycle")?;
    let load = required_identifier(entries, "lifecycle/load", ":variant/lifecycle")?;
    let close = required_identifier(entries, "lifecycle/close", ":variant/lifecycle")?;
    if load != "idempotent" || close != "idempotent" {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            ":variant/lifecycle load and close must be :idempotent",
        ));
    }
    let session_isolation =
        required_bool(entries, "lifecycle/session-isolation", ":variant/lifecycle")?;
    if !session_isolation {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            ":variant/lifecycle requires :lifecycle/session-isolation true",
        ));
    }
    let asynchronous = optional_value(entries, "lifecycle/async", ":variant/lifecycle")?
        .map(|value| parse_bool(value, ":lifecycle/async"))
        .transpose()?
        .unwrap_or(false);
    let cancellation = optional_value(entries, "lifecycle/cancellation", ":variant/lifecycle")?
        .map(|value| parse_bool(value, ":lifecycle/cancellation"))
        .transpose()?
        .unwrap_or(false);
    Ok(PackageLifecycle {
        load_idempotent: true,
        close_idempotent: true,
        session_isolation,
        asynchronous,
        cancellation,
    })
}

fn parse_identifier_set(
    value: &Form,
    context: &str,
) -> Result<BTreeSet<String>, PackageManifestError> {
    let Form::Set(values) = value else {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("{context} must be an EDN set"),
        ));
    };
    values
        .iter()
        .map(|value| identifier(value, context))
        .collect()
}

fn identifier(value: &Form, context: &str) -> Result<String, PackageManifestError> {
    match value {
        Form::Keyword(value) | Form::Symbol(value) | Form::String(value) if !value.is_empty() => {
            Ok(value.clone())
        }
        _ => Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("{context} must contain non-empty keywords, symbols, or strings"),
        )),
    }
}

fn required_identifier(
    entries: &[(Form, Form)],
    key: &str,
    context: &str,
) -> Result<String, PackageManifestError> {
    identifier(required_value(entries, key, context)?, &format!(":{key}"))
}

fn required_string(
    entries: &[(Form, Form)],
    key: &str,
    context: &str,
) -> Result<String, PackageManifestError> {
    match required_value(entries, key, context)? {
        Form::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("{context} :{key} must be a non-empty string"),
        )),
    }
}

fn required_bool(
    entries: &[(Form, Form)],
    key: &str,
    context: &str,
) -> Result<bool, PackageManifestError> {
    parse_bool(
        required_value(entries, key, context)?,
        &format!("{context} :{key}"),
    )
}

fn parse_bool(value: &Form, context: &str) -> Result<bool, PackageManifestError> {
    match value {
        Form::Bool(value) => Ok(*value),
        _ => Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("{context} must be a boolean"),
        )),
    }
}

fn expect_map<'a>(
    value: &'a Form,
    context: &str,
) -> Result<&'a [(Form, Form)], PackageManifestError> {
    match value {
        Form::Map(entries) => Ok(entries),
        _ => Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("{context} must be an EDN map"),
        )),
    }
}

fn required_value<'a>(
    entries: &'a [(Form, Form)],
    key: &str,
    context: &str,
) -> Result<&'a Form, PackageManifestError> {
    optional_value(entries, key, context)?.ok_or_else(|| {
        PackageManifestError::new(
            "package/invalid-manifest",
            format!("{context} is missing :{key}"),
        )
    })
}

fn optional_value<'a>(
    entries: &'a [(Form, Form)],
    key: &str,
    context: &str,
) -> Result<Option<&'a Form>, PackageManifestError> {
    let mut values = entries.iter().filter_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
    });
    let value = values.next();
    if values.next().is_some() {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("{context} contains duplicate :{key}"),
        ));
    }
    Ok(value)
}

fn parse_relative_path(value: &str) -> Result<PathBuf, PackageManifestError> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || value.contains('\\')
        || value.split('/').any(str::is_empty)
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
            )
        })
    {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("unsafe or noncanonical package path: {value}"),
        ));
    }
    Ok(path)
}

fn validate_sha256(value: &str) -> Result<(), PackageManifestError> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("digest must use sha256: prefix: {value}"),
        ));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PackageManifestError::new(
            "package/invalid-manifest",
            format!("digest must contain 64 lowercase hexadecimal characters: {value}"),
        ));
    }
    Ok(())
}

fn validate_descriptor(value: &Form) -> Result<(), PackageManifestError> {
    match value {
        Form::Map(entries) => {
            for (key, value) in entries {
                if let Some(key) = descriptor_key(key) {
                    let tail = key.rsplit('/').next().unwrap_or(&key);
                    if matches!(
                        tail,
                        "credential"
                            | "credentials"
                            | "classloader"
                            | "socket"
                            | "raw-socket"
                            | "native-handle"
                            | "native-pointer"
                    ) {
                        return Err(PackageManifestError::new(
                            "package/invalid-descriptor",
                            format!("runtime-neutral descriptor cannot contain :{key}"),
                        ));
                    }
                }
                validate_descriptor(value)?;
            }
        }
        Form::Set(values) | Form::Vector(values) | Form::List(values) => {
            for value in values {
                validate_descriptor(value)?;
            }
        }
        Form::Tagged(_, value) => validate_descriptor(value)?,
        Form::Metadata(metadata, value) => {
            validate_descriptor(metadata)?;
            validate_descriptor(value)?;
        }
        _ => {}
    }
    Ok(())
}

fn descriptor_key(value: &Form) -> Option<String> {
    match value {
        Form::Keyword(value) | Form::Symbol(value) | Form::String(value) => {
            Some(value.to_ascii_lowercase())
        }
        _ => None,
    }
}

fn canonical_form(value: &Form) -> Form {
    match value {
        Form::Nil => Form::Nil,
        Form::Bool(value) => Form::Bool(*value),
        Form::Number(value) => Form::Number(*value),
        Form::Float(value) => Form::Float(*value),
        Form::BigInteger(value) => Form::BigInteger(value.clone()),
        Form::Character(value) => Form::Character(*value),
        Form::Regex(value) => Form::Regex(value.clone()),
        Form::Tagged(tag, value) => Form::Tagged(tag.clone(), Box::new(canonical_form(value))),
        Form::Metadata(metadata, value) => Form::Metadata(
            Box::new(canonical_form(metadata)),
            Box::new(canonical_form(value)),
        ),
        Form::Symbol(value) => Form::Symbol(value.clone()),
        Form::Keyword(value) => Form::Keyword(value.clone()),
        Form::String(value) => Form::String(value.clone()),
        Form::Map(entries) => {
            let mut entries = entries
                .iter()
                .map(|(key, value)| (canonical_form(key), canonical_form(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| {
                left.0
                    .to_string()
                    .cmp(&right.0.to_string())
                    .then_with(|| left.1.to_string().cmp(&right.1.to_string()))
            });
            Form::Map(entries)
        }
        Form::Set(values) => {
            let mut values = values.iter().map(canonical_form).collect::<Vec<_>>();
            values.sort_by_key(ToString::to_string);
            Form::Set(values)
        }
        Form::Vector(values) => Form::Vector(values.iter().map(canonical_form).collect()),
        Form::List(values) => Form::List(values.iter().map(canonical_form).collect()),
    }
}
