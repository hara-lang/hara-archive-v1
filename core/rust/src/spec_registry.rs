//! Workspace-independent access to the canonical `hara-specs-registry` checkout.
//!
//! Registry consumers may run from Cargo, Maven, an editor, or a packaged
//! checkout. They must not encode one caller's working-directory layout.

use std::env;
use std::path::{Path, PathBuf};

const REGISTRY_ENV: &str = "HARA_SPECS_REGISTRY";
const WORKSPACE_ENV: &str = "HARA_WORKSPACE_ROOT";

/// Finds the canonical registry using explicit configuration before workspace
/// and ancestor discovery.
pub fn root() -> Option<PathBuf> {
    if let Some(configured) = nonempty_var(REGISTRY_ENV) {
        return valid_root(PathBuf::from(configured));
    }

    if let Some(workspace) = nonempty_var(WORKSPACE_ENV) {
        return valid_root(PathBuf::from(workspace).join("technology/hara-specs-registry"));
    }

    let mut starts = vec![PathBuf::from(env!("HARA_SOURCE_ROOT"))];
    if let Ok(current) = env::current_dir() {
        starts.push(current);
    }
    for start in starts {
        for cursor in start.ancestors() {
            for candidate in [
                cursor.join("hara-specs-registry"),
                cursor.join("technology/hara-specs-registry"),
            ] {
                if let Some(root) = valid_root(candidate) {
                    return Some(root);
                }
            }
        }
    }
    None
}

/// Resolves a registry-relative path when the registry is available.
pub fn resolve(relative: &str) -> Option<PathBuf> {
    let requested = Path::new(relative);
    if requested.is_absolute()
        || requested
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    root().map(|registry| registry.join(requested))
}

/// Resolves a required registry path and reports the configuration that was
/// missing instead of exposing a caller-specific relative path failure.
pub fn require(relative: &str) -> PathBuf {
    let path = resolve(relative).unwrap_or_else(|| {
        panic!(
            "cannot locate hara-specs-registry for `{relative}`; set {REGISTRY_ENV} or {WORKSPACE_ENV}"
        )
    });
    assert!(
        path.is_file(),
        "missing hara-specs-registry file: {}",
        path.display()
    );
    path
}

fn nonempty_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn valid_root(candidate: PathBuf) -> Option<PathBuf> {
    let root = candidate.canonicalize().ok()?;
    let has_manifest = root.join("spec-manifest.json").is_file();
    let has_index = root.join("registry-index.json").is_file();
    (root.is_dir() && (has_manifest || has_index)).then_some(root)
}
