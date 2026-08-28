use super::{eval_runtime, source_catalog_for_options, Options};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("hara-project-eval-{}-{nonce}", std::process::id()));
        fs::create_dir_all(root.join("src/demo")).unwrap();
        fs::create_dir_all(root.join("src/legacy")).unwrap();
        fs::create_dir_all(root.join("test/demo")).unwrap();
        fs::write(root.join("project.edn"), "{:hara/type :project\n :hara/version \"1.0.0\"\n :project/id demo/project-eval\n :project/version \"0.1.0\"\n :project/source-paths [\"src\"]\n :project/test-paths [\"test\"]\n :project/extension-paths []\n :project/capabilities #{}\n :project/dependencies {}}\n").unwrap();
        fs::write(
            root.join("src/demo/rules.hal"),
            "(ns demo.rules)\n\n(defn answer [] 42)\n",
        )
        .unwrap();
        fs::write(
            root.join("src/demo/meta.hal"),
            "^{:source/catalog true}\n(ns demo.meta\n  (:require [demo.rules :as rules]))\n\n(defn answer [] (rules/answer))\n",
        )
        .unwrap();
        fs::write(root.join("src/demo/broken.hal"), "(ns demo.broken)\n(not parsed during indexing").unwrap();
        fs::write(
            root.join("src/legacy/declared.hal"),
            "(ns demo.declared)\n\n(defn answer [] 7)\n",
        )
        .unwrap();
        Self(root)
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn project_eval_registers_sources_without_a_root_mount() {
    let project = TempProject::new();
    let options = Options {
        project: Some(project.0.clone()),
        ..Options::default()
    };
    assert!(options.root.is_none());
    let mut runtime = eval_runtime(&options).unwrap();
    assert_eq!(
        runtime
            .eval_native(
                "(ns demo.invoke\n  (:require [demo.rules :as rules]))\n\n(rules/answer)\n"
            )
            .unwrap(),
        "42"
    );
}

#[test]
fn source_catalog_uses_declarations_and_loads_metadata_namespaces() {
    let project = TempProject::new();
    let options = Options {
        project: Some(project.0.clone()),
        ..Options::default()
    };
    let catalog = source_catalog_for_options(&options).unwrap().unwrap();
    assert_eq!(
        catalog.namespaces().collect::<Vec<_>>(),
        vec!["demo.broken", "demo.declared", "demo.meta", "demo.rules"]
    );

    let mut runtime = eval_runtime(&options).unwrap();
    assert_eq!(
        runtime
            .eval_native("(ns demo.invoke\n  (:require [demo.meta :as meta]))\n\n(meta/answer)\n")
            .unwrap(),
        "42"
    );
    assert_eq!(
        runtime
            .eval_native("(do (require [demo.declared :as declared]) (declared/answer))")
            .unwrap(),
        "7"
    );
    let mut interpreter = eval_runtime(&options).unwrap();
    interpreter.set_execution_backend("interpreter").unwrap();
    assert_eq!(
        interpreter
            .eval_native("(do (require [demo.meta :as meta]) (meta/answer))")
            .unwrap(),
        "42"
    );
}
