//! Deterministic local package operations for the `hara package` command.
//!
//! Network reconciliation deliberately does not live here yet: package roots
//! are only activated after a registry and identity client has verified them.

use crate::kernel::{parse, Form};
use crate::project::{self, Project};
use crate::tap::{self, Tap};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub use crate::package_catalog::{catalog_from_lock, LockedPackage};

mod archive;
mod install;
use archive::*;
#[cfg(test)]
use install::install_archive_at;
use install::{install_archive, json_string, validate_recipe};

/// Capability adapter used by the Hara-owned CLI policy. These functions
/// expose package mechanics without parsing command-line arguments or writing
/// user-facing output.
pub fn check_path(input: &Path) -> Result<(String, String), String> {
    let project = read_project(input)?;
    Ok((project.id, project.version.to_string()))
}

pub fn build_path(input: &Path, output: Option<&Path>) -> Result<PathBuf, String> {
    let project = read_project(input)?;
    let destination = output.map(Path::to_path_buf).unwrap_or_else(|| {
        project.root.join("target").join(format!(
            "{}-{}.harp",
            archive_name(&project.id),
            project.version
        ))
    });
    build_archive(&project, &destination)?;
    Ok(destination)
}

pub fn inspect_path(archive: &Path) -> Result<String, String> {
    inspect_archive(archive)
}

pub fn install_path(input: &Path) -> Result<PathBuf, String> {
    let archive = if input.is_dir() {
        build_path(input, None)?
    } else {
        input.to_path_buf()
    };
    install_archive(&archive)
}

/// Handles the public `hara package` command group.
pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("check") => {
            let root = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let project = read_project(&root)?;
            println!("package check: {} {}", project.id, project.version);
            Ok(())
        }
        Some("build") => {
            let root = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let project = read_project(&root)?;
            let output = args
                .iter()
                .position(|arg| arg == "--output")
                .and_then(|index| args.get(index + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    project.root.join("target").join(format!(
                        "{}-{}.harp",
                        archive_name(&project.id),
                        project.version
                    ))
                });
            build_archive(&project, &output)?;
            println!("package build: {}", output.display());
            Ok(())
        }
        Some("inspect") => {
            let archive = args
                .get(1)
                .ok_or_else(|| "hara package inspect requires ARCHIVE.harp".to_owned())?;
            println!("{}", inspect_archive(Path::new(archive))?);
            Ok(())
        }
        Some("install") => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let archive = if input.is_dir() {
                let project = read_project(&input)?;
                let output = project.root.join("target").join(format!(
                    "{}-{}.harp",
                    archive_name(&project.id),
                    project.version
                ));
                build_archive(&project, &output)?;
                output
            } else {
                input
            };
            let installed = install_archive(&archive)?;
            println!("package install: {}", installed.display());
            Ok(())
        }
        Some("publish") => publish(&args[1..]),
        Some("tap") => tap_command(&args[1..]),
        Some("registry") => registry_command(&args[1..]),
        Some("sync") | Some("add") | Some("remove") | Some("update") | Some("search")
        | Some("info") => Err(format!(
            "hara package {} requires a configured GitHub registry and identity client; local package commands available now: check, build, inspect",
            args[0]
        )),
        Some("--help") | Some("-h") | None => {
            println!(
                "hara package <check|build|inspect|sync|add|remove|update|publish|tap|search|info>\n\n\
                 check [PATH]                 validate project.edn and recipe\n\
                 build [PATH] [--output PATH] build deterministic .harp\n\
                 inspect ARCHIVE.harp         print package.edn\n\
                 install [PATH|ARCHIVE.harp]  install into HARA_DIST_HOME or ~/.hara/dist\n\
                 tap bootstrap official       install the official profile\n\
                 tap init NAME --registry PATH --identity PATH --identity-root-key ED25519_HEX\n\
                 tap add NAME --registry URL --identity URL --identity-key SHA256\n\
                 tap mirror add NAME [--registry URL] [--identity URL]\n\
                 tap list|remove NAME|verify NAME\n\
                 publish [--tap official] [--dry-run] [PATH]"
            );
            Ok(())
        }
        Some(command) => Err(format!("unknown package command: {command}")),
    }
}

fn read_project(path: &Path) -> Result<Project, String> {
    project::read(path)
}

fn registry_command(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("verify-request") => {
            let request = PathBuf::from(required_option(args, "--request")?);
            let identity = PathBuf::from(required_option(args, "--identity")?);
            verify_registry_request_paths(&request, &identity)?;
            println!("registry request verified: {}", request.display());
            Ok(())
        }
        _ => {
            Err("usage: hara package registry verify-request --request PATH --identity PATH".into())
        }
    }
}

pub fn verify_registry_request_paths(request: &Path, identity: &Path) -> Result<(), String> {
    let policy = fs::read_to_string(identity)
        .map_err(|error| format!("cannot read {}: {error}", identity.display()))?;
    let Form::Map(policy) = parse(&policy)? else {
        return Err("identity policy must be an EDN map".into());
    };
    let trust = policy
        .iter()
        .find(|(key, _)| matches!(key, Form::Keyword(name) if name == "identity/trust"))
        .map(|(_, value)| value);
    if !matches!(trust, Some(Form::Keyword(mode)) if mode == "github-governed") {
        return Err("registry bootstrap verifier requires :identity/trust :github-governed".into());
    }
    let intent_path = fs::read_dir(request)
        .map_err(|error| format!("cannot read {}: {error}", request.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".publisher-intent.edn"))
        })
        .ok_or("request is missing publisher intent")?;
    let intent = fs::read_to_string(&intent_path).map_err(io_error)?;
    let Form::Map(entries) = parse(&intent)? else {
        return Err("publisher intent must be an EDN map".into());
    };
    for key in [
        "intent/format",
        "tap",
        "coordinate",
        "version",
        "repository",
        "tag",
        "commit",
        "archive-sha256",
        "identity-revision",
    ] {
        if !entries
            .iter()
            .any(|(candidate, _)| matches!(candidate, Form::Keyword(name) if name == key))
        {
            return Err(format!("publisher intent is missing :{key}"));
        }
    }
    Ok(())
}

pub fn tap_command(args: &[String]) -> Result<(), String> {
    let root = tap::config_root();
    match args.first().map(String::as_str) {
        Some("add") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap add requires NAME".to_owned())?;
            let registry = option_values(args, "--registry");
            let identity = option_values(args, "--identity");
            let identity_key = option_value(args, "--identity-key")?;
            tap::add(
                &root,
                Tap {
                    name: name.clone(),
                    registry,
                    identity,
                    identity_key,
                    trust: tap::TrustMode::SignedRoot,
                },
            )?;
            println!("trusted tap {name}");
            Ok(())
        }
        Some("bootstrap") => {
            let profile = args
                .get(1)
                .ok_or_else(|| "tap bootstrap requires PROFILE".to_owned())?;
            let tap = tap::bootstrap(&root, profile)?;
            println!("bootstrapped tap {} (GitHub-governed)", tap.name);
            Ok(())
        }
        Some("mirror") if args.get(1).map(String::as_str) == Some("add") => {
            let name = args
                .get(2)
                .ok_or_else(|| "tap mirror add requires NAME".to_owned())?;
            let tap = tap::add_mirror(
                &root,
                name,
                optional_option(args, "--registry"),
                optional_option(args, "--identity"),
            )?;
            println!(
                "updated tap {} registry={} identity={}",
                tap.name,
                tap.registry.join(","),
                tap.identity.join(",")
            );
            Ok(())
        }
        Some("init") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap init requires NAME".to_owned())?;
            let registry = PathBuf::from(required_option(args, "--registry")?);
            let identity = PathBuf::from(required_option(args, "--identity")?);
            let root_key = required_option(args, "--identity-root-key")?;
            let initialized = tap::initialize(name, &registry, &identity, &root_key)?;
            tap::add(&root, initialized.tap)?;
            println!("initialized tap {name}");
            println!("identity-root fingerprint: {}", initialized.fingerprint);
            println!("scaffolded registry: {}", registry.display());
            println!("scaffolded identity: {}", identity.display());
            Ok(())
        }
        Some("remove") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap remove requires NAME".to_owned())?;
            tap::remove(&root, name)?;
            println!("removed tap {name}");
            Ok(())
        }
        Some("list") => {
            for tap in tap::load(&root)?.values() {
                println!(
                    "{} registry={} identity={}",
                    tap.name,
                    tap.registry.join(","),
                    tap.identity.join(",")
                );
            }
            Ok(())
        }
        Some("verify") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap verify requires NAME".to_owned())?;
            let tap = tap::trusted(&root, name)?;
            let scratch = scratch("verify")?;
            let result = tap::fetch_verified_policy(&tap, &scratch);
            let _ = fs::remove_dir_all(&scratch);
            let policy = result?;
            println!("tap verify: {} identity={}", tap.name, policy.revision);
            Ok(())
        }
        _ => {
            Err("usage: hara package tap <bootstrap|init|add|mirror add|remove|list|verify>".into())
        }
    }
}

fn publish(args: &[String]) -> Result<(), String> {
    let tap_name = optional_option(args, "--tap")
        .map(|name| {
            if name == "official" {
                "hara".into()
            } else {
                name
            }
        })
        .unwrap_or_else(|| "hara".into());
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let path = args
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with('-') && *arg != &tap_name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    println!("{}", publish_path(&path, &tap_name, dry_run)?);
    Ok(())
}

pub fn publish_path(path: &Path, tap_name: &str, dry_run: bool) -> Result<String, String> {
    let tap_name = if tap_name == "official" {
        "hara"
    } else {
        tap_name
    };
    let project = read_project(path)?;
    let coordinate = project::normalize_coordinate(&project.id)?;
    let (coordinate_tap, _) = split_coordinate(&coordinate)?;
    if coordinate_tap != tap_name {
        return Err(format!(
            "project id {} belongs to tap {coordinate_tap}, not {tap_name}",
            project.id
        ));
    }
    let trusted_tap = tap::trusted_or_builtin(&tap::config_root(), &tap_name)?;
    let scratch = scratch("publish")?;
    let result = publish_inner(&project, &trusted_tap, dry_run, &scratch);
    let _ = fs::remove_dir_all(&scratch);
    result
}

fn publish_inner(
    project: &Project,
    trusted_tap: &Tap,
    dry_run: bool,
    scratch_root: &Path,
) -> Result<String, String> {
    let policy = tap::fetch_verified_policy(trusted_tap, scratch_root)?;
    let tag = format!("v{}", project.version);
    tap::git(&project.root, ["tag", "-v", &tag])
        .map_err(|error| format!("publish requires a valid signed tag {tag}: {error}"))?;
    let commit = tap::git(&project.root, ["rev-list", "-n", "1", &tag])?;
    let repository = tap::git(&project.root, ["config", "--get", "remote.origin.url"])?;
    let recipe = validate_recipe(project)?;
    build_archive(project, &scratch_root.join("publish.harp"))?;
    let recipe_sha256 = file_sha256(&recipe)?;
    let coordinate = project::normalize_coordinate(&project.id)?;
    let intent = tap::canonical_recipe_intent(
        &coordinate,
        &project.version.to_string(),
        &repository,
        &tag,
        &commit,
        &recipe_sha256,
        &trusted_tap.name,
        &policy.revision,
    );
    let (key_id, signature) = tap::sign(intent.as_bytes())?;
    tap::authorize(&policy, &key_id, &coordinate, intent.as_bytes(), &signature)?;
    if dry_run {
        return Ok(format!(
            "publish recipe verified: {} {} tap={} recipe=sha256:{}",
            coordinate, project.version, trusted_tap.name, recipe_sha256
        ));
    }
    let endpoint = trusted_tap
        .registry
        .first()
        .ok_or("official tap has no publication endpoint")?;
    let body = format!(
        "{{\"intent\":{},\"key_id\":\"{}\",\"signature\":\"{}\"}}",
        json_string(&intent),
        key_id,
        signature
    );
    let output = std::process::Command::new("curl")
        .args([
            "--fail-with-body",
            "--silent",
            "--show-error",
            "-H",
            "content-type: application/json",
            "--data-binary",
            &body,
            &format!("{}/v1/publications", endpoint.trim_end_matches('/')),
        ])
        .output()
        .map_err(|error| format!("cannot start publication client: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "publication request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(format!(
        "publish requested: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

fn option_value(args: &[String], flag: &str) -> Result<String, String> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("publish requires {flag}"))?;
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}
fn required_option(args: &[String], flag: &str) -> Result<String, String> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("tap init requires {flag}"))?;
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}
fn option_values(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, value)| value.as_str() == flag)
        .filter_map(|(index, _)| args.get(index + 1).cloned())
        .collect()
}
fn optional_option(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1).cloned())
}
fn split_coordinate(value: &str) -> Result<(&str, &str), String> {
    let (tap, package) = value
        .split_once(':')
        .ok_or_else(|| format!("package coordinate must use TAP:owner/name: {value}"))?;
    if tap.is_empty() || package.is_empty() || package.contains(':') {
        return Err(format!("invalid tap-qualified package coordinate: {value}"));
    }
    Ok((tap, package))
}
fn scratch(label: &str) -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join(format!("hara-{label}-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(io_error)?;
    }
    fs::create_dir_all(&root).map_err(io_error)?;
    Ok(root)
}
fn file_sha256(path: &Path) -> Result<String, String> {
    Ok(hex(&Sha256::digest(fs::read(path).map_err(io_error)?)))
}

#[cfg(test)]
mod tests;
