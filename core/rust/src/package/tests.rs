use super::*;
use crate::package_manifest::{
    PackageArtifactType, PackageManifest, PackageRuntime, PackageRuntimeRequirements,
    PackageSelection,
};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hara-package-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("src/example")).unwrap();
    fs::write(root.join("src/example/main.hal"), "(ns example.main) 42\n").unwrap();
    fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1.0.0\" :project/id example/app :project/version \"1.2.3\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [\"extensions\"] :project/capabilities #{} :project/dependencies {\"hara:hara/graph\" {:version \"^1.2.0\"}}}").unwrap();
    fs::write(
        root.join("project.lock.edn"),
        "{:lock/format \"0.0.0-alpha\" :packages {}}\n",
    )
    .unwrap();
    root
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn write_archive(path: &Path, entries: &[(String, Vec<u8>)]) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (name, bytes) in entries {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn runtime_archive(
    root: &Path,
    declared_artifact: &[u8],
    archived_artifact: &[u8],
    extra_file: bool,
) -> PathBuf {
    let project = br#"{:hara/type :project
 :hara/version "1.0.0"
 :project/id "hara:example/provider"
 :project/version "1.0.0"
 :project/source-paths []
 :project/test-paths []
 :project/extension-paths []
 :project/capabilities #{}
 :project/dependencies {}}
"#;
    let project_digest = digest(project);
    let artifact_digest = digest(declared_artifact);
    let package = format!(
        r#"{{:harp/format "0.0.0-alpha"
 :package {{:identity "hara:example/provider"
           :version "1.0.0"
           :provenance
           {{:repository "https://github.com/example/provider"
            :commit "0123456789abcdef0123456789abcdef01234567"}}}}
 :files
 {{"artifacts/provider.hta" {{:sha256 "{artifact_digest}" :size {artifact_size}}}
  "project.edn" {{:sha256 "{project_digest}" :size {project_size}}}}}
 :wasm-imports
 {{:provider
   {{:variant/artifact
     {{:artifact/type :hta
      :artifact/path "artifacts/provider.hta"
      :artifact/sha256 "{artifact_digest}"
      :artifact/target "wasm32-wasi-preview1"
      :artifact/abi "hta.v1"
      :artifact/entry-point "provider_init"}}
    :variant/required-capabilities #{{:db/connect}}
    :variant/host-calls #{{:db/socket}}
    :variant/exports #{{:provider/open :provider/close}}
    :variant/dependencies {{}}
    :variant/lifecycle
    {{:lifecycle/load :idempotent
     :lifecycle/close :idempotent
     :lifecycle/session-isolation true
     :lifecycle/async true
     :lifecycle/cancellation true}}}}}}}}"#,
        artifact_size = declared_artifact.len(),
        project_size = project.len()
    );
    let archive = root.join(format!(
        "runtime-{}.harp",
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut entries = vec![
        ("package.edn".to_owned(), package.into_bytes()),
        (
            "artifacts/provider.hta".to_owned(),
            archived_artifact.to_vec(),
        ),
        ("project.edn".to_owned(), project.to_vec()),
    ];
    if extra_file {
        entries.push(("hidden/payload.bin".to_owned(), b"hidden".to_vec()));
    }
    write_archive(&archive, &entries);
    archive
}

fn wasm_requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        supported_targets: ["wasm32-wasi-preview1".to_owned()].into_iter().collect(),
        supported_abis: ["hta.v1".to_owned()].into_iter().collect(),
        available_capabilities: ["db/connect".to_owned()].into_iter().collect(),
        allowed_host_calls: ["db/socket".to_owned()].into_iter().collect(),
    }
}

#[test]
fn validates_and_builds_deterministic_archive() {
    let root = fixture();
    let project = read_project(&root).unwrap();
    let first = root.join("one.harp");
    let second = root.join("two.harp");
    build_archive(&project, &first).unwrap();
    build_archive(&project, &second).unwrap();
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    let manifest = inspect_archive(&first).unwrap();
    assert!(manifest.contains(":harp/format \"0.0.0-alpha\""));
    assert!(manifest.contains(":identity \"hara:example/app\""));
    assert!(manifest.contains("\"example.main\" \"src/example/main.hal\""));
    let file = File::open(&first).unwrap();
    let mut zip = ZipArchive::new(file).unwrap();
    assert!(zip.by_name("project.edn").is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packages_only_explicit_source_files_from_custom_manifest() {
    let root = fixture();
    fs::write(
        root.join("src/example/provider.hal"),
        "(ns example.provider) 7\n",
    )
    .unwrap();
    fs::write(
        root.join("portable.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:example/portable\" :project/version \"1.0.0\" :project/source-paths [] :project/source-files [\"src/example/main.hal\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/dependencies {}}",
    )
    .unwrap();
    let project = read_project(&root.join("portable.edn")).unwrap();
    let archive = root.join("portable.harp");
    build_archive(&project, &archive).unwrap();
    let file = File::open(&archive).unwrap();
    let mut zip = ZipArchive::new(file).unwrap();
    assert!(zip.by_name("project.edn").is_ok());
    assert!(zip.by_name("src/example/main.hal").is_ok());
    assert!(zip.by_name("src/example/provider.hal").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn canonical_database_packages_exclude_runtime_providers() {
    let root = fixture();
    let database_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../lib/src/db");
    for (manifest, expected_sources) in [
        ("protocol.project.edn", vec!["protocol.hal"]),
        (
            "text.project.edn",
            vec![
                "text/base_check.hal",
                "text/base_flatten.hal",
                "text/base_graph.hal",
                "text/base_schema.hal",
                "text/base_scope.hal",
                "text/base_tree.hal",
                "text/base_util.hal",
                "text/sql_call.hal",
                "text/sql_graph.hal",
                "text/sql_raw.hal",
                "text/sql_tree.hal",
                "text/sql_util.hal",
                "text/sql_view.hal",
            ],
        ),
    ] {
        let project = read_project(&database_root.join(manifest)).unwrap();
        let archive = root.join(manifest.replace(".project.edn", ".harp"));
        build_archive(&project, &archive).unwrap();
        let file = File::open(&archive).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let mut names = (0..zip.len())
            .map(|index| zip.by_index(index).unwrap().name().to_owned())
            .collect::<Vec<_>>();
        names.sort();
        let mut expected = expected_sources
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        expected.extend(["project.edn".to_owned(), "project.lock.edn".to_owned()]);
        expected.push("package.edn".to_owned());
        expected.sort();
        assert_eq!(names, expected);
        assert!(!names.iter().any(|name| {
            name.contains("/postgres/") || name.contains("/sqlite/") || name.contains("/node/")
        }));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_project_keys_and_bad_ranges() {
    let root = fixture();
    fs::write(root.join("project.edn"), "{:hara/type :project}").unwrap();
    assert!(read_project(&root).unwrap_err().contains(":hara/version"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packages_declared_artifacts_under_the_archive_root() {
    let root = fixture();
    fs::create_dir_all(root.join("target/package/ledger/noir/assets")).unwrap();
    fs::write(
        root.join("target/package/ledger/noir/assets/worker.mjs"),
        "export {};\n",
    )
    .unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id hara/ledger-noir :project/version \"0.1.0\" :project/source-paths [] :project/test-paths [\"test\"] :project/extension-paths [\"target/package\"] :project/capabilities #{} :project/artifact-paths [\"target/package\"] :project/archive-root \"target/package\" :project/extensions {ledger.noir {:provider :hta :abi :hta.v1 :targets {:node {:provider \"ledger/noir/assets/worker.mjs\" :runtime :process}}}}}",
    )
    .unwrap();
    let project = read_project(&root).unwrap();
    let archive = root.join("ledger-noir.harp");
    build_archive(&project, &archive).unwrap();
    let file = File::open(&archive).unwrap();
    let mut zip = ZipArchive::new(file).unwrap();
    assert!(zip.by_name("ledger/noir/assets/worker.mjs").is_ok());
    let mut package = String::new();
    zip.by_name("package.edn")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(package.contains(":extensions {ledger.noir"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_declared_artifacts() {
    let root = fixture();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id example/app :project/version \"1.2.3\" :project/source-paths [] :project/test-paths [\"test\"] :project/extension-paths [\"extensions\"] :project/capabilities #{} :project/artifact-paths [\"target/package\"]}",
    )
    .unwrap();
    let project = read_project(&root).unwrap();
    assert!(build_archive(&project, &root.join("missing.harp"))
        .unwrap_err()
        .contains("does not exist"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packages_lock_and_explicit_portable_workspace_only() {
    let root = fixture();
    fs::write(
        root.join("project.lock.edn"),
        "{:lock/format \"0.0.0-alpha\" :packages {}}\n",
    )
    .unwrap();
    fs::write(
        root.join("workspace.edn"),
        "{:hara/type :workspace :hara/version \"1.0.0\"}\n",
    )
    .unwrap();
    let undeclared = root.join("undeclared-workspace.harp");
    build_archive(&read_project(&root).unwrap(), &undeclared).unwrap();
    let file = File::open(&undeclared).unwrap();
    let mut zip = ZipArchive::new(file).unwrap();
    assert!(zip.by_name("workspace.edn").is_err());
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id example/app :project/version \"1.2.3\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [\"extensions\"] :project/capabilities #{} :project/dependencies {\"hara:hara/graph\" {:version \"^1.2.0\"}} :project/package {:workspace true}}",
    )
    .unwrap();
    let archive = root.join("workspace.harp");
    build_archive(&read_project(&root).unwrap(), &archive).unwrap();
    let file = File::open(&archive).unwrap();
    let mut zip = ZipArchive::new(file).unwrap();
    assert!(zip.by_name("project.lock.edn").is_ok());
    assert!(zip.by_name("workspace.edn").is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn validates_typed_recipes_and_installs_content_addressed_roots() {
    let root = fixture();
    fs::write(root.join("hara.recipe.edn"), "{:recipe/format \"0.0.0-alpha\" :recipe/adapter :hal :recipe/toolchain {} :recipe/inputs {} :recipe/outputs []}\n").unwrap();
    let source = fs::read_to_string(root.join("project.edn")).unwrap();
    fs::write(
        root.join("project.edn"),
        source.trim().strip_suffix('}').unwrap().to_owned()
            + " :project/recipe \"hara.recipe.edn\"}\n",
    )
    .unwrap();
    let project = read_project(&root).unwrap();
    assert_eq!(
        validate_recipe(&project).unwrap(),
        root.join("hara.recipe.edn")
    );
    let archive = root.join("package.harp");
    build_archive(&project, &archive).unwrap();
    let dist = root.join("dist");
    let installed = install_archive_at(&archive, &dist).unwrap();
    assert!(installed.join("hara.recipe.edn").is_file());
    assert!(dist.join("packages/hara/example/app/1.2.3.edn").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn installs_semantic_packages_under_their_manifest_identity() {
    let root = fixture();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/packages.edn"),
        "{demo.public {:include [[demo.public :complete]]}}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/example/public.hal"),
        "(ns demo.public) (def answer 42)\n",
    )
    .unwrap();
    let source = fs::read_to_string(root.join("project.edn")).unwrap();
    fs::write(
        root.join("project.edn"),
        source.trim().strip_suffix('}').unwrap().to_owned()
            + " :project/package {:name \"demo.public\" :profile \"config/packages.edn\"}}\n",
    )
    .unwrap();

    let project = read_project(&root).unwrap();
    let archive = root.join("semantic.harp");
    build_archive(&project, &archive).unwrap();
    let manifest = PackageManifest::read_archive(&archive).unwrap();
    assert_eq!(manifest.identity, "hara:demo/public");
    assert_eq!(manifest.name.as_deref(), Some("demo.public"));

    let distribution = root.join("dist");
    install_archive_at(&archive, &distribution).unwrap();
    assert!(distribution
        .join("packages/hara/demo/public/1.2.3.edn")
        .is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selects_verified_runtime_archive_before_and_after_installation() {
    let root = fixture();
    let archive = runtime_archive(&root, b"wasm", b"wasm", false);
    let verified =
        PackageManifest::select_hta_require_archive(&archive, "provider", &wasm_requirements())
            .unwrap();
    let PackageSelection::Variant(variant) = &verified.selection else {
        panic!("expected Wasm import");
    };
    assert_eq!(variant.artifact.artifact_type, PackageArtifactType::Hta);
    assert_eq!(variant.artifact.path, Path::new("artifacts/provider.hta"));

    let dist = root.join("runtime-dist");
    let installed = install_archive_at(&archive, &dist).unwrap();
    let installed_manifest = PackageManifest::read(&installed.join("package.edn")).unwrap();
    let installed_selection = installed_manifest
        .select_hta_require("provider", &wasm_requirements())
        .unwrap();
    assert_eq!(installed_selection, verified.selection);
    assert!(dist
        .join("packages/hara/example/provider/1.0.0.edn")
        .is_file());

    let error = PackageManifest::select_archive(
        &archive,
        PackageRuntime::Jvm,
        &PackageRuntimeRequirements::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "package/missing-flavor");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_tampered_and_undeclared_archive_payloads_before_activation() {
    let root = fixture();
    let tampered = runtime_archive(&root, b"wasm", b"evil", false);
    let error = PackageManifest::read_archive(&tampered).unwrap_err();
    assert_eq!(error.code, "package/digest-mismatch");
    assert!(inspect_archive(&tampered)
        .unwrap_err()
        .contains("package/digest-mismatch"));

    let dist = root.join("tampered-dist");
    let install_error = install_archive_at(&tampered, &dist).unwrap_err();
    assert!(install_error.contains("package/digest-mismatch"));
    let archive_cache = dist.join("archives/sha256");
    assert!(!archive_cache.exists() || fs::read_dir(archive_cache).unwrap().next().is_none());
    let roots = dist.join("roots/sha256");
    assert!(!roots.exists() || fs::read_dir(roots).unwrap().next().is_none());

    let undeclared = runtime_archive(&root, b"wasm", b"wasm", true);
    let error = PackageManifest::read_archive(&undeclared).unwrap_err();
    assert_eq!(error.code, "package/invalid-manifest");
    assert!(error.detail.contains("undeclared file"));
    let undeclared_dist = root.join("undeclared-dist");
    assert!(install_archive_at(&undeclared, &undeclared_dist)
        .unwrap_err()
        .contains("undeclared file"));
    assert!(!undeclared_dist.join("packages").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_tampering_in_an_existing_content_addressed_root() {
    let root = fixture();
    let archive = runtime_archive(&root, b"wasm", b"wasm", false);
    let dist = root.join("existing-dist");
    let installed = install_archive_at(&archive, &dist).unwrap();
    fs::write(installed.join("artifacts/provider.hta"), b"evil").unwrap();
    let error = install_archive_at(&archive, &dist).unwrap_err();
    assert!(error.contains("package/digest-mismatch") || error.contains("package/size-mismatch"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_shell_recipe_escape_hatches() {
    let root = fixture();
    fs::write(root.join("hara.recipe.edn"), "{:recipe/format \"0.0.0-alpha\" :recipe/adapter :hal :recipe/toolchain {} :recipe/inputs {:command [\"sh\"]} :recipe/outputs []}\n").unwrap();
    let source = fs::read_to_string(root.join("project.edn")).unwrap();
    fs::write(
        root.join("project.edn"),
        source.trim().strip_suffix('}').unwrap().to_owned()
            + " :project/recipe \"hara.recipe.edn\"}\n",
    )
    .unwrap();
    assert!(validate_recipe(&read_project(&root).unwrap())
        .unwrap_err()
        .contains("cannot declare commands"));
    fs::remove_dir_all(root).unwrap();
}
