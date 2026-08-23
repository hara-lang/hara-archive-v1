//! Shared CLI contracts for Hara runtimes.

mod manifest;
mod outcome;
#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::OnceLock;

pub use outcome::CliOutcome;

pub const BASE_MANIFEST_SOURCE: &str = include_str!("../resources/hara-cli.edn");
pub const PROJECT_BUILD_MANIFEST_SOURCE: &str =
    include_str!("../resources/hara-cli-project-build.edn");
pub const EXTENSION_INSPECT_MANIFEST_SOURCE: &str =
    include_str!("../resources/hara-cli-extension-inspect.edn");
pub const EXTENSION_BIND_MANIFEST_SOURCE: &str =
    include_str!("../resources/hara-cli-extension-bind.edn");
pub const EXTENSION_WIT_IMPORT_MANIFEST_SOURCE: &str =
    include_str!("../resources/hara-cli-extension-wit-import.edn");
pub const EXTENSION_WIT_PROJECT_MANIFEST_SOURCE: &str =
    include_str!("../resources/hara-cli-extension-wit-project.edn");

#[derive(Clone, Copy)]
pub struct ManifestSource;

pub const MANIFEST_SOURCE: ManifestSource = ManifestSource;

static MERGED_MANIFEST_SOURCE: OnceLock<String> = OnceLock::new();

impl fmt::Debug for ManifestSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(merged_manifest_source(), formatter)
    }
}

pub fn merged_manifest_source() -> &'static str {
    MERGED_MANIFEST_SOURCE
        .get_or_init(|| {
            let project =
                manifest::merge_sources(BASE_MANIFEST_SOURCE, PROJECT_BUILD_MANIFEST_SOURCE)
                    .expect("embedded project-build CLI manifest extension must be valid");
            let inspect = manifest::merge_sources(&project, EXTENSION_INSPECT_MANIFEST_SOURCE)
                .expect("embedded extension-inspect CLI manifest extension must be valid");
            let bind = manifest::merge_sources(&inspect, EXTENSION_BIND_MANIFEST_SOURCE)
                .expect("embedded extension-bind CLI manifest extension must be valid");
            let wit_import = manifest::merge_sources(&bind, EXTENSION_WIT_IMPORT_MANIFEST_SOURCE)
                .expect("embedded WIT import CLI manifest extension must be valid");
            manifest::merge_sources(&wit_import, EXTENSION_WIT_PROJECT_MANIFEST_SOURCE)
                .expect("embedded WIT projection CLI manifest extension must be valid")
        })
        .as_str()
}

/// Installs the immutable HAL namespace closure required by the native Hara
/// command router. The launcher applies this catalog after project resources so
/// an external project cannot replace the CLI that selected it.
pub fn install_embedded_cli_sources(runtime: &mut crate::Runtime) {
    for &(namespace, _, source) in crate::EMBEDDED_CLI_RESOURCES {
        runtime.register_resource(namespace, source);
    }
}
