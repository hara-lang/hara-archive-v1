use hara_wasm::package;
use hara_wasm::package_manifest::PackageManifest;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "hara-package-profile-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos()
    ))
}

#[test]
fn package_build_filters_namespaces_and_emits_verified_bytecode() {
    let root = temp_root();
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("resources/assets/demo")).unwrap();
    fs::write(
        root.join("project.edn"),
        r#"{:hara/type :project
           :hara/version "1.0.0"
           :project/id "hara:demo/profiled"
           :project/version "1.0.0"
           :project/source-paths ["src"]
           :project/test-paths []
           :project/extension-paths []
           :project/capabilities #{}
           :project/package {:name "demo.public" :profile "config/packages.edn"}}"#,
    )
    .unwrap();
    fs::write(
        root.join("config/packages.edn"),
        r#"{demo.public {:include [[demo.public :complete]]
                         :bundle [{:path "resources" :include ["assets/demo"]}]}
           demo.private {:include [[demo.private :complete]]}}"#,
    )
    .unwrap();
    fs::write(
        root.join("resources/assets/demo/info.txt"),
        "portable asset\n",
    )
    .unwrap();
    fs::write(
        root.join("src/demo/public.hal"),
        "(ns demo.public) (def answer 42)",
    )
    .unwrap();
    fs::write(
        root.join("src/demo/private.hal"),
        "(ns demo.private) (def secret 7)",
    )
    .unwrap();

    let archive = root.join("target/demo.harp");
    let result = package::build_path(&root, Some(&archive));
    assert!(result.is_ok(), "package build failed: {result:?}");
    let manifest = PackageManifest::read_archive(&archive).unwrap();
    assert_eq!(manifest.name.as_deref(), Some("demo.public"));
    assert_eq!(manifest.identity, "hara:demo/public");
    assert!(manifest.bytecode.is_some());
    assert!(manifest
        .files
        .contains_key(&PathBuf::from("src/demo/public.hal")));
    assert!(manifest
        .files
        .contains_key(&PathBuf::from("config/packages.edn")));
    assert!(manifest
        .files
        .contains_key(&PathBuf::from("assets/demo/info.txt")));
    assert!(!manifest
        .files
        .contains_key(&PathBuf::from("src/demo/private.hal")));
    assert!(manifest
        .canonical_edn()
        .contains("\"demo.public\" \"src/demo/public.hal\""));
    assert!(!manifest.canonical_edn().contains("demo.private"));
    let distribution = root.join("dist");
    package::install_path_at(&archive, &distribution).unwrap();
    assert!(distribution
        .join("packages/hara/demo/public/1.0.0.edn")
        .is_file());

    let _ = fs::remove_dir_all(root);
}
