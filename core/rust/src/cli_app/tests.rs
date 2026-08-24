use super::manifest::merge_sources;
use super::{
    merged_manifest_source, BASE_MANIFEST_SOURCE, EXTENSION_BIND_MANIFEST_SOURCE,
    EXTENSION_INSPECT_MANIFEST_SOURCE, EXTENSION_WIT_IMPORT_MANIFEST_SOURCE,
    EXTENSION_WIT_PROJECT_MANIFEST_SOURCE, PROJECT_BUILD_MANIFEST_SOURCE,
};
use crate::kernel::parse;

fn repo_text(relative: &str) -> Option<String> {
    crate::spec_registry::resolve(relative).and_then(|path| std::fs::read_to_string(path).ok())
}

#[test]
fn vendored_manifest_matches_specs_submodule_when_present() {
    let Some(submodule) = repo_text("00-unsorted/cli/draft/hara-cli.edn") else {
        return;
    };
    assert_eq!(submodule, BASE_MANIFEST_SOURCE);
}

#[test]
fn embedded_manifest_extensions_are_valid_and_idempotent() {
    for source in [
        BASE_MANIFEST_SOURCE,
        PROJECT_BUILD_MANIFEST_SOURCE,
        EXTENSION_INSPECT_MANIFEST_SOURCE,
        EXTENSION_BIND_MANIFEST_SOURCE,
        EXTENSION_WIT_IMPORT_MANIFEST_SOURCE,
        EXTENSION_WIT_PROJECT_MANIFEST_SOURCE,
        merged_manifest_source(),
    ] {
        parse(source).expect("embedded CLI manifest source must be valid EDN");
    }
    for source in [
        PROJECT_BUILD_MANIFEST_SOURCE,
        EXTENSION_INSPECT_MANIFEST_SOURCE,
        EXTENSION_BIND_MANIFEST_SOURCE,
        EXTENSION_WIT_IMPORT_MANIFEST_SOURCE,
        EXTENSION_WIT_PROJECT_MANIFEST_SOURCE,
    ] {
        assert_eq!(
            merge_sources(merged_manifest_source(), source).unwrap(),
            merged_manifest_source()
        );
    }
    for route in [
        ":tool.cli.route/project-build",
        ":tool.cli.route/extension-inspect",
        ":tool.cli.route/extension-bind",
        ":tool.cli.route/extension-wit-import",
        ":tool.cli.route/extension-wit-project",
    ] {
        assert!(merged_manifest_source().contains(route));
    }
}

#[test]
fn embedded_cli_inventory_contains_runtime_entrypoints() {
    for namespace in ["tool.cli.main", "tool.cli.handlers", "tool.cli.model"] {
        assert!(
            crate::CLI_BOOTSTRAP_INVENTORY.contains(&namespace),
            "missing CLI bootstrap entrypoint {namespace}"
        );
    }
    let foundation = crate::FOUNDATION_BOOTSTRAP_INVENTORY
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let embedded = crate::EMBEDDED_CLI_RESOURCES
        .iter()
        .map(|(namespace, _, _)| *namespace)
        .collect::<std::collections::HashSet<_>>();
    assert!(embedded.is_disjoint(&foundation));
    assert!(crate::CLI_BOOTSTRAP_INVENTORY
        .iter()
        .all(|namespace| foundation.contains(namespace) || embedded.contains(namespace)));
}
