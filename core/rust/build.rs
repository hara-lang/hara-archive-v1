use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn declared_namespace(source: &str, path: &Path) -> String {
    for line in source.lines() {
        let line = line.trim_start();
        let remainder = line
            .strip_prefix("(ns ")
            .or_else(|| line.strip_prefix("(ns+ "));
        if let Some(remainder) = remainder {
            let namespace = remainder
                .split(|character: char| character.is_whitespace() || character == ')')
                .next()
                .unwrap_or_default();
            if !namespace.is_empty() {
                return namespace.to_owned();
            }
        }
    }
    panic!(
        "{} does not declare an ns or ns+ namespace on its own line",
        path.display()
    );
}

fn namespace_path(namespace: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}.hal",
        namespace.replace('.', "/").replace('-', "_")
    ))
}

fn source_roots(manifest: &Path) -> Vec<(PathBuf, PathBuf)> {
    for canonical in [manifest.join("../lib"), manifest.join("../../lib")] {
        let canonical_roots = [canonical.join("src"), canonical.join("src-lang")];
        if canonical_roots.iter().all(|root| root.is_dir()) {
            return vec![
                (canonical_roots[0].clone(), PathBuf::from("lib/src")),
                (canonical_roots[1].clone(), PathBuf::from("lib/src")),
            ];
        }
    }

    for packaged in [manifest.join("hal-src"), manifest.join("../hal-src")] {
        if packaged.is_dir() {
            return vec![(packaged, PathBuf::from("lib/src"))];
        }
    }

    panic!(
        "HAL sources are unavailable: build from the Hara repository or run \
         scripts/runtime/sync-rust-hal-src before packaging"
    );
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source_root = if manifest.join("../src/runtime_lib.rs").is_file() {
        manifest
            .join("..")
            .canonicalize()
            .unwrap_or_else(|error| panic!("cannot resolve runtime source root: {error}"))
    } else {
        manifest.clone()
    };
    println!("cargo:rustc-env=HARA_SOURCE_ROOT={}", source_root.display());
    // The runtime artifact contains the explicit Foundation bootstrap and the
    // small portable library catalog required by the native runtime. Repository
    // builds resolve it from canonical core/lib source; published Cargo archives
    // carry the same inventory beneath the crate-local hal-src.
    let source_roots = source_roots(&manifest);
    let inventory_path = [
        manifest.join("bootstrap.namespaces"),
        manifest.join("../bootstrap.namespaces"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(|| manifest.join("bootstrap.namespaces"));
    let cli_inventory_path = [
        manifest.join("cli-bootstrap.namespaces"),
        manifest.join("../cli-bootstrap.namespaces"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(|| manifest.join("cli-bootstrap.namespaces"));
    let hta_path = [manifest.join("src/hta.rs"), manifest.join("../src/hta.rs")]
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| manifest.join("src/hta.rs"));
    println!("cargo:rerun-if-changed={}", inventory_path.display());
    println!("cargo:rerun-if-changed={}", cli_inventory_path.display());
    println!("cargo:rerun-if-changed={}", hta_path.display());

    let inventory = fs::read_to_string(&inventory_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", inventory_path.display()))
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(
        !inventory.is_empty(),
        "{} is empty",
        inventory_path.display()
    );
    let cli_inventory = fs::read_to_string(&cli_inventory_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", cli_inventory_path.display()))
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(
        !cli_inventory.is_empty(),
        "{} is empty",
        cli_inventory_path.display()
    );
    let foundation_names = inventory.iter().cloned().collect::<HashSet<_>>();
    let cli_only_inventory = cli_inventory
        .iter()
        .filter(|namespace| !foundation_names.contains(*namespace))
        .cloned()
        .collect::<Vec<_>>();

    let resolve_resources = |namespaces: &[String]| {
        namespaces
        .iter()
        .map(|namespace| {
            let relative = namespace_path(namespace);
            let matches = source_roots
                .iter()
                .filter_map(|(source_root, embedded_root)| {
                    let path = source_root.join(&relative);
                    path.is_file()
                        .then(|| (path, embedded_root.join(&relative)))
                })
                .collect::<Vec<_>>();
            let [(path, embedded)] = matches.as_slice() else {
                panic!(
                    "embedded namespace {namespace} must resolve to exactly one HAL source, found {}",
                    matches.len()
                );
            };
            println!("cargo:rerun-if-changed={}", path.display());
            let source = fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let declared = declared_namespace(&source, path);
            assert_eq!(
                declared, *namespace,
                "{} does not declare embedded namespace {namespace}",
                path.display()
            );
            (namespace.clone(), path.clone(), embedded.clone())
        })
        .collect::<Vec<_>>()
    };
    let resources = resolve_resources(&inventory);
    let cli_resources = resolve_resources(&cli_only_inventory);

    let mut generated =
        "pub(crate) static EMBEDDED_HAL_RESOURCES: &[(&str, &str, &str)] = &[\n".to_owned();
    for (namespace, path, relative) in resources {
        let path = path
            .canonicalize()
            .unwrap_or_else(|error| panic!("cannot resolve {}: {error}", path.display()));
        let relative = relative.to_string_lossy().replace('\\', "/");
        generated.push_str(&format!(
            "    ({namespace:?}, {relative:?}, include_str!({path:?})),\n",
            namespace = namespace,
            relative = relative,
            path = path.to_string_lossy()
        ));
    }
    generated
        .push_str("];\n\npub(crate) static EMBEDDED_CLI_RESOURCES: &[(&str, &str, &str)] = &[\n");
    for (namespace, path, relative) in cli_resources {
        let path = path
            .canonicalize()
            .unwrap_or_else(|error| panic!("cannot resolve {}: {error}", path.display()));
        let relative = relative.to_string_lossy().replace('\\', "/");
        generated.push_str(&format!(
            "    ({namespace:?}, {relative:?}, include_str!({path:?})),\n",
            namespace = namespace,
            relative = relative,
            path = path.to_string_lossy()
        ));
    }
    generated.push_str("];\n");
    generated
        .push_str("#[cfg(test)]\npub(crate) static FOUNDATION_BOOTSTRAP_INVENTORY: &[&str] = &[\n");
    for namespace in &inventory {
        generated.push_str(&format!("    {namespace:?},\n"));
    }
    generated.push_str("];\n");
    generated.push_str("#[cfg(test)]\npub(crate) static CLI_BOOTSTRAP_INVENTORY: &[&str] = &[\n");
    for namespace in &cli_inventory {
        generated.push_str(&format!("    {namespace:?},\n"));
    }
    generated.push_str("];\n");

    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("embedded_hal.rs");
    fs::write(&output, generated)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", output.display()));
}
