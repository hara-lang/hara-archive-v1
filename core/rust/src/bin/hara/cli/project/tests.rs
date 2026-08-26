use super::{eval_runtime, Options};
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
        fs::create_dir_all(root.join("test/demo")).unwrap();
        fs::write(root.join("project.edn"), "{:hara/type :project\n :hara/version \"1.0.0\"\n :project/id demo/project-eval\n :project/version \"0.1.0\"\n :project/source-paths [\"src\"]\n :project/test-paths [\"test\"]\n :project/extension-paths []\n :project/capabilities #{}\n :project/dependencies {}}\n").unwrap();
        fs::write(
            root.join("src/demo/rules.hal"),
            "(ns demo.rules)\n\n(defn answer [] 42)\n",
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
