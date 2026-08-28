#[cfg(test)]
mod tests {
    use super::*;

    fn conformance_entry<'a>(entries: &'a [(Form, Form)], key: &str) -> &'a Form {
        entries
            .iter()
            .find_map(|(candidate, value)| {
                matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
            })
            .unwrap_or_else(|| panic!("missing conformance key :{key}"))
    }

    fn protocol_method_surface(source: &str) -> std::collections::BTreeSet<(String, String)> {
        let Form::Map(root) = kernel::parse_forms(source).unwrap().remove(0) else {
            panic!("protocol contract must be a map")
        };
        let mut surface = std::collections::BTreeSet::new();
        for key in ["protocols", "capability-protocols"] {
            let Form::Vector(protocols) = conformance_entry(&root, key) else {
                panic!(":{key} must be a vector")
            };
            for protocol in protocols {
                let Form::Map(protocol) = protocol else {
                    panic!("protocol entries must be maps")
                };
                let Form::Symbol(name) = conformance_entry(protocol, "name") else {
                    panic!("protocol :name must be a symbol")
                };
                let Form::Map(methods) = conformance_entry(protocol, "methods") else {
                    panic!("protocol :methods must be a map")
                };
                for (method, _) in methods {
                    let Form::Symbol(method) = method else {
                        panic!("protocol method names must be symbols")
                    };
                    surface.insert((name.clone(), method.clone()));
                }
            }
        }
        surface
    }

    fn protocol_case_surface(source: &str) -> std::collections::BTreeSet<(String, String)> {
        let Form::Map(root) = kernel::parse_forms(source).unwrap().remove(0) else {
            panic!("protocol case catalog must be a map")
        };
        let Form::Vector(cases) = conformance_entry(&root, "cases") else {
            panic!(":cases must be a vector")
        };
        cases
            .iter()
            .map(|case| {
                let Form::Map(case) = case else {
                    panic!("protocol cases must be maps")
                };
                let Form::Symbol(protocol) = conformance_entry(case, "protocol") else {
                    panic!("case :protocol must be a symbol")
                };
                let Form::Symbol(method) = conformance_entry(case, "method") else {
                    panic!("case :method must be a symbol")
                };
                (protocol.clone(), method.clone())
            })
            .collect()
    }

    fn sandbox_eval(
        kernel: &mut SessionKernel,
        sandbox: SandboxId,
        source: &str,
    ) -> Result<String, SandboxError> {
        kernel.sandbox_eval(sandbox, source)?.wait()
    }

    fn sandbox_call(
        kernel: &mut SessionKernel,
        sandbox: SandboxId,
        callable: &str,
        arguments: &[u8],
    ) -> Result<Vec<u8>, SandboxError> {
        kernel.sandbox_call(sandbox, callable, arguments)?.wait()
    }

    #[test]
    fn direct_import_discovers_only_verified_core_wasm_packages() {
        use sha2::{Digest, Sha256};

        const ADD: &[u8] =
            b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";
        let root = std::env::temp_dir().join(format!(
            "hara-runtime-package-import-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        let project = r#"{:hara/type :project :hara/version "1.0.0"
 :project/id "example/provider" :project/version "1.0.0"
 :project/source-paths [] :project/test-paths [] :project/extension-paths []
 :project/capabilities #{}
 :project/extensions {demo.provider {:identity "example/provider" :provider :wasm
                                      :module "provider.wasm" :abi :core.v1
                                      :exports {"add" {:args [:i64 :i64] :returns :i64}}
                                      :capabilities []}}}"#;
        let digest = |bytes: &[u8]| {
            format!(
                "sha256:{}",
                Sha256::digest(bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        };
        let wasm_digest = digest(ADD);
        let project_digest = digest(project.as_bytes());
        let package = r#"{:harp/format "0.0.0-alpha"
 :package {:identity "example/provider" :version "1.0.0"
           :provenance {:repository "https://github.com/example/provider"
                        :commit "0123456789abcdef0123456789abcdef01234567"}}
 :files {"project.edn" {:sha256 "__PROJECT_DIGEST__" :size __PROJECT_SIZE__}
          "provider.wasm" {:sha256 "__WASM_DIGEST__" :size __WASM_SIZE__}}
 :wasm-imports {:demo.provider {:variant/artifact {:artifact/type :wasm
                                      :artifact/path "provider.wasm"
                                      :artifact/sha256 "__WASM_DIGEST__"
                                      :artifact/target "wasm32-wasi-preview1"
                                      :artifact/abi "core.v1"
                                      :artifact/entry-point "add"}
                    :variant/required-capabilities #{}
                    :variant/host-calls #{}
                    :variant/exports #{:add}}}}"#
            .replace("__PROJECT_DIGEST__", &project_digest)
            .replace("__PROJECT_SIZE__", &project.len().to_string())
            .replace("__WASM_DIGEST__", &wasm_digest)
            .replace("__WASM_SIZE__", &ADD.len().to_string());
        std::fs::write(root.join("project.edn"), project).unwrap();
        std::fs::write(root.join("provider.wasm"), ADD).unwrap();
        std::fs::write(root.join("package.edn"), package).unwrap();

        let mut runtime = Runtime::new();
        runtime.add_extension_root(root.clone());
        assert_eq!(
            runtime
                .eval_text("(ns imported (:import demo.provider)) (demo.provider/add 20 22)")
                .unwrap(),
            "42"
        );

        std::fs::write(root.join("provider.wasm"), vec![0u8; ADD.len()]).unwrap();
        let mut rejected = Runtime::new();
        rejected.add_extension_root(root.clone());
        let error = rejected
            .eval_text("(ns rejected (:import demo.provider))")
            .unwrap_err();
        assert!(error.starts_with("package/digest-mismatch:"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn in_process_sandbox_lifecycle_is_private_and_explicitly_non_secure() {
        let mut kernel = SessionKernel::new();
        let provider = Rc::new(InProcessSandboxProvider);
        assert!(!provider.secure());
        kernel.register_sandbox_provider(provider);
        let sessions_before = kernel.session_names();

        let sandbox = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
        assert_eq!(kernel.session_names(), sessions_before);
        assert_eq!(
            sandbox_eval(&mut kernel, sandbox, "(def answer 41) answer").unwrap(),
            "41"
        );
        let arguments = crate::hta::encode(&core::Value::Vector(
            vec![core::Value::Number(41), core::Value::Number(1)].into(),
        ))
        .unwrap();
        let result = sandbox_call(&mut kernel, sandbox, "std.foundation/+", &arguments).unwrap();
        assert_eq!(
            crate::hta::decode(&result).unwrap(),
            core::Value::Number(42)
        );
        let inert_source = "(do (def injected 99) :executed)";
        let arguments = crate::hta::encode(&core::Value::Vector(
            vec![core::Value::String(inert_source.into())].into(),
        ))
        .unwrap();
        let result =
            sandbox_call(&mut kernel, sandbox, "std.foundation/identity", &arguments).unwrap();
        assert_eq!(
            crate::hta::decode(&result).unwrap(),
            core::Value::String(inert_source.into())
        );
        assert_eq!(
            kernel.sandbox_status(sandbox).unwrap().state,
            SandboxState::Open
        );
        assert!(!kernel.cancel_sandbox(sandbox).unwrap());
        assert_eq!(
            kernel.sandbox_status(sandbox).unwrap().state,
            SandboxState::Open
        );
        kernel.close_sandbox(sandbox).unwrap();
        assert_eq!(
            kernel.sandbox_status(sandbox).unwrap_err().code,
            SandboxErrorCode::NotFound
        );
        let injection_probe = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
        assert!(sandbox_eval(&mut kernel, injection_probe, "injected").is_err());
        kernel.close_sandbox(injection_probe).unwrap();
    }

    #[test]
    fn sandbox_spec_validation_and_runtime_isolation_are_enforced() {
        let invalid = SandboxSpec::new(
            SANDBOX_SPEC_PROTOCOL,
            "in-process",
            "hara.standard/0-alpha",
            "user",
            SandboxLimits {
                active_evaluations: 2,
                ..SandboxLimits::default()
            },
        );
        assert_eq!(invalid.unwrap_err().code, SandboxErrorCode::InvalidSpec);

        let mut kernel = SessionKernel::new();
        kernel.register_sandbox_provider(Rc::new(InProcessSandboxProvider));
        let root = SessionId::parse("ROOT").unwrap();
        kernel
            .eval(&root, "(do (def parent-secret 42) nil)")
            .unwrap();
        let parent_probe = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
        let error = sandbox_eval(&mut kernel, parent_probe, "parent-secret").unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::EvaluationFailed);
        kernel.close_sandbox(parent_probe).unwrap();
        for symbol in [
            "Runtime",
            "Kernel",
            "Sandbox",
            "Crypto",
            "File",
            "Socket",
            "Process",
            "OS",
            "Package",
            "Host",
            "std.native.Runtime/current",
            "std.native.Kernel",
        ] {
            let sandbox = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
            let error = sandbox_eval(&mut kernel, sandbox, symbol).unwrap_err();
            assert_eq!(error.code, SandboxErrorCode::EvaluationFailed, "{symbol}");
            kernel.close_sandbox(sandbox).unwrap();
        }
        let sandbox = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
        assert_eq!(
            sandbox_eval(&mut kernel, sandbox, "(ns-find 'std.native.Kernel)").unwrap(),
            "nil"
        );
        assert_eq!(
            sandbox_eval(&mut kernel, sandbox, "(ns-loaded? 'std.native.Runtime)").unwrap(),
            "false"
        );
        assert_eq!(
            sandbox_eval(&mut kernel, sandbox, "(ns-state 'std.native.Package)").unwrap(),
            ":unknown"
        );
        assert_eq!(
            sandbox_eval(
                &mut kernel,
                sandbox,
                "(do (defn sandbox-sum [xs] (reduce + 0 xs)) (sandbox-sum (map inc [0 1 2])))",
            )
            .unwrap(),
            "6"
        );
        assert!(sandbox_eval(&mut kernel, sandbox, "(ns-publics 'std.native.File)").is_err());
        kernel.close_sandbox(sandbox).unwrap();
    }

    #[test]
    fn sandbox_bundles_and_mounts_are_resolved_and_released_by_the_kernel() {
        let mut kernel = SessionKernel::new();
        kernel.register_sandbox_provider(Rc::new(InProcessSandboxProvider));
        let digest = "sha256:039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81";
        kernel.register_bundle(digest, &[1, 2, 3]).unwrap();
        let mount = kernel.create_memory_filesystem("sandbox-test");
        let spec = SandboxSpec::with_inputs(
            SANDBOX_SPEC_PROTOCOL,
            "in-process",
            "hara.standard/0-alpha",
            "user",
            vec![SandboxBundleReference::new(digest, "halc").unwrap()],
            Some(mount),
            Vec::new(),
            SandboxLimits::default(),
        )
        .unwrap();
        let sandbox = kernel.open_sandbox(spec).unwrap();
        assert_eq!(kernel.filesystem_info(mount).unwrap().2, 1);
        assert!(kernel.close_filesystem(mount).is_err());
        kernel.close_sandbox(sandbox).unwrap();
        assert_eq!(kernel.filesystem_info(mount).unwrap().2, 0);
        kernel.close_filesystem(mount).unwrap();

        let missing = SandboxSpec::with_inputs(
            SANDBOX_SPEC_PROTOCOL,
            "in-process",
            "hara.standard/0-alpha",
            "user",
            vec![SandboxBundleReference::new(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "halc",
            )
            .unwrap()],
            None,
            Vec::new(),
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(
            kernel.open_sandbox(missing).unwrap_err().code,
            SandboxErrorCode::BundleNotFound
        );

        let mismatched = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        kernel.register_bundle(mismatched, &[9]).unwrap();
        let mismatch = SandboxSpec::with_inputs(
            SANDBOX_SPEC_PROTOCOL,
            "in-process",
            "hara.standard/0-alpha",
            "user",
            vec![SandboxBundleReference::new(mismatched, "halc").unwrap()],
            None,
            Vec::new(),
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(
            kernel.open_sandbox(mismatch).unwrap_err().code,
            SandboxErrorCode::BundleDigestMismatch
        );
    }

    #[test]
    fn sandbox_evaluations_are_busy_cancellable_timed_and_terminal() {
        let mut kernel = SessionKernel::new();
        kernel.register_sandbox_provider(Rc::new(InProcessSandboxProvider));

        let sandbox = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
        let pending = kernel.sandbox_eval(sandbox, "(loop [] (recur))").unwrap();
        assert_eq!(
            kernel.sandbox_eval(sandbox, "42").unwrap_err().code,
            SandboxErrorCode::Busy
        );
        assert!(kernel.cancel_sandbox(sandbox).unwrap());
        assert!(matches!(
            kernel.sandbox_status(sandbox).unwrap().state,
            SandboxState::Cancelling | SandboxState::Cancelled
        ));
        assert_eq!(
            pending.wait().unwrap_err().code,
            SandboxErrorCode::Cancelled
        );
        assert_eq!(
            kernel.sandbox_status(sandbox).unwrap().state,
            SandboxState::Cancelled
        );
        assert!(!kernel.cancel_sandbox(sandbox).unwrap());
        assert_eq!(
            kernel.sandbox_eval(sandbox, "42").unwrap_err().code,
            SandboxErrorCode::Closed
        );
        kernel.close_sandbox(sandbox).unwrap();

        let timeout_spec = SandboxSpec::new(
            SANDBOX_SPEC_PROTOCOL,
            "in-process",
            "hara.standard/0-alpha",
            "user",
            SandboxLimits {
                evaluation_ms: 5,
                ..SandboxLimits::default()
            },
        )
        .unwrap();
        let sandbox = kernel.open_sandbox(timeout_spec).unwrap();
        let error = kernel
            .sandbox_eval(sandbox, "(loop [] (recur))")
            .unwrap()
            .wait()
            .unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::Timeout);
        assert_eq!(
            kernel.sandbox_status(sandbox).unwrap().state,
            SandboxState::Failed
        );
        kernel.close_sandbox(sandbox).unwrap();

        let sandbox = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
        let pending = kernel.sandbox_eval(sandbox, "(loop [] (recur))").unwrap();
        kernel.close_sandbox(sandbox).unwrap();
        assert_eq!(
            pending.wait().unwrap_err().code,
            SandboxErrorCode::Cancelled
        );
        assert_eq!(
            kernel.sandbox_status(sandbox).unwrap_err().code,
            SandboxErrorCode::NotFound
        );

        let small_result = SandboxSpec::new(
            SANDBOX_SPEC_PROTOCOL,
            "in-process",
            "hara.standard/0-alpha",
            "user",
            SandboxLimits {
                result_bytes: 2,
                ..SandboxLimits::default()
            },
        )
        .unwrap();
        let sandbox = kernel.open_sandbox(small_result).unwrap();
        assert_eq!(
            kernel
                .sandbox_eval(sandbox, "\"abcd\"")
                .unwrap()
                .wait()
                .unwrap_err()
                .code,
            SandboxErrorCode::LimitExceeded
        );
        kernel.close_sandbox(sandbox).unwrap();

        let sandbox = kernel.open_sandbox(SandboxSpec::in_process()).unwrap();
        assert_eq!(
            kernel
                .sandbox_eval(sandbox, "(fn [] 1)")
                .unwrap()
                .wait()
                .unwrap_err()
                .code,
            SandboxErrorCode::ResultNotTransferable
        );
        kernel.close_sandbox(sandbox).unwrap();
    }

    #[test]
    fn execution_state_owns_lexical_state_without_owning_namespace_state() {
        let registry = kernel::NamespaceRegistry::<core::Value>::new("user");
        let mut execution = RuntimeExecutionState::new();
        execution
            .environment_mut()
            .insert("local".into(), core::Value::Number(42));

        assert_eq!(
            execution.environment().get("local"),
            Some(&core::Value::Number(42))
        );
        assert!(registry.current().mappings().is_empty());
    }

    fn session_id(name: &str) -> SessionId {
        SessionId::parse(name).unwrap()
    }

    #[test]
    fn defonce_preserves_the_existing_var_root() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(macroexpand-1 '(defonce retained-state (atom 1)))")
                .unwrap(),
            "(if (Base/resolve (quote user/retained-state)) (Base/resolve (quote user/retained-state)) (Runtime/eval (quote (def retained-state (atom 1)))))"
        );
        assert_eq!(
            runtime
                .eval_text("(do (eval '(def eval-defined 1)) [(Base/resolve 'user/eval-defined) eval-defined])")
                .unwrap(),
            "[#'user/eval-defined 1]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(do (defonce retained-state (atom 1)) \
                         (swap! retained-state inc) \
                         (defonce retained-state (atom 99)) \
                         (deref retained-state))",
                )
                .unwrap(),
            "2"
        );
    }

    #[test]
    fn syntax_runtime_and_result_native_contracts_are_available() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_native(
                    "[(comment missing-symbol (throw (ex-info \"boom\" {})) (def leaked 1)) \
                      (special-symbol? 'comment) \
                      (type (std.native.Result/create :success 1)) \
                      (std.native.Result/status (std.native.Result/create :error \"boom\")) \
                      (std.native.Result/context (std.native.Result/create :success 1)) \
                      (:status (std.native.Result/create :success 1)) \
                      (:data (std.native.Result/create :success 42)) \
                      (:error (std.native.Result/create :success 1)) \
                      (:context (std.native.Result/create :success 1 {:source :test})) \
                      (Runtime/current) \
                      (Runtime/eval '(+ 19 23)) \
                      (Runtime/load-string \"(+ 19 23)\") \
                      (map? (std.foundation/env-snapshot)) \
                      (get (Runtime/namespace 'std.native.Runtime) :namespace/state) \
                      (Runtime/namespace 'std.native.Env) \
                      (std.foundation/resolve 'std.native.Env/current)]",
                )
                .unwrap(),
            "[nil true :std.native.Result :error nil :success 42 nil {:source :test} user 42 42 true :loaded nil nil]"
        );
        assert!(runtime.eval_native("leaked").is_err());
        assert_eq!(
            runtime
                .eval_native(
                    "[(get (Runtime/namespace 'user) :namespace/state) \
                      (Runtime/eval-in 'user '[(+ 19 23)])]",
                )
                .unwrap(),
            "[:loaded 42]"
        );
    }

    #[test]
    fn catch_selectors_match_structured_error_codes() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(try (throw (ex :file/not-found {})) \
                       (catch :socket/closed error :wrong) \
                       (catch :file/not-found error (:ex/code (ex-data error))))",
                )
                .unwrap(),
            ":file/not-found"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(try (throw (ex :file/not-found {:ex/message \"missing\"})) \
                       (catch [:file/not-found :file/permission-denied] error :file-error))",
                )
                .unwrap(),
            ":file-error"
        );
        assert_eq!(
            runtime
                .eval_text("(ex-message (ex :failure/code {}))")
                .unwrap(),
            "\":failure/code\""
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(try\n  (throw (ex :test/provenance {}))\n  (catch caught\n    (let [provenance (ex-provenance caught)]\n      [(:line (:ex/created-at provenance))\n       (:column (:ex/created-at provenance))\n       (:line (first (:ex/throws provenance)))\n       (:column (first (:ex/throws provenance)))\n       (count (:ex/throws provenance))])))",
                )
                .unwrap(),
            "[2 10 2 3 1]"
        );
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn embedding_registry_exposes_the_foundation_json_shortcut() {
        let namespaces = embedding_namespace_registry();
        assert!(vm::compile_source_with("(Json/write {\"a\" 1})", &namespaces).is_ok());
    }

    fn repo_text(relative: &str) -> Option<String> {
        let path = crate::spec_registry::resolve(relative)?;
        match std::fs::read_to_string(&path) {
            Ok(content) => Some(content),
            Err(_) => {
                eprintln!(
                    "skipping: {} is unavailable (hara-specs-registry sibling repo not present)",
                    path.display()
                );
                None
            }
        }
    }

    #[test]
    fn shared_stack_safety_corpus_runs_on_the_native_runtime() {
        let Some(corpus) = repo_text("01-lang/001-language/draft/conformance/stack-safety.edn")
        else {
            return;
        };
        let document = kernel::parse_forms(&corpus)
            .expect("stack-safety corpus must parse")
            .remove(0);
        let Form::Map(document) = document else {
            panic!("stack-safety corpus must be a map");
        };
        let Form::Vector(cases) = conformance_entry(&document, "cases") else {
            panic!("stack-safety corpus must contain :cases");
        };
        assert!(!cases.is_empty());

        let mut passed = 0;
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for raw_case in cases {
            let Form::Map(test_case) = raw_case else {
                panic!("stack-safety cases must be maps");
            };
            let id = conformance_entry(test_case, "id").to_string();
            let Form::String(source) = conformance_entry(test_case, "source") else {
                panic!("stack-safety case {id} must contain a string :source");
            };
            let Form::Map(expect) = conformance_entry(test_case, "expect") else {
                panic!("stack-safety case {id} must contain an :expect map");
            };
            let expected_error = expect.iter().find_map(|(key, value)| {
                matches!(key, Form::Keyword(name) if name == "error").then_some(value)
            });
            let expected_message = expect.iter().find_map(|(key, value)| {
                matches!(key, Form::Keyword(name) if name == "message").then_some(value)
            });
            let mut runtime = Runtime::new();
            match runtime.eval_text(source) {
                Ok(actual) if expected_error.is_none() => {
                    let expected = conformance_entry(expect, "value").to_string();
                    if actual == expected {
                        passed += 1;
                        results.push(format!("{{:id {id} :status :passed}}"));
                    } else {
                        failures.push(format!("{id}: expected {expected}, got {actual}"));
                        results.push(format!("{{:id {id} :status :failed}}"));
                    }
                }
                Ok(_) => {
                    failures.push(format!("{id}: expected an error"));
                    results.push(format!("{{:id {id} :status :failed}}"));
                }
                Err(error) if expected_error.is_some() => {
                    let message = error.to_string();
                    if expected_message.is_some_and(|expected| {
                        matches!(expected, Form::String(value) if message.contains(value))
                    }) {
                        passed += 1;
                        results.push(format!("{{:id {id} :status :passed}}"));
                    } else {
                        failures.push(format!("{id}: unexpected error {message}"));
                        results.push(format!("{{:id {id} :status :failed}}"));
                    }
                }
                Err(error) => {
                    failures.push(format!("{id}: {error}"));
                    results.push(format!("{{:id {id} :status :failed}}"));
                }
            }
        }

        let root = std::env::var_os("HARA_CONFORMANCE_REPORT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("HARA_SOURCE_ROOT"))
                    .join("..")
                    .join("target")
                    .join("conformance")
            });
        let directory = root.join("rust");
        std::fs::create_dir_all(&directory).expect("create Rust conformance report directory");
        let status = if failures.is_empty() {
            ":passed"
        } else {
            ":failed"
        };
        let report = format!(
            "{{:report/schema :hara.conformance.runtime/0-alpha :report/suite :hal/stack-safety :report/runtime :rust :report/status {status} :report/passed {} :report/total {} :report/cases [{}]}}\n",
            passed,
            cases.len(),
            results.join(" ")
        );
        std::fs::write(directory.join("stack-safety.edn"), report)
            .expect("write Rust conformance report");
        assert!(failures.is_empty(), "stack-safety failures: {failures:?}");
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn shared_stack_safety_corpus_runs_on_the_bytecode_runtime() {
        let Some(corpus) = repo_text("01-lang/001-language/draft/conformance/stack-safety.edn")
        else {
            return;
        };
        let document = kernel::parse_forms(&corpus)
            .expect("stack-safety corpus must parse")
            .remove(0);
        let Form::Map(document) = document else {
            panic!("stack-safety corpus must be a map");
        };
        let Form::Vector(cases) = conformance_entry(&document, "cases") else {
            panic!("stack-safety corpus must contain :cases");
        };
        assert!(!cases.is_empty());

        let mut passed = 0;
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for raw_case in cases {
            let Form::Map(test_case) = raw_case else {
                panic!("stack-safety cases must be maps");
            };
            let id = conformance_entry(test_case, "id").to_string();
            let Form::String(source) = conformance_entry(test_case, "source") else {
                panic!("stack-safety case {id} must contain a string :source");
            };
            let Form::Map(expect) = conformance_entry(test_case, "expect") else {
                panic!("stack-safety case {id} must contain an :expect map");
            };
            let expected_error = expect.iter().find_map(|(key, value)| {
                matches!(key, Form::Keyword(name) if name == "error").then_some(value)
            });
            let expected_message = expect.iter().find_map(|(key, value)| {
                matches!(key, Form::Keyword(name) if name == "message").then_some(value)
            });
            let mut runtime = Runtime::new();
            match runtime.eval_bytecode_native(source) {
                Ok(actual) if expected_error.is_none() => {
                    let expected = conformance_entry(expect, "value").to_string();
                    if actual == expected {
                        passed += 1;
                        results.push(format!("{{:id {id} :status :passed}}"));
                    } else {
                        failures.push(format!("{id}: expected {expected}, got {actual}"));
                        results.push(format!("{{:id {id} :status :failed}}"));
                    }
                }
                Ok(_) => {
                    failures.push(format!("{id}: expected an error"));
                    results.push(format!("{{:id {id} :status :failed}}"));
                }
                Err(error) if expected_error.is_some() => {
                    let message = error.to_string();
                    if expected_message.is_some_and(|expected| {
                        matches!(expected, Form::String(value) if message.contains(value))
                    }) {
                        passed += 1;
                        results.push(format!("{{:id {id} :status :passed}}"));
                    } else {
                        failures.push(format!("{id}: unexpected error {message}"));
                        results.push(format!("{{:id {id} :status :failed}}"));
                    }
                }
                Err(error) => {
                    failures.push(format!("{id}: {error}"));
                    results.push(format!("{{:id {id} :status :failed}}"));
                }
            }
        }

        let root = std::env::var_os("HARA_CONFORMANCE_REPORT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("HARA_SOURCE_ROOT"))
                    .join("..")
                    .join("target")
                    .join("conformance")
            });
        let directory = root.join("bytecode");
        std::fs::create_dir_all(&directory).expect("create bytecode conformance report directory");
        let status = if failures.is_empty() {
            ":passed"
        } else {
            ":failed"
        };
        let report = format!(
            "{{:report/schema :hara.conformance.runtime/0-alpha :report/suite :hal/stack-safety :report/runtime :bytecode :report/status {status} :report/passed {} :report/total {} :report/cases [{}]}}\n",
            passed,
            cases.len(),
            results.join(" ")
        );
        std::fs::write(directory.join("stack-safety.edn"), report)
            .expect("write bytecode conformance report");
        assert!(failures.is_empty(), "stack-safety failures: {failures:?}");
    }

    fn foundation_behavior_sources() -> Option<(String, String)> {
        let corpus = repo_text(
            "01-lang/004-foundation/draft/conformance/fixtures/foundation_behavioral.hal",
        )?;
        let root = std::path::Path::new(env!("HARA_SOURCE_ROOT"))
            .join("..")
            .join("lib")
            .join("src")
            .join("std")
            .join("foundation");
        let mut source = String::new();
        for module in [
            "bytes.hal",
            "coroutine.hal",
            "pretty.hal",
            "promise.hal",
            "string.hal",
        ] {
            let path = root.join(module);
            source.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
            );
            source.push('\n');
        }
        Some((source, corpus))
    }

    fn foundation_surface() -> Option<(
        std::collections::BTreeSet<String>,
        std::collections::BTreeSet<String>,
    )> {
        let source = repo_text("01-lang/004-foundation/draft/conformance/foundation-surface.edn")?;
        let forms = kernel::parse_forms(&source).expect("parse specs-owned Foundation surface");
        let [Form::Map(root)] = forms.as_slice() else {
            panic!("Foundation surface must contain one map");
        };
        let Form::Vector(namespaces) = conformance_entry(root, "namespaces") else {
            panic!("Foundation surface must contain :namespaces");
        };
        let mut namespace_names = std::collections::BTreeSet::new();
        let mut symbols = std::collections::BTreeSet::new();
        for namespace in namespaces {
            let Form::Map(namespace) = namespace else {
                panic!("Foundation namespace entries must be maps");
            };
            let Form::Symbol(namespace_name) = conformance_entry(namespace, "namespace") else {
                panic!("Foundation namespace entry must name a symbol");
            };
            assert!(namespace_names.insert(namespace_name.clone()));
            let Form::Vector(vars) = conformance_entry(namespace, "vars") else {
                panic!("Foundation namespace entry must contain :vars");
            };
            for var in vars {
                let Form::Map(var) = var else {
                    panic!("Foundation Var entries must be maps");
                };
                let Form::Symbol(name) = conformance_entry(var, "name") else {
                    panic!("Foundation Var entry must name a symbol");
                };
                assert!(symbols.insert(format!("{namespace_name}/{name}")));
            }
        }
        Some((namespace_names, symbols))
    }

    fn register_lib_tree(runtime: &mut Runtime, root: &std::path::Path, dir: &std::path::Path) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let file_name = path.file_name().and_then(|name| name.to_str());
            if file_name.is_some_and(|name| name.starts_with('.')) {
                continue;
            }
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                register_lib_tree(runtime, root, &path);
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "hal") {
                let relative = path.strip_prefix(root).unwrap().with_extension("");
                let namespace = relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(".")
                    .replace('_', "-");
                let source = std::fs::read_to_string(&path).unwrap();
                runtime.register_resource(&namespace, &source);
            }
        }
    }

    fn development_runtime() -> Runtime {
        let mut runtime = Runtime::new();
        let lib = std::path::Path::new(env!("HARA_SOURCE_ROOT"))
            .join("..")
            .join("lib");
        for source_root in [lib.join("src"), lib.join("src-lang")] {
            register_lib_tree(&mut runtime, &source_root, &source_root);
        }
        runtime
    }

    fn module_case(id: &str) -> Vec<(Form, Form)> {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
        }

        let corpus = repo_text("01-lang/001-language/draft/conformance/modules.edn")
            .expect("specs submodule must be initialized for module conformance tests");
        let manifest = kernel::parse_forms(&corpus)
            .expect("module conformance corpus must parse")
            .remove(0);
        let Form::Map(manifest) = manifest else {
            panic!("module conformance corpus must be a map")
        };
        let Some(Form::Vector(cases)) = entry(&manifest, "cases") else {
            panic!("module conformance :cases must be a vector")
        };
        cases
            .iter()
            .find_map(|case| {
                let Form::Map(case) = case else {
                    return None;
                };
                matches!(entry(case, "id"), Some(Form::Keyword(candidate)) if candidate == id)
                    .then(|| case.clone())
            })
            .unwrap_or_else(|| panic!("missing module conformance case :{id}"))
    }

    fn module_expect(id: &str, key: &str) -> Form {
        let case = module_case(id);
        let expect = case.iter().find_map(|(candidate, value)| match candidate {
            Form::Keyword(name) if name == "expect" => Some(value),
            _ => None,
        });
        let Some(Form::Map(expect)) = expect else {
            panic!("module conformance case :{id} must have an :expect map")
        };
        expect
            .iter()
            .find_map(|(candidate, value)| {
                matches!(candidate, Form::Keyword(name) if name == key).then(|| value.clone())
            })
            .unwrap_or_else(|| panic!("module conformance case :{id} is missing :expect :{key}"))
    }

    fn module_runtime_profile(runtime: &str, key: &str) -> Form {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
        }

        let corpus = repo_text("01-lang/001-language/draft/conformance/modules.edn")
            .expect("specs submodule must be initialized for module conformance tests");
        let manifest = kernel::parse_forms(&corpus)
            .expect("module conformance corpus must parse")
            .remove(0);
        let Form::Map(manifest) = manifest else {
            panic!("module conformance corpus must be a map")
        };
        let Some(Form::Map(profiles)) = entry(&manifest, "runtime/profiles") else {
            panic!("module conformance corpus must declare :runtime/profiles")
        };
        let Some(Form::Map(profile)) = entry(profiles, runtime) else {
            panic!("module conformance corpus has no :{runtime} profile")
        };
        entry(profile, key)
            .cloned()
            .unwrap_or_else(|| panic!("module runtime profile :{runtime} has no :{key}"))
    }

    fn host_conformance_case(id: &str) -> Vec<(Form, Form)> {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
        }

        let document_source = repo_text("00-unsorted/runtime/draft/host-runtime.edn")
            .expect("specs submodule must be initialized for host runtime conformance tests");
        let document = kernel::parse_forms(&document_source)
            .expect("Host runtime specification must parse")
            .remove(0);
        let Form::Map(document) = document else {
            panic!("Host runtime specification must be a map")
        };
        let Some(Form::Vector(cases)) = entry(&document, "host/conformance") else {
            panic!("Host runtime specification must declare :host/conformance")
        };
        cases
            .iter()
            .find_map(|case| {
                let Form::Map(case) = case else {
                    return None;
                };
                matches!(entry(case, "id"), Some(Form::Keyword(candidate)) if candidate == id)
                    .then(|| case.clone())
            })
            .unwrap_or_else(|| panic!("missing Host conformance case :{id}"))
    }

    #[test]
    fn session_kernel_mounts_preserve_state_and_enforce_lifetime() {
        let mut kernel = SessionKernel::new();
        let alpha = session_id("alpha");
        let beta = session_id("beta");
        kernel.create_session(alpha.clone()).unwrap();
        kernel.create_session(beta.clone()).unwrap();
        assert_eq!(kernel.eval(&alpha, "(def answer 41) answer").unwrap(), "41");
        assert_eq!(kernel.eval(&beta, "(def answer 6) answer").unwrap(), "6");
        let mount = kernel.create_memory_filesystem("/");
        kernel.attach_filesystem(&alpha, mount).unwrap();
        kernel.attach_filesystem(&beta, mount).unwrap();
        assert_eq!(kernel.filesystem(&alpha), Some(mount));
        assert_eq!(kernel.eval(&alpha, "answer").unwrap(), "41");
        assert_eq!(
            kernel
                .eval(
                    &alpha,
                    "(deref (std.native.File/write \"/shared.bin\" (bytes 7 8)))",
                )
                .unwrap(),
            "\"/shared.bin\""
        );
        assert_eq!(
            kernel
                .eval(&beta, "(deref (std.native.File/exists? \"/shared.bin\"))",)
                .unwrap(),
            "true"
        );
        assert_eq!(
            kernel
                .eval(
                    &alpha,
                    "(deref (std.native.File/write \"/source.hal\" \
                       (str/encode-utf8 \"(+ 19 23)\")))",
                )
                .unwrap(),
            "\"/source.hal\""
        );
        assert_eq!(
            kernel
                .eval(
                    &beta,
                    "(str/decode-utf8 \
                       (deref (std.native.File/read \"/source.hal\")))",
                )
                .unwrap(),
            "\"(+ 19 23)\""
        );
        assert_eq!(
            kernel.close_filesystem(mount).unwrap_err(),
            format!("FILESYSTEM_ATTACHED {mount}")
        );
        kernel.detach_filesystem(&alpha).unwrap();
        kernel.detach_filesystem(&beta).unwrap();
        kernel.close_filesystem(mount).unwrap();
        assert_eq!(
            kernel.session_names(),
            vec![session_id("ROOT"), alpha, beta]
        );
    }

    #[test]
    fn named_sessions_conform_to_context_component_and_applicative_protocols() {
        use crate::lang::protocol::{IApplicable, IComponent, IContext, IInvokeIn};

        let mut alpha = Session::new("alpha", Runtime::new());
        let mut beta = Session::new("beta", Runtime::new());

        assert!(alpha.started());
        assert_eq!(alpha.props().namespace, "user");
        assert_eq!(
            alpha.call("(do (ns alpha.core) (def answer 41) answer)"),
            Ok("41".into())
        );
        assert_eq!(alpha.props().namespace, "alpha.core");
        assert_eq!(beta.current_namespace(), "user");

        assert_eq!(alpha.apply_in(&mut beta, "(+ 20 22)"), Ok("42".into()));
        assert_eq!(alpha.invoke_in(&mut beta, "(+ 40 2)"), Ok("42".into()));
        assert_eq!(alpha.transform_in(&beta, "answer"), "answer");
        assert_eq!(
            alpha.transform_out(&beta, "answer", Ok("41".into())),
            Ok("41".into())
        );
        assert_eq!(alpha.apply_default().current_namespace(), "alpha.core");

        alpha.stop();
        assert!(alpha.stopped());
        assert_eq!(alpha.call("answer"), Err("SESSION_CLOSED alpha".into()));
    }

    fn ignore_socket_event(_event: core::SocketEvent) {}

    static SOCKET_EVENTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn count_socket_event(_event: core::SocketEvent) {
        SOCKET_EVENTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_socket_provider_sends_callbacks_and_bytes() {
        use crate::core::SocketProvider;
        use std::io::Read;
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 3];
            stream.read_exact(&mut bytes).unwrap();
            bytes
        });
        SOCKET_EVENTS.store(0, std::sync::atomic::Ordering::SeqCst);
        let sockets = core::NativeSocketProvider::default();
        let handle = sockets
            .connect("127.0.0.1", port, Rc::new(count_socket_event))
            .unwrap();
        assert_eq!(sockets.send(handle, &[7, 8, 9]).unwrap(), 3);
        sockets.close(handle).unwrap();
        assert_eq!(server.join().unwrap(), [7, 8, 9]);
        assert_eq!(SOCKET_EVENTS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_socket_server_streams_real_tcp_events() {
        use crate::core::{PromiseState, SocketProvider};
        use std::io::Write;
        let sockets = core::NativeSocketProvider::default();
        let server = sockets.listen("127.0.0.1", 0, Rc::new(|_| {})).unwrap();
        let (host, port) = sockets.endpoint(server).unwrap();
        let stream = sockets.events(server).unwrap();
        let mut client = std::net::TcpStream::connect((host.as_str(), port)).unwrap();
        let open = sockets.next(stream).unwrap().wait_state();
        assert!(
            matches!(open, PromiseState::Fulfilled(value) if value.display().contains(":open"))
        );
        client.write_all(b"ping").unwrap();
        let data = sockets.next(stream).unwrap().wait_state();
        assert!(
            matches!(data, PromiseState::Fulfilled(value) if value.display().contains(":data") && value.display().contains("112 105 110 103"))
        );
        sockets.close(server).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_file_provider_round_trips_bytes() {
        use crate::core::FileProvider;
        let path = std::env::temp_dir().join(format!("hara-wasm-test-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let provider = core::NativeFileProvider::new(&path);
        let resolved = provider
            .resolve(path.to_str().unwrap(), "data.bin")
            .unwrap();
        assert_eq!(
            provider.write(&resolved, vec![4, 5, 6]).unwrap().state(),
            core::PromiseState::Fulfilled(core::Value::Nil)
        );
        assert_eq!(
            provider.read(&resolved).unwrap().state(),
            core::PromiseState::Fulfilled(core::Value::Bytes(vec![4, 5, 6]))
        );
        std::fs::remove_file(resolved).unwrap();
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn extension_provider_values_load_and_iterate_through_protocols() {
        let mut runtime = Runtime::new();
        runtime.extensions.install(RangeExtension);
        assert!(runtime.extension_available("range"));
        assert_eq!(runtime.require_resource("range").unwrap(), ":loaded");
        let value = runtime
            .extensions
            .construct("range", "range", &[core::Value::Number(3)])
            .unwrap();
        assert_eq!(core::receiver_category(&value), "extension");
        runtime
            .execution
            .environment_mut()
            .insert("r".into(), value);
        assert_eq!(runtime.eval_text("(iter-next (iter r))").unwrap(), "0");
        assert_eq!(runtime.eval_text("(iter-next (iter r))").unwrap(), "0");
        assert_eq!(runtime.require_resource("range").unwrap(), ":loaded");
    }

    struct LazyMapExtension;

    impl core::ExtensionProvider for LazyMapExtension {
        fn name(&self) -> &str {
            "lazy-map"
        }

        fn install(&self, protocols: &mut core::ProtocolRegistry) {
            protocols.register_extension_category("lazy-map", "request", "map");
            protocols.register_extension(
                "lazy-map",
                "request",
                "std.protocol.ilookup.ILookup",
                "lookup",
                |arguments| match arguments {
                    [core::Value::Extension(value), key, default]
                        if value.provider == "lazy-map" && value.type_name == "request" =>
                    {
                        let matches = matches!(
                            key,
                            core::Value::Keyword(keyword) if keyword.as_str() == "value"
                        );
                        Ok(if matches {
                            core::Value::Number(value.handle as i64)
                        } else {
                            default.clone()
                        })
                    }
                    _ => Err("lazy-map/lookup expects its request extension".into()),
                },
            );
            protocols.register_extension(
                "lazy-map",
                "request",
                "std.protocol.icount.ICount",
                "count",
                |arguments| match arguments {
                    [core::Value::Extension(value)]
                        if value.provider == "lazy-map" && value.type_name == "request" =>
                    {
                        Ok(core::Value::Number(1))
                    }
                    _ => Err("lazy-map/count expects its request extension".into()),
                },
            );
        }

        fn construct(
            &self,
            type_name: &str,
            arguments: &[core::Value],
        ) -> Result<core::Value, String> {
            let [core::Value::Number(value)] = arguments else {
                return Err("lazy-map expects one numeric value".into());
            };
            if type_name != "request" || *value < 0 {
                return Err("lazy-map/request expects a non-negative value".into());
            }
            Ok(core::Value::Extension(core::ExtensionValue {
                provider: "lazy-map".into(),
                type_name: "request".into(),
                handle: *value as u64,
            }))
        }
    }

    #[test]
    fn extension_backed_maps_dispatch_collection_primitives() {
        let mut runtime = Runtime::new();
        runtime.extensions.install(LazyMapExtension);
        runtime.require_resource("lazy-map").unwrap();
        let value = runtime
            .extensions
            .construct("lazy-map", "request", &[core::Value::Number(42)])
            .unwrap();
        runtime
            .execution
            .environment_mut()
            .insert("request".into(), value);
        assert_eq!(runtime.eval_text("(:value request)").unwrap(), "42");
        assert_eq!(
            runtime
                .eval_text("(get request :missing :fallback)")
                .unwrap(),
            ":fallback"
        );
        assert_eq!(runtime.eval_text("(count request)").unwrap(), "1");
        assert_eq!(runtime.eval_text("(map? request)").unwrap(), "true");
    }

    #[test]
    fn runtime_routes_file_operations_through_provider_registry() {
        let mut runtime = Runtime::new();
        assert!(!runtime.file_supported());
        runtime.install_memory_file_provider("/");
        assert!(runtime.file_supported());
        let path = runtime.file_resolve("/", "data.bin").unwrap();
        assert_eq!(path, "/data.bin");
        assert_eq!(
            runtime
                .file_write(&path, vec![1, 2, 3])
                .unwrap()
                .value()
                .unwrap(),
            "\"/data.bin\""
        );
        assert_eq!(
            runtime.file_read(&path).unwrap().value().unwrap(),
            "#bytes[1 2 3]"
        );
        runtime.install_loopback_socket_provider();
        assert!(runtime.socket_supported());
    }

    #[test]
    fn runtime_core_evaluates_embedded_commands() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(+ 19 23)").unwrap(), "42");
        assert_eq!(runtime.eval_text("(let (x 7) (* x 6))").unwrap(), "42");
        assert_eq!(runtime.eval_text("(if true 1 0)").unwrap(), "1");
    }

    #[test]
    fn foundation_boolean_and_not_equal_are_portable() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(not nil)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(not :present)").unwrap(), "false");
        assert_eq!(runtime.eval_text("(boolean :present)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(boolean nil)").unwrap(), "false");
        assert_eq!(runtime.eval_text("(compare 1 2)").unwrap(), "-1");
        assert_eq!(runtime.eval_text("(not= 1 2)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(not= 1 1)").unwrap(), "false");
    }

    #[test]
    fn portable_pretty_renderer_groups_and_breaks_documents() {
        let mut runtime = Runtime::new();
        runtime.require_resource("std.foundation.pretty").unwrap();
        assert_eq!(
            runtime
                .eval_text("(std.foundation.pretty/render \"abc\")")
                .unwrap(),
            "\"abc\""
        );
        let document = "[:document/group \"(\" [:document/nest 2 [:document/line] \"alpha\" [:document/line] \"beta\"] \")\"]";
        assert_eq!(
            runtime
                .eval_text(&format!(
                    "(std.foundation.pretty/render {document} {{:width 80}})"
                ))
                .unwrap(),
            "\"( alpha beta)\""
        );
        assert_eq!(
            runtime
                .eval_text(&format!(
                    "(std.foundation.pretty/render {document} {{:width 8}})"
                ))
                .unwrap(),
            "\"(\\n  alpha\\n  beta)\""
        );
        assert_eq!(
            runtime
                .eval_text("(std.foundation.pretty/pprint-str {:b 2 :a 1})")
                .unwrap(),
            "\"{:a 1, :b 2}\""
        );
    }

    #[test]
    fn threading_macros_expand_finite_iterator_clauses() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(cond-> 1 (= 1 1) inc)").unwrap(), "2");
        assert_eq!(runtime.eval_text("(cond-> 1 (= 1 2) inc)").unwrap(), "1");
        assert_eq!(runtime.eval_text("(cond->> 1 (= 1 1) inc)").unwrap(), "2");
        assert_eq!(runtime.eval_text("(cond->> 1 (= 1 2) inc)").unwrap(), "1");
        assert_eq!(
            runtime.eval_text("(vec (drop 2 [1 2 3 4]))").unwrap(),
            "[3 4]"
        );
    }

    #[test]
    fn hara_file_operations_use_capability_providers() {
        let mut runtime = Runtime::new();
        runtime.register_resource(
            "std.fs.path",
            include_str!("../../../lib/src/std/fs/path.hal"),
        );
        runtime
            .eval_text("(require [std.fs.path :as path])")
            .unwrap();
        assert_eq!(
            runtime.eval_text("(path/parent \"/a/b\")").unwrap(),
            "\"/a\""
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(try (File/parent \"/a/b\")\n                           (catch error (get (ex-data error) :ex/code)))",
                )
                .unwrap(),
            ":native/capability-denied"
        );
        assert_eq!(
            runtime.eval_text("(path/parent \"/\")").unwrap(),
            "\"/\""
        );
        assert_eq!(
            runtime.eval_text("(path/join \"/a\" \"b\")").unwrap(),
            "\"/a/b\""
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(try
                       (deref (File/read \"/data.bin\"))
                       (catch error (get (ex-data error) :ex/code)))",
                )
                .unwrap(),
            ":native/capability-denied"
        );

        runtime.install_memory_file_provider("/");
        assert_eq!(
            runtime
                .eval_text("(File/resolve \"/sandbox\" \"data.bin\")")
                .unwrap(),
            "\"/sandbox/data.bin\""
        );
        assert_eq!(
            runtime
                .eval_text("(std.native.File/parent \"/sandbox/data.bin\")")
                .unwrap(),
            "\"/sandbox\""
        );
        assert_eq!(
            runtime.eval_text("(std.native.File/parent \"/\")").unwrap(),
            "nil"
        );
        assert!(runtime
            .eval_text("(std.native.File/parent \"/../escape\")")
            .unwrap_err()
            .contains("file/outside-root"));
        assert!(runtime
            .eval_text("(File/resolve \"/\" \"../escape\")")
            .unwrap_err()
            .contains("file/outside-root"));
        assert_eq!(
            runtime
                .eval_text("(deref (File/write \"/data.bin\" (bytes 0 127 255)))")
                .unwrap(),
            "\"/data.bin\""
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/read \"/data.bin\"))")
                .unwrap(),
            "#bytes[0 127 -1]"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/exists? \"/data.bin\"))")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let (entry (deref (File/stat \"/data.bin\")))
                       [(:path entry) (:name entry) (:size entry) (:type entry)
                        (map? (:extensions entry))])",
                )
                .unwrap(),
            "[\"/data.bin\" \"data.bin\" 3 :file true]"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/exists? \"/missing.bin\"))")
                .unwrap(),
            "false"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/write \"/list/b.bin\" (bytes 2) {:parents? true}))",)
                .unwrap(),
            "\"/list/b.bin\""
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/write \"/list/a.bin\" (bytes 1)))")
                .unwrap(),
            "\"/list/a.bin\""
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(mapv (fn [entry] (:path entry))
                           (deref (File/entries \"/list\")))",
                )
                .unwrap(),
            "[\"/list/a.bin\" \"/list/b.bin\"]"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/copy \"/data.bin\" \"/copy.bin\"))")
                .unwrap(),
            "\"/copy.bin\""
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/move \"/copy.bin\" \"/moved.bin\"))")
                .unwrap(),
            "\"/moved.bin\""
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/delete \"/data.bin\"))")
                .unwrap(),
            "\"/data.bin\""
        );
        assert_eq!(
            runtime
                .eval_text("(deref (File/exists? \"/data.bin\"))")
                .unwrap(),
            "false"
        );
    }

    #[test]
    fn hara_socket_operations_use_callback_providers() {
        let mut runtime = Runtime::new();
        let error = runtime
            .eval_text("(socket/connect \"localhost\" 8080 {} (fn [error socket] socket))")
            .unwrap_err();
        assert!(
            error.contains("native/capability-denied"),
            "unexpected error: {error}"
        );

        runtime.install_loopback_socket_provider();
        assert_eq!(
            runtime
                .eval_text("(def socket-handle (socket/connect \"localhost\" 8080 {} (fn [error socket] socket)))")
                .unwrap(),
            "#'user/socket-handle"
        );
        assert_eq!(
            runtime
                .eval_text("(socket/send socket-handle (bytes 0 127 255))")
                .unwrap(),
            "3"
        );
        assert_eq!(
            runtime.eval_text("(socket/close socket-handle)").unwrap(),
            "nil"
        );
        assert!(runtime
            .eval_text("(socket/send socket-handle (bytes 1))")
            .unwrap_err()
            .contains("socket/invalid"));
    }

    #[test]
    fn provider_registry_reports_installed_capabilities() {
        let mut registry = core::ProviderRegistry::new();
        assert_eq!(
            registry.capabilities(),
            core::ProviderCapabilities {
                file: false,
                socket: false,
                process: false
            }
        );
        registry.install_file(core::MemoryFileProvider::new("/sandbox"));
        registry.install_socket(core::LoopbackSocketProvider::default());
        registry.install_process();
        assert_eq!(
            registry.capabilities(),
            core::ProviderCapabilities {
                file: true,
                socket: true,
                process: true
            }
        );
        assert!(registry.file().is_some());
        assert!(registry.socket().is_some());
        assert!(registry.process());
    }

    #[test]
    fn runtime_routes_socket_handles_through_callback_provider() {
        let mut runtime = Runtime::new();
        runtime.install_loopback_socket_provider();
        let socket = runtime.socket_connect("localhost", 8080).unwrap();
        assert_eq!(runtime.socket_send(socket, vec![1, 2, 3]).unwrap(), 3);
        runtime.socket_close(socket).unwrap();
    }

    #[test]
    fn loopback_socket_is_callback_based_and_counts_bytes() {
        use crate::core::SocketProvider;
        SOCKET_EVENTS.store(0, std::sync::atomic::Ordering::SeqCst);
        let sockets = core::LoopbackSocketProvider::default();
        let handle = sockets
            .connect("localhost", 8080, Rc::new(count_socket_event))
            .unwrap();
        assert_eq!(sockets.send(handle, &[1, 2, 3]).unwrap(), 3);
        sockets.close(handle).unwrap();
        assert_eq!(SOCKET_EVENTS.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(
            sockets.send(handle, &[9]).unwrap_err(),
            core::SocketError::Invalid("unknown socket".into())
        );
    }

    #[test]
    fn memory_file_provider_exposes_one_logical_root_and_preserves_bytes() {
        use crate::core::FileProvider;
        let files = core::MemoryFileProvider::new("ignored-host-label");
        assert_eq!(files.resolve("/", "docs/../secret").unwrap(), "/secret");
        assert_eq!(
            files.resolve("/", "../escape").unwrap_err(),
            core::FileError::OutsideRoot
        );
        let path = files.resolve("/", "data.bin").unwrap();
        let write = files.write(&path, vec![0, 127, 255]).unwrap();
        assert_eq!(
            write.state(),
            core::PromiseState::Fulfilled(core::Value::String("/data.bin".into()))
        );
        let read = files.read(&path).unwrap();
        assert_eq!(
            read.state(),
            core::PromiseState::Fulfilled(core::Value::Bytes(vec![0, 127, 255]))
        );
        assert_eq!(
            files.read("../outside").unwrap_err(),
            core::FileError::OutsideRoot
        );
    }

    #[test]
    fn unsupported_capabilities_fail_stably() {
        use crate::core::{FileProvider, SocketProvider};
        let files = core::UnsupportedFileProvider;
        assert_eq!(
            files.resolve("/root", "data.bin").unwrap(),
            "/root/data.bin"
        );
        assert!(matches!(
            files.read("data.bin").unwrap().state(),
            core::PromiseState::Rejected(_)
        ));
        let sockets = core::UnsupportedSocketProvider;
        assert_eq!(
            sockets
                .connect("localhost", 80, Rc::new(ignore_socket_event))
                .unwrap_err(),
            core::SocketError::Unsupported
        );
        assert_eq!(
            sockets.send(1, &[1, 2]).unwrap_err(),
            core::SocketError::Unsupported
        );
        assert_eq!(
            sockets.close(1).unwrap_err(),
            core::SocketError::Unsupported
        );
    }

    #[test]
    fn namespace_aliases_route_evaluation_and_resources() {
        let mut runtime = Runtime::new();
        assert!(runtime.create_namespace("hara.math"));
        assert!(runtime.alias_namespace("math", "hara.math"));
        assert_eq!(runtime.resolve_namespace("math"), "hara.math");
        assert_eq!(
            runtime
                .eval_in_namespace("math", "(defn answer [] 42) (answer)")
                .unwrap(),
            "42"
        );
        runtime.register_resource("helpers", "(defn helper [] 7) (helper)");
        assert_eq!(
            runtime
                .require_resource_in_namespace("helpers", "math")
                .unwrap(),
            "7"
        );
        assert_eq!(runtime.eval_text("(helper)").unwrap(), "7");
    }

    #[test]
    fn native_host_routes_calls_without_a_facade() {
        let mut runtime = Runtime::new();
        let error = runtime
            .eval_text(
                "(deref (std.native.Host/call \"browser.dom\" \"set-text\" [\"#sel\" \"hi\"]))",
            )
            .unwrap_err();
        assert!(
            error.contains("native/capability-denied"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn host_modules_route_through_the_native_type() {
        let mut runtime = Runtime::new();
        runtime.register_resource(
            "host.browser.dom",
            "(ns host.browser.dom) (defn set-text [selector text] (std.native.Host/call \"browser.dom\" \"set-text\" [selector text]))",
        );
        let error = runtime
            .eval_text(
                "(ns user (:require [host.browser.dom :as dom])) (deref (dom/set-text \"#sel\" \"hi\"))",
            )
            .unwrap_err();
        assert!(
            error.contains("native/capability-denied"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn namespace_registry_owns_qualified_vars_without_changing_identity() {
        let mut runtime = Runtime::new();
        runtime.use_namespace("alpha");
        runtime
            .eval_text("(def ^{:dynamic true} answer 41)")
            .unwrap();
        let local = match runtime.execution.environment().get("answer").unwrap() {
            core::Value::Var(var) => var.clone(),
            _ => panic!("definition must be a Var"),
        };
        assert_eq!(local.symbol().as_str(), "alpha/answer");
        let qualified = match runtime.execution.environment().get("alpha/answer").unwrap() {
            core::Value::Var(var) => var.clone(),
            _ => panic!("qualified definition must be a Var"),
        };
        assert!(local.same_identity(&qualified));
        assert!(qualified.is_dynamic());
        runtime.use_namespace("user");
        runtime.alias_namespace("a", "alpha");
        let alias = match runtime.execution.environment().get("a/answer").unwrap() {
            core::Value::Var(var) => var.clone(),
            _ => panic!("alias must resolve to a Var"),
        };
        assert!(local.same_identity(&alias));
    }

    #[test]
    fn qualified_namespace_symbols_resolve_shared_vars_and_aliases() {
        let mut runtime = Runtime::new();
        assert!(runtime.create_namespace("alpha"));
        assert_eq!(
            runtime
                .eval_in_namespace("alpha", "(def answer 41)")
                .unwrap(),
            "#'alpha/answer"
        );
        runtime.use_namespace("user");
        assert_eq!(runtime.eval_text("alpha/answer").unwrap(), "41");
        assert!(runtime.alias_namespace("a", "alpha"));
        assert_eq!(runtime.eval_text("a/answer").unwrap(), "41");
        assert_eq!(
            runtime
                .eval_text("(do (set! alpha/answer 42) alpha/answer)")
                .unwrap(),
            "42"
        );
        runtime.use_namespace("alpha");
        assert_eq!(runtime.eval_text("answer").unwrap(), "42");
    }

    #[test]
    fn required_sources_predeclare_forward_globals_for_facade_aliases() {
        let mut runtime = Runtime::new();
        runtime.register_resource(
            "example.forward",
            "(ns example.forward) (defn call-answer [] (answer)) (defn answer [] 42)",
        );
        runtime.register_resource(
            "example.facade",
            "(ns example.facade (:require [example.forward :as forward])) \
             (def answer forward/call-answer)",
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(ns example.probe (:require [example.facade :as facade])) \
                     (facade/answer)"
                )
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn get_returns_the_default_for_non_associative_sequences() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("[(get (seq [1 2]) :missing) (get (seq [1 2]) :missing 42)]")
                .unwrap(),
            "[nil 42]"
        );
    }

    #[test]
    fn functions_resolve_lazy_globals_in_their_defining_namespace() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (ns example.function-a) (def answer 41) (defn read-answer [] answer) (ns example.function-b) (def answer 42) (example.function-a/read-answer))",
                )
                .unwrap(),
            "41"
        );
        assert_eq!(runtime.current_namespace(), "example.function-b");
    }

    #[test]
    fn functions_resolve_unqualified_vars_to_full_definition_symbols() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (ns example.resolver) (def answer 41) \
                     (defn answer-symbol [] \
                       (std.foundation/var-sym (std.foundation/resolve 'answer))) \
                     (ns example.caller) (example.resolver/answer-symbol))",
                )
                .unwrap(),
            "example.resolver/answer"
        );
    }

    #[test]
    fn namespace_declaration_restores_declared_namespace_after_requires() {
        let mut runtime = Runtime::new();
        runtime.register_resource("example.required", "(ns example.required) (def answer 42)");
        runtime
            .eval_text("(ns example.client (:require [example.required :as required]))")
            .unwrap();
        assert_eq!(runtime.current_namespace(), "example.client");
        assert_eq!(runtime.eval_text("required/answer").unwrap(), "42");

        runtime.use_namespace("user");
        runtime
            .eval_text("(require [example.required :as direct])")
            .unwrap();
        assert_eq!(runtime.current_namespace(), "user");
        assert_eq!(runtime.eval_text("direct/answer").unwrap(), "42");
    }

    #[test]
    fn dash_qualifier_resolves_values_and_vars_in_the_current_namespace() {
        let mut runtime = Runtime::new();
        runtime.use_namespace("example.current");
        assert_eq!(
            runtime
                .eval_text("(def answer 42) [answer -/answer (= #'answer #'-/answer)]")
                .unwrap(),
            "[42 42 true]"
        );
        assert_eq!(runtime.eval_text("(quote -/answer)").unwrap(), "-/answer");
        assert!(runtime
            .eval_text("-/missing")
            .unwrap_err()
            .contains("unbound symbol"));
    }

    #[test]
    fn defn_schema_var_references_must_resolve() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(def Customer [:map [:id :int]]) \
                     (defn ^{:schema #'-/Customer} customer-id [customer] (get customer :id)) \
                     (customer-id {:id 42})",
                )
                .unwrap(),
            "42"
        );
        assert!(runtime
            .eval_text("(defn ^{:schema #'MissingSchema} invalid [value] value)")
            .unwrap_err()
            .contains("schema Var does not exist: MissingSchema"));
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn bytecode_vm_canonicalizes_the_current_namespace_qualifier() {
        let mut runtime = Runtime::new();
        runtime.use_namespace("example.bytecode-current");
        assert_eq!(
            runtime
                .eval_bytecode_native("(def answer 42) [answer -/answer (= #'answer #'-/answer)]")
                .unwrap(),
            "[42 42 true]"
        );
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn bytecode_compiler_checks_named_schema_vars() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_bytecode_native(
                    "(def Customer [:map [:id :int]]) \
                     (defn ^{:schema #'-/Customer} customer-id [customer] (get customer :id)) \
                     (customer-id {:id 42})",
                )
                .unwrap(),
            "42"
        );
        assert!(runtime
            .compile_bytecode("(defn ^{:schema #'MissingSchema} invalid [value] value)")
            .unwrap_err()
            .contains("schema Var does not exist: MissingSchema"));
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_is_an_explicit_runtime_backend() {
        let mut runtime = Runtime::core();
        assert_eq!(runtime.execution_backend(), "interpreter");
        runtime
            .configure_execution_backend("direct-native")
            .unwrap();
        assert_eq!(runtime.execution_backend(), "direct-native");
        assert_eq!(
            runtime.eval_native("(let [value 20] (+ value 22))").unwrap(),
            "42"
        );
        runtime
            .eval_native("(defn add-one [value] (+ value 1))")
            .unwrap();
        assert_eq!(runtime.eval_native("(add-one 41)").unwrap(), "42");
        runtime
            .configure_execution_backend("interpreter")
            .unwrap();
        assert_eq!(runtime.execution_backend(), "interpreter");
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_keeps_interpreter_owned_functions_out_of_the_native_scope() {
        let mut runtime = Runtime::core();
        runtime
            .eval_native("(defn interpreted [value] (+ value 1))")
            .unwrap();
        runtime
            .configure_execution_backend("direct-native")
            .unwrap();
        let error = runtime
            .eval_native("(interpreted 41)")
            .expect_err("a function created by the interpreter must not cross the native scope");
        assert!(
            error.contains("evaluator- or fiber-backed Hara function"),
            "{error}"
        );
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_compiles_runtime_eval_without_fallback() {
        let mut runtime = Runtime::core();
        runtime
            .configure_execution_backend("direct-native")
            .unwrap();
        assert_eq!(
            runtime.eval_native("(Runtime/eval '(+ 1 2))").unwrap(),
            "3"
        );
        assert_eq!(
            runtime
                .eval_native("(Runtime/load-string \"(+ 19 23)\")")
                .unwrap(),
            "42"
        );
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_async_functions_return_promises_on_the_fast_path() {
        let mut runtime = Runtime::core();
        runtime
            .configure_execution_backend("direct-native")
            .unwrap();
        assert_eq!(
            runtime
                .eval_native("(do (defn ^:async answer [] 42) (answer))")
                .unwrap(),
            "<promise>"
        );
        assert_eq!(
            runtime
                .eval_native("(std.protocol.ideref.IDeref/deref (answer))")
                .unwrap(),
            "42"
        );
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn session_kernel_propagates_the_shared_native_backend() {
        let mut kernel = SessionKernel::new();
        kernel.set_execution_backend("direct-native").unwrap();
        let root = SessionId::parse("ROOT").unwrap();
        assert_eq!(
            kernel
                .eval(&root, "(let [value 20] (+ value 22))")
                .unwrap(),
            "42"
        );
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_loads_source_namespaces_through_the_shared_loader() {
        let mut runtime = Runtime::core();
        runtime.register_resource(
            "example.direct-dependency",
            "(ns example.direct-dependency) (defn increment [value] (+ value 1))",
        );
        runtime.register_resource(
            "example.direct",
            "(ns example.direct (:require [example.direct-dependency :as dependency])) (defn answer [value] (dependency/increment value))",
        );
        runtime
            .configure_execution_backend("direct-native")
            .unwrap();
        assert_eq!(
            runtime
                .eval_native("(require [example.direct :as direct]) (direct/answer 41)")
                .unwrap(),
            "42"
        );
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_loads_a_namespace_requested_inside_native_code() {
        let mut runtime = Runtime::core();
        runtime.register_resource(
            "example.direct-late-dependency",
            "(ns example.direct-late-dependency) (defn increment [value] (+ value 1))",
        );
        runtime
            .configure_execution_backend("direct-native")
            .unwrap();

        // The lazy alias makes the namespace visible to the compiler without
        // materializing it. The nested require therefore exercises the native
        // loader while a native-substrate frame is active; it must not call
        // core::eval for the loaded namespace declaration.
        runtime
            .eval_native(
                "(require [example.direct-late-dependency :as dependency :lazy true])",
            )
            .unwrap();
        runtime
            .eval_native(
                "(defn load-late [value] (require [example.direct-late-dependency]) (dependency/increment value))",
            )
            .unwrap();
        assert_eq!(runtime.eval_native("(load-late 41)").unwrap(), "42");
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_engine_shares_telemetry_without_sharing_runtime_state() {
        let engine = crate::direct_native::NativeEngine::new();
        let mut first = Runtime::with_native_engine(engine.clone());
        first
            .configure_execution_backend("direct-native")
            .unwrap();
        assert_eq!(first.eval_native("(def isolated 41) isolated").unwrap(), "41");
        let first_telemetry = first.native_execution_telemetry();
        assert!(first_telemetry.bytecode_functions > 0);
        assert!(first_telemetry.bytecode_instructions > 0);

        let mut second = Runtime::with_native_engine(engine);
        second
            .configure_execution_backend("direct-native")
            .unwrap();
        assert_eq!(second.eval_native("(def isolated 42) isolated").unwrap(), "42");
        let after_definitions = second.native_execution_telemetry();
        assert_eq!(first.eval_native("isolated").unwrap(), "41");
        assert_eq!(second.eval_native("isolated").unwrap(), "42");
        assert_eq!(
            second.eval_native("(let [value 20] (+ value 22))").unwrap(),
            "42"
        );
        let second_telemetry = second.native_execution_telemetry();
        assert!(
            second_telemetry.bytecode_functions > after_definitions.bytecode_functions,
            "each native entry validates its bytecode unit"
        );
        assert!(
            second_telemetry.native_target_calls > after_definitions.native_target_calls,
            "native target count did not advance: before={}, after={}",
            after_definitions.native_target_calls,
            second_telemetry.native_target_calls
        );
        assert!(second_telemetry.invocations > first_telemetry.invocations);
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    #[test]
    fn direct_native_loads_bytecode_namespaces_through_the_shared_loader() {
        let modules = [
            crate::vm::ModuleSource {
                resource: "example.direct-artifact",
                source: "(ns example.direct-artifact (:require [example.direct-artifact-dependency :as dependency])) (defn answer [value] (dependency/increment value))",
            },
            crate::vm::ModuleSource {
                resource: "example.direct-artifact-dependency",
                source: "(ns example.direct-artifact-dependency) (defn increment [value] (+ value 1))",
            },
        ];
        let bundle = crate::vm::compile_bytecode_bundle(&modules).unwrap();
        let mut runtime = Runtime::core();
        runtime
            .configure_execution_backend("direct-native")
            .unwrap();
        crate::vm::eval_bytecode_bundle(&mut runtime, &bundle).unwrap();
        runtime
            .eval_native("(require [example.direct-artifact])")
            .unwrap();
        assert!(runtime.use_namespace("example.direct-artifact"));
        assert_eq!(runtime.eval_native("(answer 41)").unwrap(), "42");
    }

    #[test]
    fn resolve_does_not_load_an_unregistered_qualified_resource() {
        let mut runtime = Runtime::new();
        runtime.register_resource("demo.required", "(ns demo.required) (def answer 42)");
        assert_eq!(
            runtime
                .eval_text("(resolve 'demo.required/answer)")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime
                .eval_text("[(boolean (Base/resolve 'std.foundation/resolve)) (Base/resolve 'demo.required/answer)]")
                .unwrap(),
            "[true nil]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(ns gate.resolve (:require [demo.required :as required])) required/answer"
                )
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn namespaces_isolate_bindings_and_can_be_selected() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.current_namespace(), "user");
        assert!(runtime.create_namespace("math"));
        runtime.eval_text("(defn answer [] 42)").unwrap();
        runtime.use_namespace("math");
        assert_eq!(
            runtime.eval_text("(defn answer [] 7) (answer)").unwrap(),
            "7"
        );
        runtime.use_namespace("user");
        assert_eq!(runtime.eval_text("(answer)").unwrap(), "42");
        runtime.use_namespace("math");
        assert_eq!(runtime.eval_text("(answer)").unwrap(), "7");
    }

    #[test]
    fn generated_namespaces_configure_aliases_refers_and_intrinsics_without_sources() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(str/trim \"  hara  \")").unwrap(),
            "\"hara\""
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(ns app (:config {:rename {:exclude [bytes] :alias {string text}}})                       (:require [hara.lib.string :as s :refer [trim]]))                       (trim (s/trim (text/upper \" x \")))"
                )
                .unwrap(),
            "\"X\""
        );
        assert_eq!(
            runtime
                .eval_text("(get (ns-alias-state 'bytes) :state)")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime
                .eval_text("(ns core-user (:require [hara.lib.core :as core])) (core/bit-not 0)")
                .unwrap(),
            "-1"
        );
    }

    #[test]
    fn generated_namespace_require_never_falls_back_to_registered_source() {
        let mut runtime = Runtime::new();
        runtime.register_resource("std.foundation.string", "(def poisoned 42)");
        assert_eq!(
            runtime
                .eval_text("(ns app (:require [hara.lib.string :as text])) (text/trim \" x \")")
                .unwrap(),
            "\"x\""
        );
        assert!(runtime
            .eval_text("poisoned")
            .unwrap_err()
            .contains("unbound symbol"));
    }

    #[test]
    fn strict_native_json_matches_the_portable_contract() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(std.native.Json/read \"[null,true,-2,\\\"x\\\",[3],{\\\"a\\\":4}]\")")
                .unwrap(),
            "[nil true -2 \"x\" [3] {\"a\" 4}]"
        );
        assert_eq!(
            runtime
                .eval_text("(std.native.Json/write {\"a\" 1 \"b\" [true nil]})")
                .unwrap(),
            "\"{\\\"a\\\":1,\\\"b\\\":[true,null]}\""
        );
        assert_eq!(
            runtime.eval_text("(Json/write {\"a\" 1})").unwrap(),
            "\"{\\\"a\\\":1}\""
        );
        assert_eq!(
            runtime
                .eval_text("(std.native.Json/pretty {\"a\" 1} {})")
                .unwrap(),
            "\"{\\n  \\\"a\\\": 1\\n}\""
        );
        assert!(runtime
            .eval_text("(std.native.Json/pretty {\"a\" 1} nil)")
            .unwrap_err()
            .contains("options map"));
        assert!(runtime
            .eval_text("(std.native.Json/read \"1.5\")")
            .unwrap_err()
            .contains("signed 64-bit integers"));
        assert_eq!(
            runtime.eval_text("(pretty/pprint-str {:a [1 2]})").unwrap(),
            "\"{:a [1 2]}\""
        );
    }

    #[test]
    fn restricted_native_edn_reads_and_writes_without_evaluation() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(std.native.Edn/read \"{:a [1 2] :b #{:x}}\")")
                .unwrap(),
            "{:a [1 2] :b #{:x}}"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "[(std.native.Edn/write {:a [1 2]}) \
                      (std.native.Edn/pretty [:a 1] {})]"
                )
                .unwrap(),
            "[\"{:a [1 2]}\" \"[:a 1]\"]"
        );
        assert!(runtime
            .eval_text("(std.native.Edn/pretty [:a 1] nil)")
            .unwrap_err()
            .contains("options map"));
        assert_eq!(
            runtime
                .eval_text("(std.native.Edn/read \"(+ 1 2)\")")
                .unwrap(),
            "(+ 1 2)"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(try \
                       (throw (ex-info \"bad input\" {:kind :invalid})) \
                       (catch Throwable error \
                         [(ex-message error) (ex-data error)]))"
                )
                .unwrap(),
            "[\"bad input\" {:kind :invalid}]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(IExInfo/data \
                       (ex-info \"bad input\" {:kind :invalid}))"
                )
                .unwrap(),
            "{:kind :invalid}"
        );
        for source in ["1/2", "1 2"] {
            let escaped = source.replace('\\', "\\\\").replace('"', "\\\"");
            assert!(runtime
                .eval_text(&format!("(std.native.Edn/read \"{escaped}\")"))
                .is_err());
        }
    }

    #[test]
    fn resource_sources_accept_namespace_declarations() {
        let mut runtime = Runtime::new();
        runtime.register_resource(
            "module",
            "(ns demo (:require [core])) (defn answer [] 42) (answer)",
        );
        assert_eq!(runtime.load_resource("module").unwrap(), "42");
    }

    #[test]
    fn substrate_protocol_resource_loads_in_the_native_runtime() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text("(require 'std.substrate.protocol) :loaded")
                .unwrap(),
            ":loaded"
        );
    }

    #[test]
    fn guest_struct_protocols_dispatch_like_truffle() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (defstruct Box [value]) \
                     (defprotocol BoxOps (read [self]) (add [self amount])) \
                     (extend-type Box BoxOps \
                       (read [self] (:value self)) \
                       (add [self amount] (+ (:value self) amount))) \
                     [(read (Box 40)) \
                      (add (map->Box {:value 40}) 2) \
                      (user/read (Box 41)) \
                      (instance? Box (Box 1))])",
                )
                .unwrap(),
            "[40 42 41 true]"
        );
        assert!(runtime
            .eval_text(
                "(do (ns protocol-probe (:config {:blank true}) (:require [std.foundation :refer :all :exclude [get]])) (defstruct Missing []) (defprotocol Needed (get [self])) \
                     (get (Missing)))",
            )
            .unwrap_err()
            .contains("missing protocol implementation: protocol-probe.Needed/get"));
    }

    #[test]
    fn keyword_invocation_uses_defstruct_map_semantics() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (defstruct Point [x y]) \
                     (let [point (map->Point {:x 1 :extra 9})] \
                       [(:x point) (:missing point 7) (:extra point) \
                        (get point :x) (type point)]))",
                )
                .unwrap(),
            "[1 7 nil 1 :user.Point]"
        );
    }

    #[test]
    fn typed_named_values_publish_one_type_schema_contract() {
        let source = "(do (defstruct Person [[name :str] [age {:optional true} :int]]) \
                      (let [schema (std.native.Schema/of (var Person)) \
                            person (Person \"Ada\" 37)] \
                        [(std.native.Schema/kind schema) \
                         (= schema (std.native.Schema/of (var Person))) \
                         (std.native.Schema/form schema) \
                         (:name person) (:age (map->Person {:name \"Ada\" :age 37})) \
                         (nil? (get (meta (var ->Person)) :schema))]))";
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text(source).unwrap(),
            "[:struct true [:struct (var user/Person) [:name :str] [:age {:optional true} :int]] \"Ada\" 37 true]"
        );

        assert!(runtime
            .eval_text("(defstruct Broken [[value :int] [value :str]])")
            .unwrap_err()
            .contains("Duplicate defstruct field"));
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn bytecode_typed_named_values_match_the_evaluator() {
        let source = "(do (defmutable Cursor [[position :int] [limit {:optional true} :int]]) \
                      (let [cursor (Cursor 2 10)] \
                        [(std.native.Schema/kind (std.native.Schema/of (var Cursor))) \
                         (field cursor :position) (field cursor :limit)]))";
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_native(source).unwrap(), "[:struct 2 10]");
        assert_eq!(runtime.eval_bytecode_native(source).unwrap(), "[:struct 2 10]");
    }

    #[test]
    fn map_destructuring_uses_defstruct_lookup_semantics() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (defstruct Point [x y]) \
                     [(let [{:keys [x y missing] :or {missing 7} :as point} \
                            (Point 1 2)] \
                        [x y missing (type point)]) \
                      ((fn [{:keys [x y]}] [x y]) (Point 3 4))])",
                )
                .unwrap(),
            "[[1 2 7 :user.Point] [3 4]]"
        );
    }

    #[test]
    fn compatibility_primitives_cover_renamed_maps_cons_and_parsing() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(let [{answer :answer :as whole} {:answer 42}] [answer whole]) \
                     (let [[first second & more] (cons 1 (cons 2 (cons 3 nil)))] \
                       [first second more]) \
                     (list? (cons 1 nil)) (cons? (cons 1 nil)) \
                     (std.native.Num/parse-long \"42\") \
                     (std.native.Num/parse-long \"4x\") \
                     (std.native.Num/parse-double \"3x\") \
                     (std.native.String/split \"\" \",\") \
                     (std.native.RegExp/split (std.native.RegExp/compile \",\") \"\")]",
                )
                .unwrap(),
            "[[42 {:answer 42}] [1 2 [3]] false true 42 nil nil nil nil]"
        );
    }

    #[test]
    fn guest_mutables_share_storage_and_reject_persistent_updates() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (defmutable Cursor [x y]) \
                     (let [cursor (Cursor 1 2) alias cursor order (atom []) \
                           snapshot (into {} cursor) \
                           result (set! (field (do (swap! order conj :receiver) cursor) :x) \
                                        (do (swap! order conj :replacement) 10))] \
                       [result @order (field alias :x) (= cursor alias) \
                        (= cursor (Cursor 10 2)) snapshot (into {} cursor)]))",
                )
                .unwrap(),
            "[10 [:receiver :replacement] 10 true false {:x 1 :y 2} {:x 10 :y 2}]"
        );
        assert!(runtime
            .eval_text("(assoc (Cursor 1 2) :x 3)")
            .unwrap_err()
            .contains("assoc does not support mutable values"));
        assert!(runtime
            .eval_text("(dissoc (Cursor 1 2) :x)")
            .unwrap_err()
            .contains("dissoc does not support mutable values"));
        assert!(runtime
            .eval_text("(do (defstruct Point [x]) (field (Point 1) :x))")
            .unwrap_err()
            .contains("field expects a mutable value"));
    }

    #[test]
    fn guest_protocol_dispatch_can_register_protocols_during_a_method_call() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (defstruct Loader []) \
                     (defprotocol Loading (load [self])) \
                     (extend-type Loader Loading \
                       (load [self] \
                         (do (defstruct Loaded [value]) \
                             (defprotocol Reading (read-loaded [self])) \
                             (extend-type Loaded Reading \
                               (read-loaded [self] (:value self))) \
                             (read-loaded (Loaded 42))))) \
                     (load (Loader)))",
                )
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn guest_protocol_methods_reload_and_reject_collisions_atomically() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(do (defstruct Box [value]) \
                     (defprotocol BoxOps (read [self])) \
                     (extend-type Box BoxOps (read [self] (:value self))) \
                     [(read (Box 41)) (user/read (Box 42))])",
                )
                .unwrap(),
            "[41 42]"
        );
        assert_eq!(
            runtime
                .eval_text("(defprotocol BoxOps (read [self]))")
                .unwrap(),
            "#protocol[user/BoxOps]"
        );
        let collision = runtime
            .eval_text(
                "(do (def ordinary 1) \
                 (defprotocol Broken (fresh [self]) (ordinary [self])))",
            )
            .unwrap_err();
        assert!(collision.contains("Protocol method Var already exists"));
        assert_eq!(runtime.eval_text("ordinary").unwrap(), "1");
        assert!(runtime.eval_text("fresh").is_err());
        assert!(runtime.eval_text("Broken").is_err());
        assert!(runtime
            .eval_text("(protocol-call BoxOps read (Box 1))")
            .is_err());
        assert!(runtime.eval_text("(BoxOps/read (Box 1))").is_err());
    }

    #[test]
    fn required_guest_protocol_methods_are_called_through_namespace_aliases() {
        let mut runtime = Runtime::new();
        runtime.register_resource(
            "acme.box",
            "(ns acme.box) \
             (defstruct Box [value]) \
             (defprotocol BoxOps (read [self])) \
             (extend-type Box BoxOps (read [self] (:value self)))",
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(ns consumer (:require [acme.box :as box])) \
                     (box/read (acme.box/Box 42))"
                )
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn foundation_protocols_are_canonical() {
        let mut runtime = Runtime::new();
        let Some(contract) = repo_text("01-lang/001-language/draft/conformance/protocols.edn")
        else {
            return;
        };
        let fixture = repo_text(
            "01-lang/001-language/draft/conformance/fixtures/protocol_surface.hal",
        )
        .expect("the specs-owned protocol surface fixture must be available");
        let foundation = runtime
            .namespace_registry
            .find("std.foundation")
            .expect("std.foundation namespace");
        let portable_contract = contract
            .split(":capability-protocols")
            .next()
            .expect("portable protocol contract");
        let mut portable_protocol_count = 0usize;
        let mut portable_method_count = 0usize;
        let mut capability_protocol_count = 0usize;
        let mut capability_method_count = 0usize;
        for declaration in core::protocol_declarations() {
            let name = declaration.name;
            let in_contract = portable_contract.contains(&format!(":name {name}"));
            match declaration.availability {
                crate::lang::protocol::ProtocolAvailability::Portable if in_contract => {
                    portable_protocol_count += 1;
                    portable_method_count += declaration.methods.len();
                }
                crate::lang::protocol::ProtocolAvailability::CapabilityGated if !in_contract => {
                    capability_protocol_count += 1;
                    capability_method_count += declaration.methods.len();
                }
                availability => {
                    panic!("unexpected protocol availability for {name}: {availability:?}");
                }
            }
            let namespace_name = declaration.runtime_name();
            let namespace = runtime
                .namespace_registry
                .find(&namespace_name)
                .unwrap_or_else(|| panic!("missing {namespace_name} namespace"));
            let protocol = namespace
                .resolve(&lang::data::Symbol::parse(name))
                .unwrap_or_else(|| panic!("missing {namespace_name}/{name}"))
                .deref_value();
            let core::Value::Protocol(descriptor) = &protocol else {
                panic!("{namespace_name}/{name} is not a protocol");
            };
            assert_eq!(descriptor.name, declaration.runtime_name());
            assert_eq!(descriptor.methods.len(), declaration.methods.len());
            assert!(descriptor
                .methods
                .keys()
                .all(|method| !method.ends_with('!')));
            let canonical_protocol = namespace
                .resolve(&lang::data::Symbol::parse(name))
                .expect("canonical protocol Var");
            let foundation_protocol = foundation
                .resolve(&lang::data::Symbol::parse(name))
                .unwrap_or_else(|| panic!("std.foundation/{name} must expose the annotated protocol"));
            assert!(canonical_protocol.same_identity(&foundation_protocol));
            for method in declaration.methods {
                let canonical_method_var = namespace
                    .resolve(&lang::data::Symbol::parse(method.name))
                    .unwrap_or_else(|| panic!("missing {namespace_name}/{}", method.name));
                assert_eq!(
                    runtime
                        .namespace_registry
                        .resolve(&lang::data::Symbol::parse(&format!(
                            "{namespace_name}/{}",
                            method.name
                        )))
                        .unwrap_or_else(|| panic!("missing {namespace_name}/{}", method.name))
                        .same_identity(&canonical_method_var),
                    true
                );
                assert!(
                    foundation
                        .resolve(&lang::data::Symbol::parse(&format!(
                            "{name}/{}",
                            method.name
                        )))
                        .is_none(),
                    "std.foundation/{name}/{} must not be a protocol alias",
                    method.name
                );
                assert!(
                    runtime
                        .namespace_registry
                        .resolve(&lang::data::Symbol::parse(&format!(
                            "std.protocol.{}/{}",
                            declaration.name.to_ascii_lowercase(),
                            method.name
                        )))
                        .is_none(),
                    "legacy protocol method path must not be intrinsic"
                );
                if in_contract {
                    assert!(
                        fixture.contains(&format!("({namespace_name}/{} fixture", method.name)),
                        "shared fixture does not directly call {namespace_name}/{}",
                        method.name
                    );
                }
            }
        }
        assert_eq!(portable_protocol_count, 61);
        assert_eq!(portable_method_count, 109);
        assert_eq!(capability_protocol_count, 15);
        assert_eq!(capability_method_count, 20);
        assert!(contract.contains(":capability-specific-protocol-count 15"));
        assert!(contract.contains(":capability-specific 20"));
        for protocol in [
            "IHasRuntime",
            "IRanged",
            "IValidate",
            "IComponentOptions",
            "IComponentProps",
            "IComponentQuery",
            "IComponentTrack",
        ] {
            assert!(
                crate::lang::protocol::find_protocol(protocol).is_none(),
                "retired protocol {protocol} must not remain in the annotation registry"
            );
            assert!(
                runtime
                    .eval_text(&format!("std.protocol.{}.{}/{}", protocol.to_ascii_lowercase(), protocol, protocol))
                    .unwrap_err()
                    .contains("unbound symbol"),
                "retired protocol {protocol} must not be guest-visible"
            );
            assert!(
                runtime
                    .eval_text(&format!("std.foundation/{protocol}"))
                    .unwrap_err()
                    .contains("unbound symbol"),
                "std.foundation/{protocol} must not be guest-visible"
            );
        }
        assert_eq!(
            runtime
                .eval_text("(std.protocol.icount.ICount/count [1 2 3])")
                .unwrap(),
            "3"
        );
        assert_eq!(
            runtime
                .eval_text("(std.protocol.icas.ICas/cas (atom 1) 1 2)")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(std.protocol.ireduce.IReduce/reduce \
                       [1 2 3] (fn [left right] (+ left right)) 0)",
                )
                .unwrap(),
            "6"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(std.protocol.ipromise.IPromise/state (std.foundation.promise/from 7))",
                )
                .unwrap(),
            ":fulfilled"
        );
        assert_eq!(
            runtime
                .eval_text("(require [std.protocol.ifind.IFind]) :loaded")
                .unwrap(),
            ":loaded"
        );
        assert_eq!(
            runtime
                .eval_text("(defprotocol PredicateProtocol (ready? [self]))")
                .unwrap(),
            "#protocol[user/PredicateProtocol]"
        );
    }

    #[test]
    fn core_runtime_publishes_foundation_protocol_aliases() {
        let runtime = Runtime::core();
        let foundation = runtime
            .namespace_registry
            .find("std.foundation")
            .expect("core runtime Foundation namespace");

        for name in ["IAssoc", "IColl", "IEncodeVisitor", "IMetadata"] {
            let canonical = runtime
                .namespace_registry
                .find(&format!("std.protocol.{0}.{1}", name.to_ascii_lowercase(), name))
                .and_then(|namespace| namespace.resolve(&lang::data::Symbol::parse(name)))
                .unwrap_or_else(|| panic!("missing canonical protocol {name}"));
            let alias = foundation
                .resolve(&lang::data::Symbol::parse(name))
                .unwrap_or_else(|| panic!("missing std.foundation/{name}"));
            assert!(canonical.same_identity(&alias));
        }
    }

    #[test]
    fn named_declaration_registration_rolls_back_type_and_protocol_state() {
        let mut runtime = Runtime::new();
        let error = runtime
            .eval_text(
                "(defstruct Atomic [value] ICount (count [self extra] (:value self)))",
            )
            .unwrap_err();
        assert!(error.contains("invalid protocol method implementation"), "{error}");
        for name in ["Atomic", "->Atomic", "map->Atomic"] {
            assert!(
                runtime.eval_text(name).is_err(),
                "failed declaration must not publish {name}"
            );
        }
        assert!(runtime.eval_text("ICount/count (Atomic 1)").is_err());
    }

    #[test]
    fn bytecode_named_declaration_registration_rolls_back_type_and_protocol_state() {
        let mut runtime = Runtime::new();
        let program = runtime
            .compile_bytecode(
                "(defstruct Atomic [value]) \
                 (extend-type Atomic ICount (count [self extra] (:value self)))",
            )
            .expect("bytecode declaration must compile before registration");
        let error = runtime
            .execute_compiled_bytecode_value(program)
            .unwrap_err();
        assert!(error.contains("invalid protocol method implementation"), "{error}");
        for name in ["Atomic", "->Atomic", "map->Atomic"] {
            assert!(
                runtime
                    .namespace_registry
                    .resolve(&lang::data::Symbol::parse(name))
                    .is_none(),
                "failed bytecode declaration must not publish {name}"
            );
        }
    }

    #[test]
    fn annotated_protocol_manifest_matches_the_specs_registry() {
        let Some(source) = repo_text("01-lang/001-language/draft/conformance/protocols.edn")
        else {
            return;
        };
        let Form::Map(root) = kernel::parse_forms(&source).unwrap().remove(0) else {
            panic!("protocol contract must be a map")
        };
        let mut expected = Vec::new();
        for (section, availability, capability) in [
            ("protocols", "portable", ""),
            (
                "capability-protocols",
                "capability-gated",
                "native-runtime-protocols",
            ),
        ] {
            let Form::Vector(protocols) = conformance_entry(&root, section) else {
                panic!(":{section} must be a vector")
            };
            for protocol in protocols {
                let Form::Map(protocol) = protocol else {
                    panic!("protocol entries must be maps")
                };
                let Form::Symbol(name) = conformance_entry(protocol, "name") else {
                    panic!("protocol :name must be a symbol")
                };
                let parents = protocol
                    .iter()
                    .find_map(|(key, value)| {
                        matches!(key, Form::Keyword(key) if key == "extends").then_some(value)
                    })
                    .map(|value| match value {
                        Form::Vector(parents) => parents
                            .iter()
                            .map(|parent| match parent {
                                Form::Symbol(parent) => parent.clone(),
                                _ => panic!("protocol parents must be symbols"),
                            })
                            .collect::<Vec<_>>(),
                        _ => panic!("protocol :extends must be a vector"),
                    })
                    .unwrap_or_default();
                let Form::Map(methods) = conformance_entry(protocol, "methods") else {
                    panic!("protocol :methods must be a map")
                };
                let namespace = format!(
                    "std.protocol.{}.{}",
                    name.to_ascii_lowercase(),
                    name
                );
                let mut parents = parents;
                parents.sort();
                let mut methods = methods
                    .iter()
                    .map(|(method, arity)| {
                        let Form::Symbol(method) = method else {
                            panic!("protocol method names must be symbols")
                        };
                        let Form::Number(arity) = arity else {
                            panic!("protocol method arities must be numbers")
                        };
                        format!("{namespace}/{method}:{arity}")
                    })
                    .collect::<Vec<_>>();
                methods.sort();
                expected.push(format!(
                    "protocol|{namespace}|{name}|{availability}|{capability}|annotation|{}|{}",
                    parents.join(","),
                    methods.join(",")
                ));
            }
        }
        expected.sort();
        let actual = core::protocol_manifest();
        assert_eq!(expected, actual);
        assert!(
            actual
                .iter()
                .any(|line| line.starts_with("protocol|std.protocol.icoll.IColl|")),
            "IColl must be present in the annotated protocol manifest"
        );
        assert!(
            actual
                .iter()
                .any(|line| line.starts_with("protocol|std.protocol.imetadata.IMetadata|")),
            "IMetadata must be present in the annotated protocol manifest"
        );
    }

    #[test]
    fn external_protocol_method_names_allow_bangs() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(defprotocol MutatingProtocol (mutate! [self]))")
                .unwrap(),
            "#protocol[user/MutatingProtocol]"
        );
    }

    #[test]
    fn every_native_std_protocol_interface_is_requireable() {
        let mut runtime = Runtime::new();
        for declaration in core::protocol_declarations() {
            let protocol = declaration.name;
            let namespace = declaration.runtime_name();
            runtime
                .eval_text(&format!("(require [{namespace}])"))
                .unwrap_or_else(|error| panic!("cannot require {namespace}: {error}"));
            let loaded = runtime
                .namespace_registry
                .find(&namespace)
                .unwrap_or_else(|| panic!("missing loaded namespace {namespace}"));
            assert!(
                matches!(
                    loaded
                        .resolve(&lang::data::Symbol::parse(protocol))
                        .map(|var| var.deref_value()),
                    Some(core::Value::Protocol(_))
                ),
                "missing {namespace}/{protocol}"
            );
            for method in declaration.methods {
                assert!(
                    loaded
                        .resolve(&lang::data::Symbol::parse(method.name))
                        .is_some(),
                    "missing {namespace}/{}",
                    method.name
                );
            }
        }
    }

    #[test]
    fn namespace_declarations_require_builtin_protocol_interfaces_without_extensions() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns native.protocol-consumer)
                     (boolean IMatch)",
                )
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn specs_owned_foundation_behavioral_corpus_runs_in_the_native_runtime() {
        let Some((modules, corpus)) = foundation_behavior_sources() else {
            return;
        };
        let (surface_namespaces, surface_symbols) =
            foundation_surface().expect("the specs-owned Foundation surface must be available");
        let inventory = std::fs::read_to_string(
            std::path::Path::new(env!("HARA_SOURCE_ROOT")).join("bootstrap.namespaces"),
        )
        .expect("read Foundation bootstrap inventory");
        let registered_namespaces = inventory
            .lines()
            .filter(|line| line.starts_with("std.foundation"))
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            surface_namespaces, registered_namespaces,
            "registered Foundation namespaces must equal the specs-owned namespace surface"
        );
        let evaluator_source = modules.clone() + &corpus;
        let expected_source_symbols = surface_symbols.clone();
        let result = std::thread::Builder::new()
            .name("foundation-conformance".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                let mut runtime = Runtime::new();
                let report = runtime.eval_text(&evaluator_source)?;
                let failures = runtime.eval_text(
                    "{:calibrations (vec (filter (fn [case] (not (:pass case))) (foundation-calibration-results)))
                      :portable (vec (filter (fn [case] (not (:pass case))) (:results (foundation-profile-report))))}",
                )?;
                for qualified in expected_source_symbols {
                    let (namespace, local) = qualified
                        .rsplit_once('/')
                        .unwrap_or_else(|| panic!("qualified Foundation Var: {qualified}"));
                    let var = runtime
                        .namespace_registry
                        .find(namespace)
                        .and_then(|namespace| {
                            namespace.resolve(&lang::data::Symbol::parse(local))
                        })
                        .unwrap_or_else(|| panic!("missing live Foundation Var {qualified}"));
                    assert!(
                        matches!(
                            var.origin(),
                            kernel::VarOrigin::Source | kernel::VarOrigin::HalFallback
                        ),
                        "Foundation Var {qualified} is owned by {:?}, not canonical HAL/HALC/HBX",
                        var.origin()
                    );
                }
                Ok::<_, String>((report, failures))
            })
            .expect("spawn Foundation conformance evaluator")
            .join()
            .expect("Foundation conformance evaluator panicked")
            .unwrap_or_else(|error| panic!("Foundation behavioral corpus: {error}"));
        let (result, failures) = result;
        assert!(
            result.contains(":corpus-valid true"),
            "{result}\n{failures}"
        );
        assert!(
            result.contains(":calibration-failed 0"),
            "{result}\n{failures}"
        );
        assert!(
            result.contains(":boundary-failed 0"),
            "{result}\n{failures}"
        );
        assert!(result.contains(":failed 0"), "{result}\n{failures}");

        let forms = kernel::parse_forms(&result).expect("parse Foundation report");
        let [Form::Map(report)] = forms.as_slice() else {
            panic!("Foundation report must be one map: {result}");
        };
        assert_eq!(
            conformance_entry(report, "surface"),
            conformance_entry(report, "classified"),
            "every specs-owned Foundation Var must have exactly one classification"
        );

        #[cfg(feature = "bytecode-vm")]
        {
            let bytecode = std::thread::Builder::new()
                .name("foundation-bytecode-conformance".into())
                .stack_size(32 * 1024 * 1024)
                .spawn(move || {
                    let mut bytecode_runtime = Runtime::new();
                    bytecode_runtime.eval_text(&modules)?;
                    bytecode_runtime.eval_text(&corpus)?;
                    bytecode_runtime.eval_bytecode_native("(foundation-summary-report)")
                })
                .expect("spawn Foundation bytecode conformance")
                .join()
                .expect("Foundation bytecode conformance panicked")
                .unwrap_or_else(|error| panic!("Foundation bytecode corpus: {error}"));
            let bytecode_forms =
                kernel::parse_forms(&bytecode).expect("parse bytecode Foundation report");
            let [Form::Map(bytecode_report)] = bytecode_forms.as_slice() else {
                panic!("bytecode Foundation report must be one map: {bytecode}");
            };
            assert_eq!(
                report.len(),
                bytecode_report.len(),
                "Foundation report shape"
            );
            for (key, value) in report {
                let Form::Keyword(key) = key else {
                    panic!("Foundation report keys must be keywords: {key:?}");
                };
                assert_eq!(
                    value,
                    conformance_entry(bytecode_report, key),
                    "evaluator/bytecode Foundation report key :{key}"
                );
            }
        }
    }

    #[test]
    fn shared_foundation_protocol_conformance_fixture_runs_in_the_native_runtime() {
        let mut runtime = Runtime::new();
        let source = repo_text(
            "01-lang/001-language/draft/conformance/fixtures/protocol_surface.hal",
        )
        .expect("the specs-owned protocol surface fixture must be available");
        let legacy_protocol_type = regex::Regex::new(r"std\.protocol\.[^\s/]+/I[A-Z]")
            .expect("legacy protocol type path pattern must compile");
        assert!(
            !legacy_protocol_type.is_match(&source),
            "protocol types must resolve unqualified in guest source"
        );
        let result = runtime.eval_text(&source).unwrap();
        assert!(!result.contains(":pass false"), "{result}");
        assert_eq!(result.matches(":pass true").count(), 57, "{result}");

        #[cfg(feature = "bytecode-vm")]
        {
            let mut bytecode_runtime = Runtime::new();
            let bytecode_result = bytecode_runtime.eval_bytecode_native(&source).unwrap();
            assert!(
                !bytecode_result.contains(":pass false"),
                "{bytecode_result}"
            );
            assert_eq!(bytecode_result.matches(":pass true").count(), 57);
        }
    }

    #[test]
    fn shared_foundation_protocol_functionality_fixture_runs_in_the_native_runtime() {
        let source =
            repo_text("01-lang/001-language/draft/conformance/fixtures/protocol_behavioral.hal")
                .expect("the specs-owned behavioral protocol corpus must be available");
        let legacy_protocol_type = regex::Regex::new(r"std\.protocol\.[^\s/]+/I[A-Z]")
            .expect("legacy protocol type path pattern must compile");
        assert!(
            !legacy_protocol_type.is_match(&source),
            "protocol types must resolve unqualified in guest source"
        );
        let catalog = repo_text("01-lang/001-language/draft/conformance/protocol-method-cases.edn")
            .expect("the protocol case catalog must accompany its behavioral corpus");
        let protocols = repo_text("01-lang/001-language/draft/conformance/protocols.edn")
            .expect("protocol contract must accompany its case catalog");
        assert_eq!(
            protocol_case_surface(&catalog),
            protocol_method_surface(&protocols),
            "behavioral protocol cases must exactly close the authoritative method surface"
        );
        assert_eq!(protocols.matches(" -1").count(), 6);
        assert_eq!(catalog.matches(":case :declared-variadic").count(), 6);
        let expected_cases = catalog
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("{:protocol ") || line.starts_with("[{:protocol ")
            })
            .count();
        let expected_failures = catalog.matches(":case :unsupported-receiver").count();
        assert!(expected_cases > 0, "protocol method catalog is empty");
        let mut runtime = Runtime::new();
        let result = runtime.eval_text(&source).unwrap();
        assert!(!result.contains(":pass false"), "{result}");
        assert_eq!(result.matches(":pass true").count(), 109, "{result}");
        let capability_result = runtime.eval_text("(capability-protocol-results)").unwrap();
        assert!(
            !capability_result.contains(":pass false"),
            "{capability_result}"
        );
        assert_eq!(capability_result.matches(":pass true").count(), 20);
        let receiver_matrix = runtime
            .eval_text("(protocol-receiver-matrix-results)")
            .unwrap();
        assert!(
            !receiver_matrix.contains(":pass false"),
            "{receiver_matrix}"
        );
        assert_eq!(receiver_matrix.matches(":pass true").count(), 10);
        let cross_cutting = runtime
            .eval_text("(protocol-cross-cutting-results)")
            .unwrap();
        assert!(!cross_cutting.contains(":pass false"), "{cross_cutting}");
        assert_eq!(cross_cutting.matches(":pass true").count(), 6);
        let capability_receivers = runtime
            .eval_text("(protocol-capability-receiver-results)")
            .unwrap();
        assert!(
            !capability_receivers.contains(":pass false"),
            "{capability_receivers}"
        );
        assert_eq!(capability_receivers.matches(":pass true").count(), 8);
        let native_values = runtime
            .eval_text("(protocol-native-value-results)")
            .unwrap();
        assert!(!native_values.contains(":pass false"), "{native_values}");
        assert_eq!(native_values.matches(":pass true").count(), 15);
        let predicates = runtime.eval_text("(protocol-predicate-results)").unwrap();
        assert!(!predicates.contains(":pass false"), "{predicates}");
        assert_eq!(predicates.matches(":pass true").count(), 7);

        #[cfg(feature = "bytecode-vm")]
        let mut bytecode_runtime = Runtime::new();
        #[cfg(feature = "bytecode-vm")]
        let bytecode_result = bytecode_runtime.eval_bytecode_native(&source).unwrap();
        #[cfg(feature = "bytecode-vm")]
        {
            assert!(
                !bytecode_result.contains(":pass false"),
                "compiled protocol corpus contains failures: {bytecode_result}"
            );
            assert_eq!(
                bytecode_result.matches(":pass true").count(),
                109,
                "compiled protocol corpus did not close the authoritative method surface: {bytecode_result}"
            );
            let bytecode_capability_result = bytecode_runtime
                .eval_bytecode_native("(capability-protocol-results)")
                .unwrap();
            assert!(
                !bytecode_capability_result.contains(":pass false"),
                "compiled capability protocol corpus contains failures: {bytecode_capability_result}"
            );
            assert_eq!(bytecode_capability_result.matches(":pass true").count(), 20);
            let bytecode_receiver_matrix = bytecode_runtime
                .eval_bytecode_native("(protocol-receiver-matrix-results)")
                .unwrap();
            assert!(
                !bytecode_receiver_matrix.contains(":pass false"),
                "compiled receiver matrix contains failures: {bytecode_receiver_matrix}"
            );
            assert_eq!(bytecode_receiver_matrix.matches(":pass true").count(), 10);
            let bytecode_cross_cutting = bytecode_runtime
                .eval_bytecode_native("(protocol-cross-cutting-results)")
                .unwrap();
            assert!(
                !bytecode_cross_cutting.contains(":pass false"),
                "compiled cross-cutting matrix contains failures: {bytecode_cross_cutting}"
            );
            assert_eq!(bytecode_cross_cutting.matches(":pass true").count(), 6);
            let bytecode_capability_receivers = bytecode_runtime
                .eval_bytecode_native("(protocol-capability-receiver-results)")
                .unwrap();
            assert!(
                !bytecode_capability_receivers.contains(":pass false"),
                "compiled capability receiver matrix contains failures: {bytecode_capability_receivers}"
            );
            assert_eq!(
                bytecode_capability_receivers.matches(":pass true").count(),
                8
            );
            let bytecode_native_values = bytecode_runtime
                .eval_bytecode_native("(protocol-native-value-results)")
                .unwrap();
            assert!(
                !bytecode_native_values.contains(":pass false"),
                "compiled native-value matrix contains failures: {bytecode_native_values}"
            );
            assert_eq!(bytecode_native_values.matches(":pass true").count(), 15);
            let bytecode_predicates = bytecode_runtime
                .eval_bytecode_native("(protocol-predicate-results)")
                .unwrap();
            assert!(
                !bytecode_predicates.contains(":pass false"),
                "compiled protocol predicate matrix contains failures: {bytecode_predicates}"
            );
            assert_eq!(bytecode_predicates.matches(":pass true").count(), 7);
        }

        let method_vars = source
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let start = line.find("(protocol-case ")?;
                line[(start + "(protocol-case ".len())..]
                    .split_whitespace()
                    .nth(2)
            })
            .collect::<Vec<_>>();
        assert_eq!(method_vars.len(), expected_cases);
        for method_var in method_vars {
            let mut segments = method_var.split(['.', '/']);
            let protocol_namespace = segments.nth(2).expect("protocol namespace");
            let _protocol = segments.next().expect("protocol type");
            let method = segments.next().expect("protocol method");
            assert!(
                catalog.contains(&format!(":method {method} ")),
                "case catalog is missing {protocol_namespace}/{method}"
            );
            let error = runtime.eval_text(&format!("({method_var})")).unwrap_err();
            assert!(
                error.contains("protocol/arity"),
                "{method_var} returned an uncategorized arity error: {error}"
            );
            #[cfg(feature = "bytecode-vm")]
            {
                let bytecode_error = bytecode_runtime
                    .eval_bytecode_native(&format!("({method_var})"))
                    .unwrap_err();
                assert!(
                    bytecode_error.contains("protocol/arity"),
                    "compiled {method_var} returned an uncategorized arity error: {bytecode_error}"
                );
            }
        }

        let failure_forms = source
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let start = line.find("'(std.protocol.")? + 1;
                let form = &line[start..];
                let mut depth = 0_usize;
                for (index, character) in form.char_indices() {
                    match character {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(form[..=index].to_owned());
                            }
                        }
                        _ => {}
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        assert_eq!(failure_forms.len(), expected_failures);
        for failure_form in failure_forms {
            let error = runtime.eval_text(&failure_form).unwrap_err();
            assert!(
                error.contains("protocol/unsupported-receiver"),
                "{failure_form} returned an uncategorized dispatch error: {error}"
            );
            #[cfg(feature = "bytecode-vm")]
            {
                let bytecode_error = bytecode_runtime
                    .eval_bytecode_native(&failure_form)
                    .unwrap_err();
                assert!(
                    bytecode_error.contains("protocol/unsupported-receiver"),
                    "compiled {failure_form} returned an uncategorized dispatch error: {bytecode_error}"
                );
            }
        }

        assert_eq!(
            runtime
                .eval_text(
                    "(try (std.protocol.icount.ICount/count) false \
                       (catch Throwable error true))"
                )
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn shared_protocol_conformance_fixture_runs_in_the_native_runtime() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(include_str!(
                    "../../../lib/test-fixtures/std/substrate/protocol_conformance.hal"
                ))
                .unwrap(),
            "[40 42]"
        );
    }

    #[test]
    fn shared_substrate_frame_conformance_fixture_runs_in_the_native_runtime() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(include_str!(
                    "../../../lib/test-fixtures/std/substrate/frame_conformance.hal"
                ))
                .unwrap(),
            "[\"substrate.v1\" \"request\" \"req-1\" \"client/a\" \"server/b\" \"workspace/main\" {\"trace\" \"trace-1\"} \"math/add\" [19 23] nil nil nil nil nil nil]"
        );
        assert!(runtime
            .eval_text(
                "(do (require 'std.substrate.json) \\
                     (std.substrate.json/decode-frame {:kind :unknown :id \"evt-1\"}))",
            )
            .is_err());
    }

    #[test]
    fn shared_substrate_node_lifecycle_fixture_runs_in_the_native_runtime() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = development_runtime();
                assert_eq!(
                    runtime
                        .eval_text(include_str!(
                            "../../../lib/test-fixtures/std/substrate/node_lifecycle_conformance.hal"
                        ))
                        .unwrap(),
                    "[84 42 :rejected]"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn atom_backed_substrate_capabilities_work_in_the_native_runtime() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(
                    "(require 'std.substrate) \
                     (def node (std.substrate/node-create \"node-1\")) \
                     [(std.substrate.protocol/set-service node \"cache\" 42) \
                      (std.substrate.protocol/get-service node \"cache\") \
                      (do (std.substrate.protocol/create-space node \"main\" {}) nil) \
                      (std.substrate.protocol/set-space-state node \"main\" {:count 1}) \
                      (std.substrate.protocol/get-space-state node \"main\") ]",
                )
                .unwrap(),
            "[42 42 nil {:count 1} {:count 1}]"
        );
    }

    #[test]
    fn substrate_routes_streams_and_settles_transport_requests() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(
                    "(require 'std.substrate) \
                     (def node (std.substrate/node-create \"node-1\")) \
                     (std.substrate.protocol/attach-transport node \"peer-a\" \
                       (fn [frame] \
                         (std.substrate.protocol/set-service node \"sent\" \
                           (std.substrate/frame-data frame)))) \
                     (def subscription (promise/value (std.substrate.protocol/subscribe node \"main\" \"changed\" \"sub-1\" {}))) \
                     (promise/value (std.substrate.protocol/receive-frame node subscription {:transport-id \"peer-a\"})) \
                     (promise/value (std.substrate.protocol/publish node \"main\" \"changed\" 42 {:id \"evt-1\"})) \
                     (std.substrate.protocol/get-service node \"sent\")",
                )
                .unwrap(),
            "42"
        );

        assert_eq!(
            runtime
                .eval_text(
                    "(def requester (std.substrate/node-create \"node-2\")) \
                     (std.substrate.protocol/attach-transport requester \"peer-b\" \
                       (fn [frame] \
                         (std.substrate.protocol/receive-frame requester \
                           (std.substrate/node-frame :response \"res-1\" \"main\" {} nil [] \
                             (std.substrate/frame-id frame) :ok 84 nil nil nil) \
                           {:transport-id \"peer-b\"}))) \
                     (def reply (std.substrate.protocol/request requester \"main\" \"sum\" [] \
                                  {:id \"req-1\" :transport-id \"peer-b\"})) \
                     (promise/value reply)",
                )
                .unwrap(),
            "84"
        );
    }

    #[test]
    fn substrate_cancellation_and_rejection_settle_pending_promises() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(
                    "(require 'std.substrate) \
                     (def node (std.substrate/node-create \"node-1\")) \
                     (std.substrate.protocol/attach-transport node \"peer-a\" (fn [frame] nil)) \
                     (def cancelled (std.substrate.protocol/request node \"main\" \"wait\" [] \
                                      {:id \"req-cancel\" :transport-id \"peer-a\"})) \
                     (std.substrate.protocol/cancel-request node \"req-cancel\" :cancelled) \
                     (promise/state cancelled)",
                )
                .unwrap(),
            ":rejected"
        );
    }

    #[test]
    fn registered_resources_load_into_the_runtime_environment() {
        let mut runtime = Runtime::new();
        runtime.register_resource("demo", "(defn answer [] 42) (answer)");
        assert_eq!(runtime.load_resource("demo").unwrap(), "42");
        assert_eq!(runtime.eval_text("(answer)").unwrap(), "42");
        assert_eq!(runtime.require_resource("demo").unwrap(), "42");
        assert_eq!(runtime.require_resource("demo").unwrap(), ":loaded");
    }

    #[test]
    fn set_global_imports_use_terminal_names_and_compact_protocols() {
        let mut runtime = Runtime::core();
        runtime
            .eval_text(
                "(ns demo.global (:config {:set-global [demo.global/value]})) \
                 (def value 42)",
            )
            .unwrap();
        assert_eq!(runtime.eval_text("(ns demo.consumer) value").unwrap(), "42");

        runtime
            .eval_text(
                "(ns demo.protocol (:config {:set-global [IColl/start-string IMetadata/metatype]})) \
                 (start-string [])",
            )
            .unwrap();
        assert_eq!(runtime.eval_text("(start-string [1 2])").unwrap(), "\"[\"");
        assert_eq!(runtime.eval_text("(metatype {:value 1})").unwrap(), ":map");
    }

    #[test]
    fn foundation_resource_paths_load_root_before_children_and_rollback() {
        let mut runtime = Runtime::core();
        runtime.register_resource(
            "std/foundation.hal",
            "(ns std.foundation) (defn foundation-marker [] 41)",
        );
        runtime.register_resource(
            "std/foundation/child.hal",
            "(ns std.foundation.child) (defn child-marker [] (foundation-marker))",
        );

        assert_eq!(
            runtime.require_resource("std/foundation/child.hal").unwrap(),
            "41"
        );
        assert!(runtime.loaded_resources.contains("std.foundation"));
        assert!(runtime.loaded_resources.contains("std.foundation.child"));
        assert_eq!(
            runtime
                .eval_text("(std.foundation.child/child-marker)")
                .unwrap(),
            "41"
        );

        #[cfg(target_arch = "wasm32")]
        {
            let mut broken = Runtime::core();
            broken.register_resource(
                "std/foundation/broken.hal",
                "(ns std.foundation.broken) (def broken-marker missing-symbol)",
            );
            assert!(broken.require_resource("std/foundation/broken.hal").is_err());
            assert!(!broken.loaded_resources.contains("std.foundation"));
            assert!(!broken.loaded_resources.contains("std.foundation.broken"));
        }
    }

    #[test]
    fn vector_literals_are_values() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("[1 2 3]").unwrap(), "[1 2 3]");
    }

    #[test]
    fn set_literals_reject_duplicate_items() {
        let mut runtime = Runtime::new();
        assert!(runtime
            .eval_text("#{1 (+ 1 1) 1}")
            .unwrap_err()
            .contains("Duplicate item"));
        assert!(runtime
            .eval_text("(count #{1 2 2})")
            .unwrap_err()
            .contains("Duplicate item"));
        assert_eq!(runtime.eval_text("(has? #{1 2} 2)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(conj #{1} 2)").unwrap(), "#{1 2}");
        assert_eq!(
            runtime.eval_text("(= (set [1 2 1]) #{1 2})").unwrap(),
            "true"
        );
        assert!(runtime.eval_text("(set 1 2)").is_err());
        assert_eq!(runtime.eval_text("(= #{1 2} #{2 1})").unwrap(), "true");
        assert_eq!(runtime.eval_text("(get #{1 2} 2 :missing)").unwrap(), "2");
    }

    #[test]
    fn syntax_quote_matches_java_expansion_semantics() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("`foo").unwrap(), "foo");
        assert_eq!(
            runtime.eval_text("`(a ~(+ 1 2) ~@[4 5])").unwrap(),
            "(a 3 4 5)"
        );
        assert_eq!(runtime.eval_text("`[a ~(+ 1 2)]").unwrap(), "[a 3]");
        assert_eq!(runtime.eval_text("`{:a ~(+ 1 2)}").unwrap(), "{:a 3}");
        assert_eq!(
            runtime.eval_text("`(a (unquote))").unwrap_err(),
            "unquote expects one argument"
        );
        assert_eq!(
            runtime.eval_text("`(a ~@1)").unwrap_err(),
            "iter expects a collection, got 1"
        );
    }

    #[test]
    fn deref_of_a_global_atom_targets_the_atom_value_not_its_namespace_var() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(do (def state (atom [1])) (deref state))")
                .unwrap(),
            "[1]"
        );
        assert_eq!(
            runtime
                .eval_text("(do (swap! state conj 2) (deref state))")
                .unwrap(),
            "[1 2]"
        );
    }

    #[test]
    fn fn_forms_and_eval_forms_execute_while_hash_dispatch_extensions_are_rejected() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("((fn [x] (+ x 1)) 4)").unwrap(), "5");
        assert!(runtime.eval_text("((fn* [x] (+ x 1)) 4)").is_err());
        assert!(runtime
            .eval_text("#=(+ 2 3)")
            .unwrap_err()
            .contains("No dispatch macro for: ="));
        assert!(runtime
            .eval_text("#[(def x 4) (+ x 2)]")
            .unwrap_err()
            .contains("No dispatch macro for: ["));
        assert!(runtime
            .eval_text("(eval)")
            .unwrap_err()
            .contains("one form"));
    }

    #[test]
    fn runtime_readable_strings_escape_and_round_trip() {
        let mut runtime = Runtime::new();
        let sources = [
            r#""quote: \" slash: \\ newline: \n tab: \t""#,
            r#"{:text "line\nvalue" :nested ["a\tb" "c\\d"]}"#,
            r#"["\u0000" "unicode λ"]"#,
            r#"#"a\"b""#,
        ];
        for source in sources {
            let readable = runtime.eval_text(source).unwrap();
            assert_eq!(
                kernel::parse(&readable).unwrap(),
                kernel::parse(source).unwrap()
            );
        }
    }

    #[test]
    fn reader_literals_are_first_class_runtime_values() {
        let mut runtime = Runtime::new();
        let cases = [
            ("1.5", "(double 1.5)"),
            ("\\newline", "\\newline"),
            ("#\"a+\"", "#\"a+\""),
            ("#demo {:a 1}", "#demo{:a 1}"),
        ];
        for (source, expected) in cases {
            assert_eq!(runtime.eval_text(source).unwrap(), expected, "{source}");
        }
        for source in ["123N", "1.20M"] {
            assert!(runtime.eval_text(source).is_err(), "{source}");
        }
        assert_eq!(
            runtime.eval_text("9223372036854775808").unwrap(),
            "9223372036854775808"
        );
        for source in ["##Inf", "##-Inf", "##NaN", "1e309", "-1e309"] {
            assert!(
                runtime
                    .eval_text(source)
                    .unwrap_err()
                    .contains("non-finite number"),
                "{source}"
            );
        }
        assert_eq!(runtime.eval_text("'#demo [1 2]").unwrap(), "#demo[1 2]");
        assert_eq!(runtime.eval_text("()").unwrap(), "()");
        assert_eq!(runtime.eval_text("(list? ())").unwrap(), "true");
        assert_eq!(runtime.eval_text("(char? \\x)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(char? \"x\")").unwrap(), "false");
        assert_eq!(runtime.eval_text("(nth [1 nil 3] 1)").unwrap(), "nil");
        assert_eq!(runtime.eval_text("(nth '(1 nil 3) 1)").unwrap(), "nil");
    }

    #[test]
    fn basic_math_has_the_portable_root_surface_and_explicit_numeric_boundary() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(= E 2.718281828459045) (= PI 3.141592653589793) \
                     (sin 0) (cos 0) (tan 0) (asin 0) (acos 1) (atan 0) \
                     (atan2 0 1) (sinh 0) (cosh 0) (tanh 0) \
                     (asinh 0) (acosh 1) (atanh 0) \
                     (floor 1.75) (ceil 1.25) (pow 2 3) (abs -3) \
                     (exp 0) (sqrt 9)]"
                )
                .unwrap(),
            "[true true (double 0) (double 1) (double 0) (double 0) (double 0) (double 0) (double 0) (double 0) (double 1) (double 0) (double 0) (double 0) (double 0) (double 1) (double 2) (double 8) 3 (double 1) (double 3)]"
        );
        for source in [
            "(sqrt -1)",
            "(exp 10000)",
            "(* 1.0e308 1.0e308)",
            "(parse-double \"Infinity\")",
        ] {
            assert!(
                runtime
                    .eval_text(source)
                    .unwrap_err()
                    .contains("non-finite number"),
                "{source}"
            );
        }
        assert_eq!(runtime.eval_text("(sqrt (long 9))").unwrap(), "(double 3)");
        assert_eq!(runtime.eval_text("(long 9.9)").unwrap(), "9");
        assert_eq!(
            runtime.eval_text("(sqrt (double 9))").unwrap(),
            "(double 3)"
        );
        assert_eq!(
            runtime.eval_text("(abs -9223372036854775808)").unwrap(),
            "9223372036854775808"
        );
        assert!(runtime.eval_text("(asinh 1.0e300)").is_ok());
        assert!(runtime.eval_text("(acosh 1.0e300)").is_ok());
        for source in ["(sin)", "(pow 2)", "(sqrt \"9\")"] {
            assert!(runtime.eval_text(source).is_err(), "{source}");
        }
    }

    #[test]
    fn base_uuid_has_portable_identity_and_string_construction() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(let [random (Base/uuid) \
                           fixed (Base/uuid \"00000000-0000-0000-0000-000000000000\") \
                           byte-uuid (Base/uuid (std.native.Bytes/new 1 2 -1)) \
                           keyword (Base/uuid :demo/value) \
                           bits (Base/uuid 0 1)] \
                       [(uuid? random) (type random) \
                        (std.foundation/uuid? fixed) \
                        (std.foundation/uuid? byte-uuid) \
                        (std.foundation/uuid? keyword) \
                        (std.foundation/uuid? bits) \
                        (std.foundation/uuid? :demo/value) \
                        (= fixed (Base/uuid \"00000000-0000-0000-0000-000000000000\")) \
                        (= byte-uuid (Base/uuid \"4f989b1a-c8e4-3ab1-9569-6571104cfb67\")) \
                        (= keyword (Base/uuid \"00000000-6d44-1e45-0000-000006ac9171\")) \
                        (= bits (Base/uuid \"00000000-0000-0000-0000-000000000001\"))])"
                )
                .unwrap(),
            "[true :std.native.UUID true true true true false true true true true]"
        );
    }

    #[test]
    fn closed_native_method_inventory_is_classified_and_callable() {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> &'a Form {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing :{key}"))
        }
        fn symbols(value: &Form, label: &str) -> Vec<String> {
            let Form::Vector(values) = value else {
                panic!("{label} must be a vector")
            };
            values
                .iter()
                .map(|value| match value {
                    Form::Symbol(name) => name.clone(),
                    _ => panic!("{label} must contain symbols"),
                })
                .collect()
        }
        fn native_capabilities(source: &str) -> std::collections::BTreeMap<String, String> {
            let Form::Map(contract) = kernel::parse_forms(source).unwrap().remove(0) else {
                panic!("native runtime spec must be a map")
            };
            let Form::Vector(types) = entry(&contract, "native/types") else {
                panic!(":native/types must be a vector")
            };
            let mut capabilities = std::collections::BTreeMap::new();
            for value in types {
                let Form::Map(native_type) = value else {
                    panic!("native runtime type entries must be maps")
                };
                let Form::Symbol(name) = entry(native_type, "native/symbol") else {
                    panic!(":native/symbol must be a symbol")
                };
                let capability = native_type.iter().find_map(|(key, value)| {
                    matches!(key, Form::Keyword(key) if key == "native/capability")
                        .then(|| match value {
                            Form::Keyword(capability) => capability.clone(),
                            _ => panic!(":native/capability must be a keyword"),
                        })
                });
                if let Some(capability) = capability {
                    assert!(
                        capabilities.insert(name.clone(), capability).is_none(),
                        "duplicate native runtime type: {name}"
                    );
                }
            }
            capabilities
        }
        fn classified(value: Option<&Form>, all: &[String], label: &str) -> Vec<String> {
            match value {
                None => Vec::new(),
                Some(Form::Keyword(marker)) if marker == "all" => all.to_vec(),
                Some(value) => symbols(value, label),
            }
        }
        fn wrapper_source(path: &str) -> String {
            // The native contract still names the retired std.sandbox wrapper.
            // Its implementation now lives in std.lib.kernel, so resolve that
            // manifest alias here.
            let candidates = match path {
                "lib/src/std/sandbox.hal" => vec!["lib/src/std/lib/kernel.hal", path],
                "lib/src/std/lib/package.hal" => {
                    vec!["lib/src/code/project/package.hal", path]
                }
                _ => vec![path],
            };
            for candidate in candidates {
                if let Some(source) =
                    EMBEDDED_HAL_RESOURCES
                        .iter()
                        .find_map(|(_, resource_path, source)| {
                            (*resource_path == candidate).then_some(*source)
                        })
                {
                    return source.to_owned();
                }
                let local = std::path::Path::new(env!("HARA_SOURCE_ROOT"))
                    .join("..")
                    .join(candidate);
                if let Ok(source) = std::fs::read_to_string(&local) {
                    return source;
                }
            }
            panic!("unknown wrapper source: {path}")
        }

        let Some(contract_source) = repo_text("01-lang/001-language/draft/conformance/native.edn")
        else {
            return;
        };
        let Some(native_spec_source) = repo_text("01-lang/003-native/draft/native-spec.edn") else {
            return;
        };
        let native_capabilities = native_capabilities(&native_spec_source);
        let contract = kernel::parse_forms(&contract_source).unwrap().remove(0);
        let Form::Map(contract) = contract else {
            panic!("native contract must be a map")
        };
        let Form::Map(inventory) = entry(&contract, "inventory") else {
            panic!(":inventory must be a map")
        };
        let Form::Map(language_builtins) = entry(&contract, "language-builtins") else {
            panic!(":language-builtins must be a map")
        };
        let specified_builtins = ["evaluation", "definitions", "namespaces", "interop"]
            .into_iter()
            .map(|category| {
                (
                    category,
                    symbols(entry(language_builtins, category), category),
                )
            })
            .collect::<Vec<_>>();
        let runtime_builtins = core::LANGUAGE_BUILTINS
            .iter()
            .map(|(category, names)| {
                (
                    *category,
                    names.iter().map(|name| (*name).to_owned()).collect(),
                )
            })
            .collect::<Vec<(&str, Vec<String>)>>();
        assert_eq!(specified_builtins, runtime_builtins);
        assert!(matches!(entry(inventory, "closed"), Form::Bool(true)));
        let Form::Vector(types) = entry(&contract, "types") else {
            panic!(":types must be a vector")
        };
        assert!(!types.is_empty(), "native :types must not be empty");
        let Form::Map(source_resolution) = entry(&contract, "source-resolution") else {
            panic!(":source-resolution must be a map")
        };
        assert!(matches!(
            entry(source_resolution, "model"),
            Form::Keyword(value) if value == "builtin-static-object"
        ));
        for field in ["namespace-dependency", "requireable", "aliasable"] {
            assert_eq!(entry(source_resolution, field), &Form::Bool(false));
        }
        let Form::Map(translation) = entry(&contract, "translation-conformance") else {
            panic!(":translation-conformance must be a map")
        };
        assert!(matches!(
            entry(translation, "coverage"),
            Form::Keyword(value) if value == "all-inventory-types"
        ));
        let Form::Vector(requirements) = entry(translation, "requirements") else {
            panic!(":translation-conformance :requirements must be a vector")
        };
        for requirement in [
            "inventory-is-closed",
            "every-type-has-global-object",
            "global-object-is-runtime-qualified-object",
            "native-dependencies-are-rejected",
            "aliased-calls-are-canonicalized",
            "qualified-calls-are-canonicalized",
            "translation-is-idempotent",
        ] {
            assert!(
                requirements
                    .iter()
                    .any(|value| matches!(value, Form::Keyword(name) if name == requirement)),
                "missing native translation requirement: {requirement}"
            );
        }

        let mut specified = Vec::new();
        let mut specified_declarations = Vec::new();
        for value in types {
            let Form::Map(native_type) = value else {
                panic!("native type entries must be maps")
            };
            let Form::Symbol(name) = entry(native_type, "name") else {
                panic!("native :name must be a symbol")
            };
            let methods = symbols(entry(native_type, "methods"), ":methods");
            let Form::Keyword(availability) = entry(native_type, "availability") else {
                panic!("native :availability must be a keyword")
            };
            let capability = native_capabilities.get(name).cloned();
            assert!(
                ["implemented", "capability-gated"].contains(&availability.as_str()),
                "unsupported availability: {availability}"
            );
            let Form::Map(classification) = entry(native_type, "method-classification") else {
                panic!(":method-classification must be a map")
            };
            let hal_wrappers = classified(
                classification.iter().find_map(|(key, value)| {
                    matches!(key, Form::Keyword(name) if name == "hal-wrapper").then_some(value)
                }),
                &methods,
                ":hal-wrapper",
            );
            let primitives = classified(
                classification.iter().find_map(|(key, value)| {
                    matches!(key, Form::Keyword(name) if name == "foundation-primitive")
                        .then_some(value)
                }),
                &methods,
                ":foundation-primitive",
            );
            let native_only = classified(
                classification.iter().find_map(|(key, value)| {
                    matches!(key, Form::Keyword(name) if name == "native-only").then_some(value)
                }),
                &methods,
                ":native-only",
            );
            let mut exposed = hal_wrappers.clone();
            exposed.extend(primitives);
            exposed.extend(native_only);
            assert_eq!(
                exposed
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                methods.len(),
                "{name} methods must have one Foundation exposure"
            );
            assert_eq!(
                methods.iter().collect::<std::collections::HashSet<_>>(),
                exposed.iter().collect::<std::collections::HashSet<_>>(),
                "{name} method classifications are incomplete"
            );
            if !hal_wrappers.is_empty() {
                let Form::String(path) = entry(native_type, "wrapper-source") else {
                    panic!("{name} HAL wrappers require :wrapper-source")
                };
                let source = wrapper_source(path);
                for method in &hal_wrappers {
                    assert!(
                        source.contains(&format!("{name}/{method}")),
                        "missing HAL wrapper for {name}/{method}"
                    );
                }
            }
            specified.push((name.clone(), methods));
            specified_declarations.push((
                name.clone(),
                specified.last().unwrap().1.clone(),
                availability.clone(),
                capability,
            ));
        }

        let runtime_inventory = core::native_declarations()
            .iter()
            .map(|declaration| {
                (
                    declaration.name.to_owned(),
                    declaration
                        .methods
                        .iter()
                        .map(|method| (*method).to_owned())
                        .collect(),
                )
            })
            .collect::<Vec<(String, Vec<String>)>>();
        let unique_types = specified
            .iter()
            .map(|(name, _)| name)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            unique_types.len(),
            specified.len(),
            "duplicate native type names in native.edn"
        );
        for (name, methods) in &specified {
            let unique_methods = methods.iter().collect::<std::collections::HashSet<_>>();
            assert_eq!(
                unique_methods.len(),
                methods.len(),
                "duplicate methods declared for {name} in native.edn"
            );
        }
        assert_eq!(specified, runtime_inventory);
        let runtime_declarations = core::NATIVE_DECLARATIONS
            .iter()
            .map(|declaration| {
                let availability = match declaration.availability {
                    core::NativeAvailability::Portable => "implemented",
                    core::NativeAvailability::CapabilityGated => "capability-gated",
                    core::NativeAvailability::InventoryOnly => "inventory-only",
                };
                (
                    declaration.name.to_owned(),
                    declaration
                        .methods
                        .iter()
                        .map(|method| (*method).to_owned())
                        .collect::<Vec<_>>(),
                    availability.to_owned(),
                    declaration.capability.map(str::to_owned),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            specified_declarations, runtime_declarations,
            "annotated native declaration metadata differs from native.edn"
        );
        assert!(
            !specified.iter().any(|(name, _)| name == "Builtins"),
            "Builtins is accounting only and must not be a native type"
        );
        let Form::Map(startup_visibility) = entry(&contract, "startup-visibility") else {
            panic!(":startup-visibility must be a map")
        };
        let global_aliases = symbols(
            entry(startup_visibility, "global-aliases"),
            ":global-aliases",
        );
        assert_eq!(
            global_aliases
                .iter()
                .collect::<std::collections::HashSet<_>>(),
            specified
                .iter()
                .map(|(name, _)| name)
                .collect::<std::collections::HashSet<_>>()
        );
        assert!(
            !inventory
                .iter()
                .any(|(key, _)| matches!(key, Form::Keyword(name) if name == "method-count" || name == "type-count")),
            "native counts must be derived from :types"
        );

        let corpus = std::fs::read_to_string(crate::spec_registry::require(
            "01-lang/001-language/draft/conformance/fixtures/native_behavioral.hal",
        ))
        .expect("native behavioral corpus is readable");
        let mut runtime = Runtime::new();
        let results = runtime
            .eval_text(&format!("{corpus}\n(native-method-results)"))
            .unwrap();
        assert!(!results.contains(":pass false"), "{results}");
        assert_eq!(
            results.matches(":pass true").count(),
            specified
                .iter()
                .map(|(_, methods)| methods.len())
                .sum::<usize>()
        );
        let mut runtime = Runtime::new();
        for (native_type, _) in &specified {
            let identity_probe = runtime
                .eval_text(&format!(
                    "[{native_type} std.native.{native_type} \
                       (type {native_type}) (type std.native.{native_type})]"
                ))
                .unwrap();
            assert_eq!(
                runtime
                    .eval_text(&format!("(= {native_type} std.native.{native_type})"))
                    .unwrap(),
                "true",
                "global native type object differs for {native_type}: {identity_probe}"
            );
        }
    }

    #[test]
    fn native_types_are_descriptors_and_foundation_libraries_are_hal_wrappers() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(str std.native.Maths) \
                      (INamespaced/name std.native.Maths) \
                      (INamespaced/namespace std.native.Maths) \
                      (= std.native.Maths (with-meta std.native.Maths {:doc \"math\"})) \
                      (Maths/sin 0) \
                      (String/upper \"hara\") \
                      (str/upper \"hara\") \
                      (Bytes/u8 -1) \
                      (bytes/u8 -1)]"
                )
                .unwrap(),
            "[\"#<native-type std.native.Maths>\" \"Maths\" \"std.native\" true (double 0) \"HARA\" \"HARA\" 255 255]"
        );
        assert!(runtime.eval_text("(std.native.Maths 1)").is_err());
    }

    #[test]
    fn duplex_is_portable_and_not_exposed_as_a_native_box() {
        let mut runtime = Runtime::new();
        for source in [
            "Duplex",
            "std.native.Duplex",
            "(Process/duplex nil)",
            "(Socket/duplex nil)",
        ] {
            assert!(
                runtime.eval_text(source).is_err(),
                "{source} must remain absent"
            );
        }
    }

    #[test]
    fn native_test_run_awaits_promise_results() {
        let mut runtime = Runtime::new();
        let output = runtime
            .eval_text(
                "(Test/run [{:name \"async\" \
                              :test (fn [] (promise/delay 1 (fn [] 42))) \
                              :expected 42}])",
            )
            .unwrap();
        assert!(output.contains(":name \"async\""), "{output}");
        assert!(output.contains("#hara/Result[:success true"), "{output}");
        assert!(output.contains(":actual 42"), "{output}");
    }

    #[test]
    fn native_test_run_supports_lifecycle_maps() {
        let mut runtime = Runtime::new();
        let output = runtime
            .eval_text(
                "(let [events (atom [])] \
                   [(Test/run [{:name \"case\" :test (fn [] (swap! events conj :case) 1) :expected 1}] \
                              {:setup (fn [] (swap! events conj :setup)) \
                               :teardown (fn [] (swap! events conj :teardown))}) \
                    @events])",
            )
            .unwrap();
        assert!(output.contains("#hara/Result[:success true"), "{output}");
        assert!(output.contains("[:setup :case :teardown]"), "{output}");

        let failure = runtime
            .eval_text(
                "(let [events (atom [])] \
                   [(Test/run [{:name \"skipped\" :test (fn [] (swap! events conj :case)) :expected nil}] \
                              {:setup (fn [] (throw \"setup boom\")) \
                               :teardown (fn [] (swap! events conj :teardown) (throw \"teardown boom\"))}) \
                    @events])",
            )
            .unwrap();
        assert!(failure.contains(":phase :setup"), "{failure}");
        assert!(failure.contains(":phase :teardown"), "{failure}");
        assert!(failure.contains("[:teardown]"), "{failure}");
        assert!(!failure.contains(":name \"skipped\""), "{failure}");
    }

    #[test]
    fn removed_builtins_config_is_rejected_by_runtime() {
        let mut runtime = Runtime::new();
        assert!(runtime
            .eval_text("(ns legacy.activation (:config {:builtins [inc]}))")
            .unwrap_err()
            .contains("Unsupported :config option: :builtins"));
    }

    #[test]
    fn namespace_roles_are_parsed_retained_and_redeclared() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns role.standard) \
                     (ns role.internal (:config {:role :internal})) \
                     (ns role.facade (:config {:role :facade})) \
                     [(get (Runtime/namespace 'role.standard) :namespace/role) \
                      (get (Runtime/namespace 'role.internal) :namespace/role) \
                      (get (Runtime/namespace 'role.facade) :namespace/role)]",
                )
                .unwrap(),
            "[:standard :internal :facade]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(ns role.internal) \
                     (get (Runtime/namespace 'role.internal) :namespace/role)",
                )
                .unwrap(),
            ":standard"
        );
        assert!(runtime
            .eval_text("(ns role.invalid (:config {:role :unsupported}))")
            .unwrap_err()
            .contains(":config :role expects :default, :internal, or :facade"));
    }

    #[test]
    fn startup_defaults_expose_native_types_and_protocols() {
        let runtime = Runtime::new();
        let symbols = runtime.visible_symbols();
        assert!(symbols.iter().any(|symbol| symbol == "Edn/pretty"));
        for native_type in [
            "Maths",
            "Num",
            "Bits",
            "String",
            "Bytes",
            "Crypto",
            "OS",
            "Process",
            "File",
            "Socket",
            "Promise",
            "Coroutine",
            "Stream",
            "Arr",
            "Obj",
            "Runtime",
            "Printer",
            "Document",
            "Edn",
            "Json",
            "Host",
            "Test",
            "RegExp",
            "Result",
            "Schema",
            "Exception",
            "Base",
            "Algo",
            "Iter",
            "Kernel",
            "Package",
        ] {
            assert!(
                symbols.iter().any(|symbol| symbol == native_type),
                "{native_type}"
            );
        }
    }

    #[test]
    fn startup_completion_omits_canonical_runtime_bindings() {
        let runtime = Runtime::new();
        let symbols = runtime.visible_symbols();

        assert!(symbols.iter().any(|symbol| symbol == "co/create"));
        assert!(symbols.iter().any(|symbol| symbol == "co/resume"));
        assert!(!symbols.iter().any(|symbol| {
            symbol.starts_with("std.native.")
                || symbol.starts_with("std.protocol.")
                || symbol.starts_with("co/std.native.")
                || symbol.starts_with("co/std.protocol.")
        }));
    }

    #[test]
    fn strings_and_maps_are_values() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("\"hello\"").unwrap(), "\"hello\"");
        assert_eq!(runtime.eval_text("{\"a\" 1}").unwrap(), "{\"a\" 1}");
        assert_eq!(runtime.eval_text("(str nil \"x\" nil)").unwrap(), "\"x\"");
        assert_eq!(runtime.eval_text("(first \"ab\")").unwrap(), "\\a");
        assert_eq!(runtime.eval_text("(seq \"ab\")").unwrap(), "(\\a \\b)");
        assert_eq!(
            runtime
                .eval_text("(sequential? (map identity [1]))")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime.eval_text("(str/split-lines \"a\\nb\")").unwrap(),
            "[\"a\" \"b\"]"
        );
    }

    #[test]
    fn application_and_pair_helpers_support_bootstrap_code() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(or false false)").unwrap(), "false");
        assert_eq!(
            runtime
                .eval_text("(Printer/capture (fn [] (p \"a\") (println \"b\")))")
                .unwrap(),
            "\"ab\\n\""
        );
        assert_eq!(runtime.eval_text("(identity 42)").unwrap(), "42");
        assert_eq!(runtime.eval_text("(apply + [19 23])").unwrap(), "42");
        assert_eq!(runtime.eval_text("(apply + 19 [23])").unwrap(), "42");
        assert_eq!(
            runtime.eval_text("(apply assoc [{} :a 1])").unwrap(),
            "{:a 1}"
        );
        assert_eq!(runtime.eval_text("(apply :a [{:a 42}])").unwrap(), "42");
        assert_eq!(
            runtime.eval_text("(every? :a [{:a true} {:a 1}])").unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "[(filter :a [{:a true} {:b 1}]) \
                      (take 1 ((filter :a) [{:a 1} {:b 2}]))]",
                )
                .unwrap(),
            "[[{:a true}] [{:a 1}]]"
        );
        assert_eq!(runtime.eval_text("(key [1 2])").unwrap(), "1");
        assert_eq!(runtime.eval_text("(val [1 2])").unwrap(), "2");
        assert_eq!(runtime.eval_text("(reverse [1 2 3])").unwrap(), "(3 2 1)");
        assert_eq!(
            runtime
                .eval_text("[(conj nil 1) (satisfies? IConj nil)]")
                .unwrap(),
            "[(1) true]"
        );
    }

    #[test]
    fn structural_hashes_are_stable_and_order_independent_for_maps_and_sets() {
        let mut runtime = Runtime::new();
        let _ = &mut runtime;
        let map_a = core::Value::Map(
            vec![
                (core::Value::Keyword("a".into()), core::Value::Number(1)),
                (core::Value::Keyword("b".into()), core::Value::Number(2)),
            ]
            .into_iter()
            .collect(),
        );
        let map_b = core::Value::Map(
            vec![
                (core::Value::Keyword("b".into()), core::Value::Number(2)),
                (core::Value::Keyword("a".into()), core::Value::Number(1)),
            ]
            .into_iter()
            .collect(),
        );
        let set_a = core::Value::Set(
            vec![
                core::Value::Number(1),
                core::Value::Number(2),
                core::Value::Number(3),
            ]
            .into(),
        );
        let set_b = core::Value::Set(
            vec![
                core::Value::Number(3),
                core::Value::Number(1),
                core::Value::Number(2),
            ]
            .into(),
        );
        assert_eq!(map_a.stable_hash(), map_b.stable_hash());
        assert_eq!(set_a.stable_hash(), set_b.stable_hash());
    }

    #[test]
    fn sequential_representations_share_java_equality_and_hash_semantics() {
        let values = vec![core::Value::Number(1), core::Value::Number(2)];
        let list = core::Value::List(values.clone().into());
        let tuple = core::Value::Tuple(Box::new(
            crate::lang::data::Tuple::from_values(values.clone()).unwrap(),
        ));
        let vector = core::Value::Vector(values.into());

        assert_eq!(list, tuple);
        assert_eq!(tuple, vector);
        assert_eq!(list.stable_hash(), tuple.stable_hash());
        assert_eq!(tuple.stable_hash(), vector.stable_hash());

        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(= [1 2] '(1 2))").unwrap(), "true");
        assert_eq!(runtime.eval_text("(= [1 2] (list 1 2))").unwrap(), "true");
        assert_eq!(runtime.eval_text("(conj (list 2) 1)").unwrap(), "(1 2)");
        assert_eq!(runtime.eval_text("(pair 1 2)").unwrap(), "[1 2]");
        assert_eq!(runtime.eval_text("(key (pair 1 2))").unwrap(), "1");
        assert_eq!(runtime.eval_text("(val (pair 1 2))").unwrap(), "2");
        assert_eq!(runtime.eval_text("(tup 1 2 3 4 5)").unwrap(), "[1 2 3 4 5]");
        assert!(runtime
            .eval_text("(tup 1 2 3 4 5 6)")
            .unwrap_err()
            .contains("at most 5"));
        assert_eq!(runtime.eval_text("(= [1 2] [1 2 3])").unwrap(), "false");
        assert_eq!(
            runtime.eval_text("(get {[1 2] :found} '(1 2))").unwrap(),
            ":found"
        );
        assert_eq!(
            runtime.eval_text("(get #{[1 2]} '(1 2) :missing)").unwrap(),
            "[1 2]"
        );
    }

    #[test]
    fn std_lib_kernel_is_the_canonical_sandbox_surface() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns std-lib-kernel-rust-probe \
                       (:require [std.lib.kernel :as kernel])) \
                     [(fn? kernel/sandbox-open) \
                      (fn? kernel/sandbox-eval) \
                      (fn? kernel/sandbox-call) \
                      (fn? kernel/sandbox-cancel) \
                      (fn? kernel/sandbox-status) \
                      (fn? kernel/sandbox-close) \
                      (fn? kernel/capabilities)]"
                )
                .unwrap(),
            "[true true true true true true true]"
        );
        assert!(runtime
            .eval_text("(require [std.sandbox :as sandbox])")
            .unwrap_err()
            .contains("missing"));
    }

    #[test]
    fn std_lib_collection_families_are_first_class_runtime_values() {
        let mut runtime = Runtime::new();
        runtime
            .eval_text("(ns collection.runtime (:require [std.lib.collection :as collection]))")
            .unwrap();
        assert_eq!(
            runtime.eval_text("(Algo/deque? (Algo/deque 1 2))").unwrap(),
            "true"
        );
        for source in [
            "(= (hash-map :a 1 :b 2) (collection/ordered-map :b 2 :a 1))",
            "(= (hash-map :a 1 :b 2) (collection/sorted-map :b 2 :a 1))",
            "(= (hash-set 1 2) (collection/ordered-set 2 1))",
            "(= (hash-set 1 2) (collection/sorted-set 2 1))",
            "(= (collection/queue 1 2) [1 2])",
        ] {
            assert_eq!(runtime.eval_text(source).unwrap(), "true", "{source}");
        }
        assert_eq!(runtime.eval_text("(get (hash-map :a 1) :a)").unwrap(), "1");
        assert_eq!(
            runtime
                .eval_text("(get (collection/ordered-map :a 1) :a)")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text("(get (collection/sorted-map :a 1) :a)")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text("(get (collection/trie \"alpha\" 7) \"alpha\")")
                .unwrap(),
            "7"
        );
        assert_eq!(
            runtime
                .eval_text("(keys (collection/sorted-map :b 2 :a 1))")
                .unwrap(),
            "[:a :b]"
        );
        assert_eq!(
            runtime
                .eval_text("(nth (collection/queue 4 5 6) 1)")
                .unwrap(),
            "5"
        );
        assert_eq!(
            runtime
                .eval_text("(last (conj (collection/queue 4 5) 6))")
                .unwrap(),
            "6"
        );
        assert_eq!(
            runtime
                .eval_text("(cons 3 (collection/queue 4 5))")
                .unwrap(),
            "(3 4 5)"
        );
        assert_eq!(
            runtime
                .eval_text("(count (dissoc (collection/ordered-set 1 2) 1))")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text("(get (assoc (collection/trie) \"x\" 9) \"x\")")
                .unwrap(),
            "9"
        );
        assert_eq!(
            runtime
                .eval_text("(collection/trie? (collection/trie))")
                .unwrap(),
            "true"
        );
        let error = runtime.eval_text("(hash-map :a)").unwrap_err();
        assert!(error.contains("even number"), "{error}");
        assert!(runtime
            .eval_text("(collection/trie :a 1)")
            .unwrap_err()
            .contains("string keys"));
        assert!(runtime.eval_text("(ordered-map)").is_err());
        assert!(runtime.eval_text("(std.foundation/ordered-map)").is_err());
    }

    #[test]
    fn map_membership_keys_and_values_are_portable() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text(r#"(has? {"a" 1} "a")"#).unwrap(), "true");
        assert_eq!(runtime.eval_text("(has? [1 2] 1)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(has? [1 2] 2)").unwrap(), "false");
        assert_eq!(
            runtime.eval_text(r#"(has? {"a" nil} "a")"#).unwrap(),
            "true"
        );
        assert_eq!(
            runtime.eval_text(r#"(keys {"a" 1 "b" 2})"#).unwrap(),
            "[\"a\" \"b\"]"
        );
        assert_eq!(
            runtime.eval_text(r#"(vals {"a" 1 "b" 2})"#).unwrap(),
            "[1 2]"
        );
    }

    #[test]
    fn core_collection_navigation_and_predicates_are_host_neutral() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(first [1 2 3])").unwrap(), "1");
        assert_eq!(
            runtime
                .eval_text("[(seq? (rest [1 2 3])) (vec (rest [1 2 3]))]")
                .unwrap(),
            "[true [2 3]]"
        );
        assert_eq!(runtime.eval_text("(last [1 2 3])").unwrap(), "3");
        assert_eq!(runtime.eval_text("(empty? [])").unwrap(), "true");
        assert_eq!(runtime.eval_text("(conj [1] 2 3)").unwrap(), "[1 2 3]");
        assert_eq!(
            runtime
                .eval_text("[(sequential? [1]) (sequential? '(1)) (sequential? {:a 1})]")
                .unwrap(),
            "[true true false]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "[(map? {:a 1})\
                     (map? (std.native.Algo/ordered-map :a 1))\
                     (map? [1])\
                     (set? #{1})\
                     (set? (std.native.Algo/ordered-set 1))\
                     (set? [1])\
                     (sequential? '(1 2))\
                     (sequential? [1 2])\
                     (sequential? (tuple 1 2))\
                     (sequential? (std.native.Algo/queue 1 2))\
                     (sequential? (std.native.Algo/deque 1 2))\
                     (sequential? (cons 1 [2]))\
                     (sequential? (seq [1 2]))\
                     (sequential? (std.native.Algo/ordered-set 1))\
                     (coll? (seq [1 2]))\
                     (coll? (iter [1 2]))\
                     (seq? (seq [1 2]))\
                     (seq? [1 2])\
                     (iter? (iter [1 2]))\
                     (iter? [1 2])\
                     (map? (IToMutable/to-mutable {:a 1}))\
                     (set? (IToMutable/to-mutable #{1}))]"
                )
                .unwrap(),
            "[true true false true true false true true true true true true true false false false true false true false true true]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "[(satisfies? IMapType {:a 1})\
                     (satisfies? IMapType [1])\
                     (satisfies? ISetType #{1})\
                     (satisfies? ISetType [1])\
                     (satisfies? ILinearType [1])\
                     (satisfies? ILinearType #{1})]"
                )
                .unwrap(),
            "[true false true false true false]"
        );
        assert_eq!(runtime.eval_text("(not false)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(< 1 2 3)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(>= 3 3)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(mod 7 3)").unwrap(), "1");
    }

    #[test]
    fn atoms_match_java_identity_and_mutation_semantics() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(let [a (atom 1)] @a)").unwrap(), "1");
        assert_eq!(
            runtime
                .eval_text("(do (def deref-test-values (atom [10 18])) (deref deref-test-values))")
                .unwrap(),
            "[10 18]"
        );
        assert_eq!(
            runtime
                .eval_text("(let [a (atom 1)] (do (reset! a 2) @a))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(let [a (atom 1)] (do (swap! a (fn [x y] (+ x y)) 4) @a))")
                .unwrap(),
            "5"
        );
        assert_eq!(
            runtime
                .eval_text("(let [a (atom 1)] [(cas! a 1 2) @a])")
                .unwrap(),
            "[true 2]"
        );
        assert_eq!(
            runtime
                .eval_text("(let [a (atom 1)] [(cas! a 0 2) @a])")
                .unwrap(),
            "[false 1]"
        );
        assert_eq!(
            runtime.eval_text("(let [a (atom 1) b a] (= a b))").unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(= (atom 1) (atom 1))").unwrap(), "false");
        assert_eq!(
            runtime.eval_text("(let [a (atom 1) seen (atom nil)] (do (watch-add a :log (fn [key ref old new] (reset! seen [key @ref old new]))) (reset! a 2) @seen))").unwrap(),
            "[:log 2 1 2]"
        );
        assert_eq!(
            runtime.eval_text("(let [a (atom 1)] (do (watch-add a :log (fn [key ref old new] new)) (watch-add a :log (fn [key ref old new] old)) (count (watch-list a))))").unwrap(),
            "1"
        );
        assert_eq!(
            runtime.eval_text("(let [a (atom 1) seen (atom nil)] (do (watch-add a :log (fn [key ref old new] (reset! seen new))) (watch-remove a :log) (reset! a 2) @seen))").unwrap(),
            "nil"
        );
        assert!(runtime
            .eval_text("(watch-add (atom:basic 1) :log (fn [key ref old new] new))")
            .unwrap_err()
            .contains("unbound symbol: atom:basic"));
        assert!(runtime
            .eval_text("(reset! 1 2)")
            .unwrap_err()
            .contains("IReset/reset"));
        assert!(runtime
            .eval_text("(swap! (atom 1) 2)")
            .unwrap_err()
            .contains("value is not callable"));
        for legacy in [
            "compare:set!",
            "compare-and-set!",
            "add-watch",
            "remove-watch",
            "get-watches",
        ] {
            assert!(
                runtime.eval_text(legacy).unwrap_err().contains("unbound"),
                "{legacy} should not remain public"
            );
        }
    }

    #[test]
    fn keywords_maps_and_sets_match_java_callable_semantics() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(:answer {:answer 42})").unwrap(), "42");
        assert_eq!(runtime.eval_text("(:missing {:answer 42})").unwrap(), "nil");
        assert_eq!(runtime.eval_text("(:missing nil 7)").unwrap(), "7");
        assert_eq!(runtime.eval_text("({:answer 42} :answer)").unwrap(), "42");
        assert_eq!(runtime.eval_text("({:answer 42} :missing 7)").unwrap(), "7");
        assert_eq!(
            runtime.eval_text("(#{:answer} :answer)").unwrap(),
            ":answer"
        );
        assert_eq!(runtime.eval_text("(#{:answer} :missing 7)").unwrap(), "7");
        assert_eq!(
            runtime.eval_text("(:answer)").unwrap_err(),
            "keyword invocation expects one or two arguments"
        );
        assert_eq!(
            runtime.eval_text("({} :a :b :c)").unwrap_err(),
            "map invocation expects one or two arguments"
        );
        assert_eq!(
            runtime
                .eval_text("(map :symbol [{:symbol 'alpha} {:symbol 'beta}])")
                .unwrap(),
            "[alpha beta]"
        );
    }

    #[test]
    fn foundation_fallback_is_eager_canonical_and_shadowable() {
        let mut runtime = Runtime::new();
        let foundation = runtime
            .namespace_registry
            .find("std.foundation")
            .expect("foundation is bootstrapped");
        let canonical = foundation
            .resolve(&crate::lang::data::Symbol::parse("identity"))
            .expect("identity fallback is installed");
        assert_eq!(canonical.origin(), kernel::VarOrigin::HalFallback);
        let user = runtime.namespace_registry.find("user").unwrap();
        assert!(user
            .resolve(&crate::lang::data::Symbol::parse("identity"))
            .is_none());
        let fallback = runtime
            .namespace_registry
            .resolve(&crate::lang::data::Symbol::parse("identity"))
            .unwrap();
        assert!(canonical.same_identity(&fallback));
        assert_eq!(runtime.eval_text("(identity 42)").unwrap(), "42");
        assert_eq!(runtime.eval_text("(apply-with 2 + 1 3)").unwrap(), "6");
        assert_eq!(
            runtime
                .eval_text("(std.foundation/apply-with 2 + 19 21)")
                .unwrap(),
            "42"
        );
        assert!(runtime.eval_text("(apply-with 2 1)").is_err());
        assert_eq!(runtime.eval_text("(first (range 3))").unwrap(), "0");
        assert_eq!(runtime.eval_text("(first (range 2 5))").unwrap(), "2");

        assert_eq!(
            runtime
                .eval_text("(ns project.app (:config {:override [identity]})) (def identity (fn [value] 7)) (identity 42)")
                .unwrap(),
            "7"
        );
        let local = runtime
            .namespace_registry
            .find("project.app")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("identity"))
            .unwrap();
        assert!(!canonical.same_identity(&local));
        assert_eq!(
            runtime.eval_text("(std.foundation/identity 42)").unwrap(),
            "42"
        );
        assert!(runtime
            .eval_text("(ns legacy (:refer-clojure :exclude [identity]))")
            .unwrap_err()
            .contains("Unsupported ns clause: :refer-clojure"));
    }

    #[test]
    fn foundation_current_namespace_scope_preserves_facade_owners() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(ns-current)").unwrap(), "user");
        assert_eq!(
            runtime
                .eval_text("(ns example.current-scope) (ns-current)")
                .unwrap(),
            "example.current-scope"
        );

        runtime.use_namespace("user");
        runtime.eval_text("(require [std.block])").unwrap();

        let block = runtime
            .namespace_registry
            .find("std.block")
            .expect("std.block must be loaded");
        let block_type = block
            .resolve(&crate::lang::data::Symbol::parse("type"))
            .expect("std.block/type must be published");
        assert_eq!(block_type.symbol().as_str(), "std.block/type");

        let foundation = runtime
            .namespace_registry
            .find("std.foundation")
            .expect("std.foundation must be loaded");
        let foundation_type = foundation
            .resolve(&crate::lang::data::Symbol::parse("type"))
            .expect("std.foundation/type must remain published");
        assert_eq!(foundation_type.symbol().as_str(), "std.foundation/type");
        assert_eq!(runtime.eval_text("(std.foundation/seq? [1])").unwrap(), "false");
        assert_eq!(runtime.eval_text("(std.foundation/vector? [1])").unwrap(), "true");
    }

    #[test]
    fn config_only_selects_only_named_foundation_vars() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(ns exposed (:config {:only [identity]})) (identity 42)")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime.eval_text("(merge-nested {} {})").unwrap_err(),
            "unbound symbol: merge-nested"
        );
    }

    #[test]
    fn blank_namespace_collision_controls_preserve_the_canonical_cache() {
        let mut runtime = Runtime::new();
        let foundation = runtime
            .namespace_registry
            .find("std.foundation")
            .expect("foundation is bootstrapped");
        let canonical_identity = foundation
            .resolve(&crate::lang::data::Symbol::parse("identity"))
            .expect("foundation identity is installed");

        runtime.register_resource(
            "xt.collision-probe",
            concat!(
                "(ns xt.collision-probe ",
                "(:config {:blank true}) ",
                "(:require [std.foundation :refer :all ",
                ":exclude [/ do if fn quote try]])) ",
                "(defmacro do [value] (list 'quote value)) ",
                "(defmacro if [value] (list 'quote value)) ",
                "(defmacro fn [value] (list 'quote value)) ",
                "(defmacro quote [value] (list 'quote value)) ",
                "(defmacro try [value] (list 'quote value))"
            ),
        );

        runtime
            .eval_text("(require [xt.collision-probe :as probe])")
            .unwrap();
        runtime
            .eval_text("(require [xt.collision-probe :as probe-again])")
            .unwrap();

        assert_eq!(
            runtime
                .namespace_registry
                .module_revision("xt.collision-probe"),
            1
        );
        assert_eq!(runtime.eval_text("(probe/do 42)").unwrap(), "42");
        assert_eq!(runtime.eval_text("(probe-again/try 7)").unwrap(), "7");

        let cached_identity = runtime
            .namespace_registry
            .find("std.foundation")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("identity"))
            .unwrap();
        assert!(canonical_identity.same_identity(&cached_identity));
    }

    #[test]
    fn fallback_definitions_replace_rust_library_placeholders_with_hal_ownership() {
        let mut runtime = Runtime::new();
        let foundation = runtime.namespace_registry.find_or_create("std.foundation");
        let native = foundation.intern_with_origin(
            "optimized",
            core::Value::Number(7),
            kernel::VarOrigin::RustLibrary,
        );
        let identity = native.identity_address();
        core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
            runtime.eval_text(concat!(
                "(ns std.foundation)",
                " (defn ^{:schema [:fn [:int] :int]} optimized",
                " \"Documents the native implementation.\" [value] 9)"
            ))
        })
        .unwrap();
        let refreshed = foundation
            .resolve(&crate::lang::data::Symbol::parse("optimized"))
            .unwrap();
        assert_eq!(refreshed.identity_address(), identity);
        assert_eq!(refreshed.origin(), kernel::VarOrigin::HalFallback);
        assert!(matches!(refreshed.deref_value(), core::Value::Function(_)));
        assert_eq!(runtime.eval_text("(optimized 1)").unwrap(), "9");
        assert_eq!(refreshed.deref_value().display(), "<fn>");
        assert_eq!(
            refreshed
                .hara_metadata()
                .and_then(|metadata| metadata.doc().map(str::to_owned)),
            Some("Documents the native implementation.".into())
        );
        let metadata = refreshed.hara_metadata().expect("fallback metadata");
        assert_eq!(
            metadata.get_keyword("arglists"),
            Some(&crate::lang::data::MetadataValue::Vector(vec![
                crate::lang::data::MetadataValue::Vector(vec![
                    crate::lang::data::MetadataValue::Symbol(crate::lang::data::Symbol::from(
                        "value"
                    ))
                ])
            ]))
        );
        assert_eq!(
            metadata.get_keyword("schema"),
            Some(&crate::lang::data::MetadataValue::Vector(vec![
                crate::lang::data::MetadataValue::Keyword(crate::lang::data::Keyword::from("fn")),
                crate::lang::data::MetadataValue::Vector(vec![
                    crate::lang::data::MetadataValue::Keyword(crate::lang::data::Keyword::from(
                        "int"
                    ))
                ]),
                crate::lang::data::MetadataValue::Keyword(crate::lang::data::Keyword::from("int"))
            ]))
        );
    }

    #[test]
    fn vm_def_global_takes_ownership_from_a_bootstrap_library_var() {
        let registry = kernel::NamespaceRegistry::new("std.foundation");
        core::with_namespace_registry(&registry, || {
            let seed = registry.current().intern_with_origin(
                "optimized",
                core::Value::Number(7),
                kernel::VarOrigin::RustLibrary,
            );
            let refreshed = core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
                core::vm_def_global("optimized", core::Value::Number(9), None)
            })
            .unwrap();
            assert!(seed.same_identity(&refreshed));
            assert_eq!(refreshed.origin(), kernel::VarOrigin::HalFallback);
            assert_eq!(refreshed.deref_value(), core::Value::Number(9));
        });
    }

    #[test]
    fn base_backed_foundation_facades_are_source_owned() {
        let runtime = Runtime::new();
        let foundation = runtime.namespace_registry.find("std.foundation").unwrap();
        for name in ["list", "boolean", "compare", "long?", "double?", "hash"] {
            let var = foundation
                .resolve(&crate::lang::data::Symbol::parse(name))
                .unwrap_or_else(|| panic!("missing std.foundation/{name}"));
            assert_eq!(
                var.origin(),
                kernel::VarOrigin::HalFallback,
                "std.foundation/{name} must be owned by canonical HAL"
            );
        }

        let base = runtime.namespace_registry.find("std.native.Base").unwrap();
        for name in ["list", "long?", "hash"] {
            let var = base
                .resolve(&crate::lang::data::Symbol::parse(name))
                .unwrap_or_else(|| panic!("missing std.native.Base/{name}"));
            assert_eq!(
                var.origin(),
                kernel::VarOrigin::RuntimePrimitive,
                "std.native.Base/{name} must remain runtime-owned"
            );
        }
        let runtime_namespace = runtime
            .namespace_registry
            .find("std.native.Runtime")
            .unwrap();
        assert!(base
            .resolve(&crate::lang::data::Symbol::parse("resolve"))
            .is_some());
        assert!(base
            .resolve(&crate::lang::data::Symbol::parse("eval"))
            .is_none());
        assert!(runtime_namespace
            .resolve(&crate::lang::data::Symbol::parse("resolve"))
            .is_none());
        assert!(runtime_namespace
            .resolve(&crate::lang::data::Symbol::parse("eval"))
            .is_some());
    }

    #[test]
    fn function_metadata_is_visible_through_meta_and_var_literals() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(concat!(
                    "(defn ^{:schema [:fn [:int] :int]} documented",
                    " \"Returns its argument.\" [value] value)",
                    " (let [m (meta #'documented)]",
                    " [(get m :doc) (get m :arglists) (get m :schema)])"
                ))
                .unwrap(),
            "[\"Returns its argument.\" [[value]] [:fn [:int] :int]]"
        );
        assert_eq!(
            runtime
                .eval_text(concat!(
                    "(let [m (meta #'std.foundation.string/length)]",
                    " [(get m :doc) (get m :arglists) (get m :schema)",
                    "  (get m :inline) (get m :inline-target)])"
                ))
                .unwrap(),
            concat!(
                "[\"Returns the portable character count of value.\"",
                " [[value]] [:fn [:str] :int] true std.native.String/length]"
            )
        );
    }

    #[test]
    fn macro_expansion_preserves_nested_source_metadata() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(concat!(
                    "(defmacro preserve-form [symbol & body]",
                    "  (list 'quote (apply list 'defn symbol body)))",
                    " (let [expanded (preserve-form ^{:- [:integer] :priority 100 :default 0.06}",
                    "                              sample [] 1)]",
                    "   (meta (second expanded)))"
                ))
                .unwrap(),
            "{:- [:integer] :default 0.06 :priority 100}"
        );
    }

    #[test]
    fn definitions_accept_source_metadata_around_hir_syntax() {
        let mut runtime = Runtime::new();
        runtime
            .eval_text(concat!(
                "(defn wrapped ^{:line 1} [value] value)",
                " (defn wrapped-many",
                " ^{:line 2} ([value] value)",
                " ^{:line 3} ([left right] (+ left right)))"
            ))
            .unwrap();
        assert_eq!(
            runtime
                .eval_text(concat!(
                    "[(wrapped 42)",
                    " (wrapped-many 42)",
                    " (wrapped-many 19 23)]"
                ))
                .unwrap(),
            "[42 42 42]"
        );
    }

    #[test]
    fn namespace_values_and_operations_match_java_registry_semantics() {
        let mut runtime = Runtime::new();
        let initial_namespace_count: usize = runtime
            .eval_text("(count (Runtime/namespaces))")
            .unwrap()
            .parse()
            .unwrap();
        runtime.eval_text("(ns example.lib)").unwrap();
        assert_eq!(
            runtime
                .eval_text("(get (Runtime/namespace 'example.lib) :namespace/name)")
                .unwrap(),
            "example.lib"
        );
        assert_eq!(
            runtime
                .eval_text("(= (Runtime/namespace 'example.lib) (Runtime/namespace 'example.lib))")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(ns example.lib) (def answer 42) (ns user) (deref (get (Runtime/vars 'example.lib) 'answer))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime.eval_text("(count (Runtime/namespaces))").unwrap(),
            (initial_namespace_count + 1).to_string()
        );
        assert_eq!(
            runtime
                .eval_text("(Runtime/namespace 'missing.lib)")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime
                .eval_text("(type (Runtime/ns-create 'example.created))")
                .unwrap(),
            ":std.native.Namespace"
        );
        assert_eq!(
            runtime
                .eval_text("(Runtime/ns-name (Runtime/ns-find 'example.created))")
                .unwrap(),
            "example.created"
        );
        assert_eq!(
            runtime
                .eval_text("(Runtime/ns-find 'missing.lib)")
                .unwrap(),
            "nil"
        );
        runtime.alias_namespace("lib", "example.lib");
        assert_eq!(
            runtime
                .eval_text("(deref (Base/resolve 'lib/answer))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text("(Runtime/ns-name (get (Runtime/ns-aliases 'user) 'lib))")
                .unwrap(),
            "example.lib"
        );
        for legacy in [
            "ns:create",
            "ns:list",
            "ns:map",
            "ns:find",
            "ns:name",
            "ns:aliases",
            "ns:imports",
        ] {
            assert_eq!(
                runtime
                    .eval_text(&format!("(resolve '{legacy})"))
                    .unwrap(),
                "nil",
                "legacy namespace operation must be absent: {legacy}"
            );
        }
        let error = runtime.eval_text("(ns bad/name)").unwrap_err();
        assert!(error.contains("ns expects an unqualified namespace symbol"), "{error}");
    }

    #[test]
    fn native_imports_are_recorded_without_a_host_flavor() {
        let mut runtime = Runtime::new();
        let add = b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";
        for logical in ["math", "vendor.numeric.Vector", "vendor.numeric.Matrix"] {
            runtime.install_direct_wasm_import(logical, add).unwrap();
        }
        assert_eq!(
            runtime
                .eval_text(
                    "(ns direct.imports (:import math [vendor.numeric Vector Matrix]))\
                     (math/add 19 23)",
                )
                .unwrap(),
            "42"
        );
        let imports = runtime
            .namespace_registry
            .find("direct.imports")
            .unwrap()
            .imports();
        assert_eq!(imports.len(), 3);
        assert_eq!(
            imports
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>(),
            std::collections::HashMap::from([
                (crate::lang::data::Symbol::parse("math"), "math".into()),
                (
                    crate::lang::data::Symbol::parse("Vector"),
                    "vendor.numeric.Vector".into()
                ),
                (
                    crate::lang::data::Symbol::parse("Matrix"),
                    "vendor.numeric.Matrix".into()
                ),
            ])
        );
        assert_eq!(
            runtime
                .namespace_registry
                .find("direct.imports")
                .unwrap()
                .native_flavor(),
            None
        );
    }

    #[test]
    fn native_import_flavor_validation_is_deterministic() {
        let mut runtime = Runtime::new();
        runtime
            .install_direct_wasm_import(
                "math",
                b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b",
            )
            .unwrap();
        assert_eq!(
            runtime
                .eval_text("(ns explicit.wasm (:flavor :wasm) (:import math))")
                .unwrap_err(),
            "native/unsupported-flavor: :wasm (Wasm modules use :import)"
        );
        assert_eq!(
            runtime
                .eval_text("(ns wrong.runtime (:flavor :jvm) (:import java.lang.String))")
                .unwrap_err(),
            "native/unsupported-flavor: :jvm (host flavors are only available on JVM/.NET runtimes)"
        );
        assert_eq!(
            runtime
                .eval_text("(ns missing.import (:import absent))")
                .unwrap_err(),
            "native/import-missing: absent"
        );
    }

    #[test]
    fn native_import_traps_have_a_stable_diagnostic_boundary() {
        let mut runtime = Runtime::new();
        let trap = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x03\x02\x01\0\x07\x08\x01\x04boom\0\0\x0a\x05\x01\x03\0\0\x0b";
        runtime.install_direct_wasm_import("broken", trap).unwrap();
        let error = runtime
            .eval_text("(ns direct.trap (:import broken)) (broken/boom)")
            .unwrap_err();
        assert!(
            error.contains("native/invoke-failed: broken/boom"),
            "{error}"
        );
    }

    #[test]
    fn namespace_use_refers_portable_test_vars_and_macros() {
        // The debug evaluator recursively loads the portable code.test graph.
        // Keep that implementation detail local to this test rather than
        // raising the stack for every native runtime test.
        std::thread::Builder::new()
            .name("namespace-use-portable-test-probe".into())
            // This is a debug-only portability probe for the recursive
            // interpreter. Keep its exceptional headroom out of production
            // runtime threads, which use the bounded 8 MiB stack.
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = development_runtime();
                assert_eq!(
                    runtime
                        .eval_text(concat!(
                            "(ns code.test-rust-probe",
                            "  (:use code.test)",
                            "  (:require [code.test.base.process :as process]))",
                            " (def lifecycle (atom []))",
                            " (fact \"promise assertion\"",
                            "   {:before (fn []",
                            "              (swap! lifecycle",
                            "                     (fn [events] (conj events :before))))",
                            "    :after (fn []",
                            "             (swap! lifecycle",
                            "                    (fn [events] (conj events :after))))}",
                            "   (promise/from 42) => 42",
                            "   (+ 1 1) => 2)",
                            " (let [summary (run {:namespace \"code.test-rust-probe\"})",
                            "       timed (check (fn [] (promise/from 42)) 42",
                            "                    {:work/timeout-promise",
                            "                     (fn [promise milliseconds]",
                            "                       {:promise (promise/from {:test/status :timeout})",
                            "                        :timeout milliseconds})",
                            "                     :work/cancel-timeout identity",
                            "                     :timeout 25})",
                            "       positional (run '[code])",
                            "       cancelled",
                            "       (run {:namespace \"code.test-rust-probe\"",
                            "             :cancelled true})]",
                            " [(:status summary)",
                            "  (:passed (:counts summary))",
                            "  (count (:checks (first (:results summary))))",
                            "  (process/timeout-result? timed)",
                            "  (:facts positional)",
                            "  (:cancelled (:counts cancelled))])"
                        ))
                        .unwrap(),
                    "[:passed 1 2 true 1 1]"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn foundation_code_test_compatibility_namespaces_are_embedded() {
        std::thread::Builder::new()
            .name("foundation-code-test-compatibility".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(foundation_code_test_compatibility_namespaces_are_embedded_body)
            .unwrap()
            .join()
            .unwrap();
    }

    fn foundation_code_test_compatibility_namespaces_are_embedded_body() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns code-test-compat-rust-probe \
                       (:require [code.test :as test] \
                                 [code.test.checker.common :as common] \
                                 [code.test.checker.collection :as collection] \
                                 [code.test.checker.logic :as logic] \
                                 [code.test.base.runtime :as runtime] \
                                 [code.test.compile.types :as types])) \
                     (let [fact (types/Fact :core 'id 'probe nil nil \
                                            \"portable\" 1 1 nil nil \
                                            (fn [] 42) {})] \
                       [(common/succeeded? \
                         (common/verify (common/exactly 1) 1)) \
                        (test/comparison-passed? (test/check \
                                (fn [] {:a 1 :b 2}) \
                                (collection/contains-map {:a 1}))) \
                        (test/comparison-passed? (test/check \
                                (fn [] 3) \
                                (logic/all (fn [value] (number? value)) \
                                           (fn [value] (= 1 (mod value 2)))))) \
                        (types/fact? fact) \
                        (fact) \
                        (test/process-test-args \
                         [\":only\" \"std\" \"code\"])])"
                )
                .unwrap(),
            "[true true true true 42 {:namespace [std code]}]"
        );
    }

    #[test]
    fn canonical_component_and_context_libraries_load_without_old_aliases() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns std-lib-context-rust-probe \
                       (:require [std.lib.component :as component] \
                                 [std.lib.context.registry :as context])) \
                     (let [runtime context/+rt-null+] \
                       [(component/started? runtime) \
                        (IContext/call runtime :a :b)])"
                )
                .unwrap(),
            "[true [:a :b]]"
        );
        assert!(runtime
            .eval_text("(require [std.foundation.component :as old])")
            .unwrap_err()
            .contains("missing"));
    }

    #[test]
    fn portable_command_templates_are_data_first() {
        std::thread::Builder::new()
            .name("portable-command-probe".into())
            // Loading the portable library in the debug evaluator is deeply
            // recursive; give this portability probe the same headroom as the
            // Java and browser hosts rather than depending on the test default.
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = development_runtime();
                assert_eq!(
                    runtime
                        .eval_native(
                            "(ns std-work-command-rust-probe \
                       (:require [work.base :as work] \
                                 [work.flow.task.command :as command])) \
                     (def double-command \
                       (command/single \
                        {:id :probe/double :version 1} \
                        {:process (work/pure :probe/process \
                                   (fn [value context] (* 2 value)))})) \
                     (let [observer (work/recording-observer) \
                           host (work/local-runtime {:observer observer}) \
                           output (work/run host double-command 4) \
                           completed \
                           (filter (fn [event] \
                                     (= :command/completed (:event event))) \
                                   (work/observer-events observer))] \
                       [output \
                        (:op (work/work-spec double-command)) \
                        (count completed) \
                        (command/parse-args \
                         [\":only\" \"std\" \"code\" \
                          \":parallel\" \"true\"])])"
                        )
                        .unwrap(),
                    "[8 :chain 1 {:selector [std code] :parallel true}]"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn removed_work_compatibility_namespaces_are_not_embedded() {
        let mut runtime = Runtime::new();
        for namespace in ["std.work", "std.work.recipe"] {
            let error = runtime
                .eval_text(&format!("(require '{namespace})"))
                .unwrap_err();
            assert!(
                error.contains("missing"),
                "unexpected error requiring removed {namespace}: {error}"
            );
        }
    }

    #[test]
    fn portable_block_preserves_source_value_and_structure() {
        std::thread::Builder::new()
            .name("portable-std-block-probe".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = Runtime::new();
                runtime
                    .eval_text(
                        "(ns std-block-rust-probe \
                         (:require [std.block :as block] [std.block.grid :as grid] \
                                   [std.block.navigate :as navigate] \
                                   [std.block.reader :as reader]))",
                    )
                    .unwrap();
                let probes = [
                    ("[(type (str/char-at \" \" 0)) (str/char-at \" \" 0)]", "[:std.native.Character \\space]"),
                    ("(let [b (block/parse-string \"[1 2 3]\")] [(block/string b) (block/value b)])", "[\"[1 2 3]\" [1 2 3]]"),
                    ("(let [b (block/parse-first \"[1 2 3]\")] [(block/type b) (block/tag b) (vec (map block/value (filter block/code? (block/children b))))])", "[:container :vector [1 2 3]]"),
                    ("(let [blocks (block/spaces 3)] [(apply str (map block/string blocks)) (every? block/space? blocks)])", "[\"   \" true]"),
                    ("(block/string (block/layout '(if ready [1 2] [3 4]) {:width 10}))", "\"(if ready [1 2] [3 4])\""),
                    ("(block/string (grid/grid (block/parse-first \"(if\\nready\\ndone)\") 0 {:rules {'if {:indent 1}}}))", "\"(if\\n  ready\\n  done)\""),
                    ("(let [b (block/parse-first \"[1 #_2 3]\")] [(block/value b) (block/child-values b)])", "[[1 3] [1 3]]"),
                    ("(let [original (block/block [1 2]) location (std.lib.zip/step-right (std.lib.zip/step-right (std.lib.zip/step-inside (navigate/navigator original)))) edited (std.lib.zip/root-element (std.lib.zip/replace-right location (block/block 3)))] [(block/string original) (block/string edited)])", "[\"[1 2]\" \"[1 3]\"]"),
                    ("(let [input (reader/create \"ab\\ncd\") first-two (reader/read-times input reader/read-char 2) newline (reader/read-char input)] [first-two (reader/reader-position input) (reader/read-to-boundary input)])", "[[\\a \\b] [2 1] \"cd\"]"),
                    ("(block/value (block/parse-string \"[4 5]\"))", "[4 5]"),
                ];
                for (source, expected) in probes {
                    let actual = runtime
                        .eval_text(source)
                        .unwrap_or_else(|error| panic!("{source}: {error}"));
                    assert_eq!(actual, expected, "{source}");
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn portable_block_heal_parses_and_pairs_delimiters() {
        std::thread::Builder::new()
            .name("portable-std-block-heal-probe".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = development_runtime();
                runtime
                    .eval_text(
                        "(ns std-block-heal-rust-probe \
                         (:require [code.test.checker.common :as checker] \
                                   [std.block.heal.edit :as edit] \
                                   [std.block.heal.parse :as parse]))",
                    )
                    .unwrap();
                assert_eq!(
                    runtime
                        .eval_text("(parse/parse-delimiters \"(a)\")")
                        .unwrap(),
                    "[{:style :paren :type :open :line 1 :col 1 :char \"(\"} {:style :paren :type :close :line 1 :col 3 :char \")\"}]"
                );
                assert_eq!(
                    runtime
                        .eval_text(
                            "(let [pairs (parse/pair-delimiters (parse/parse-delimiters \"(a)\"))] \
                               [(count pairs) (every? :pair-id pairs)])",
                        )
                        .unwrap(),
                    "[2 true]"
                );
                assert_eq!(
                    runtime
                        .eval_text(
                            "[(function? vector?) ((checker/satisfies vector?) [1 2])]",
                        )
                        .unwrap(),
                    "[true true]"
                );
                assert_eq!(
                    runtime
                        .eval_text(
                            "(edit/update-content \"(\" [{:action :insert :col 1 :line 1 :new-char \")\"}])",
                        )
                        .unwrap(),
                    "\"()\""
                );
                assert_eq!(
                    runtime
                        .eval_text(
                            "[(vector? (parse/parse-lines \"(a)\")) \
                              ((checker/satisfies vector?) (parse/parse-lines \"(a)\"))]",
                        )
                        .unwrap(),
                    "[true true]"
                );
                assert_eq!(
                    runtime
                        .eval_text(
                            "(Test/run [{:name \"edit\" \
                                         :test (fn [] \
                                                 (edit/update-content \
                                                   \"(\" \
                                                   [{:action :insert \
                                                     :col 1 \
                                                     :line 1 \
                                                     :new-char \")\"}])) \
                                         :expected \"()\"}])",
                        )
                        .unwrap(),
                    "[#hara/Result[:success true nil {:failures [] :test {:name \"edit\" :actual \"()\" :expected \"()\"}}]]"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn portable_zip_is_embedded_and_preserves_original_values() {
        std::thread::Builder::new()
            .name("portable-std-lib-zip-probe".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = Runtime::new();
                assert_eq!(
                    runtime
                        .eval_text(
                            "(ns std-lib-zip-rust-probe \
                               (:require [std.lib.zip :as zip])) \
                             (let [root [1 2 3] \
                                   location (zip/step-right \
                                             (zip/step-inside (zip/vector-zip root))) \
                                   edited (zip/replace-right \
                                           (zip/insert-left location 9) 8)] \
                               [(zip/root-element edited) root])"
                        )
                        .unwrap(),
                    "[[1 9 8 3] [1 2 3]]"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn portable_collection_execution_preserves_hal_semantics_and_errors() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(reduce (fn [total value] (+ total value)) 0 [1 2 3 4]) \
                      (let [answer 41] (+ answer 1) (+ answer 2))]"
                )
                .unwrap(),
            "[10 43]"
        );
        assert!(runtime
            .eval_text("(vec (map (fn [value] (/ 1 value)) [1 0]))")
            .unwrap_err()
            .contains("division by zero"));
    }

    #[test]
    fn named_values_expose_java_basic_object_operations() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(compare :a :b)").unwrap(), "-1");
        assert_eq!(
            runtime
                .eval_text("(compare (symbol \"a\") (symbol \"a\"))")
                .unwrap(),
            "0"
        );
        assert_eq!(
            runtime
                .eval_text("(= (hash [1 2]) (hash (list 1 2)))")
                .unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(meta :answer)").unwrap(), "nil");
        assert_eq!(
            runtime
                .eval_text("(with-meta :answer {:doc \"ignored\"})")
                .unwrap(),
            ":answer"
        );
        assert_eq!(
            runtime
                .eval_text("(get (meta (with-meta (symbol \"answer\") {:doc \"named\"})) :doc)")
                .unwrap(),
            "\"named\""
        );
        assert_eq!(
            runtime
                .eval_text("(get (meta (with-meta [1] {:doc \"vector\"})) :doc)")
                .unwrap(),
            "\"vector\""
        );
        assert_eq!(
            runtime
                .eval_text("(meta (with-meta (with-meta [1] {:doc \"vector\"}) nil))")
                .unwrap(),
            "nil"
        );
        assert!(runtime
            .eval_text("(hash)")
            .unwrap_err()
            .contains("function expects 1 arguments"));
    }

    #[test]
    fn cons_pointer_and_tagged_literals_are_first_class_runtime_values() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(cons 0 [1 2])").unwrap(), "(0 1 2)");
        assert_eq!(
            runtime.eval_text("(type (cons 0 [1 2]))").unwrap(),
            ":std.native.Cons"
        );
        assert_eq!(runtime.eval_text("(count (cons 0 [1 2]))").unwrap(), "3");
        assert_eq!(runtime.eval_text("(get (cons 0 [1 2]) 2)").unwrap(), "2");
        assert_eq!(
            runtime
                .eval_text("#ptr {:context :kernel :id \"ROOT\"}")
                .unwrap(),
            "#ptr {:context :kernel :id \"ROOT\"}"
        );
        assert_eq!(
            runtime
                .eval_text("(type (pointer {:context :test :refer 'tool.lint/lint-source :id 'lint-source}))")
                .unwrap(),
            ":std.native.Pointer"
        );
        assert_eq!(
            runtime
                .eval_text("(get #ptr {:context :kernel :id \"ROOT\"} :id)")
                .unwrap(),
            "\"ROOT\""
        );
        assert_eq!(
            runtime
                .eval_text("(count #ptr {:context :kernel :id \"ROOT\"})")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(std.protocol.ifind.IFind/find #ptr {:context :kernel :id \"ROOT\"} :id)",
                )
                .unwrap(),
            "[:id \"ROOT\"]"
        );
        assert_eq!(
            runtime
                .eval_text("(keys #ptr {:context :kernel :id \"ROOT\"})")
                .unwrap(),
            "[:id]"
        );
        assert_eq!(
            runtime
                .eval_text("(vals #ptr {:context :kernel :id \"ROOT\"})")
                .unwrap(),
            "[\"ROOT\"]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(do (require 'std.lib.context.space) (deref #ptr {:context :null :id \"ROOT\"}))"
                )
                .unwrap(),
            "[:pointer/deref #ptr {:context :null :id \"ROOT\"}]"
        );
        assert_eq!(
            runtime
                .eval_text("(#ptr {:context :null :id \"ROOT\"} 1 2)")
                .unwrap(),
            "[:pointer/invoke #ptr {:context :null :id \"ROOT\"} 1 2]"
        );
        assert!(runtime
            .eval_text("#ptr {:id \"ROOT\"}")
            .unwrap_err()
            .contains("pointer descriptor requires :context"));
        assert_eq!(
            runtime.eval_text("(type #sample [1 2])").unwrap(),
            ":std.native.TaggedLiteral"
        );
        assert_eq!(runtime.eval_text("(ILookup/lookup (IObjType/meta (IObjType/with-meta (cons 0 [1]) {:doc \"cons\"})) :doc)").unwrap(), "\"cons\"");
    }

    #[test]
    fn native_test_catalog_uses_runtime_runner_and_test_context() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(= Test std.native.Test) \
                      (get (Test/catalog) :runners) \
                      (get (Test/catalog) :default) \
                      (get (Test/catalog) :context) \
                      (Test/events)]"
                )
                .unwrap(),
            "[true [:code.test :native] :code.test :test [:test/run-started :test/fact-started :test/fact-completed :test/run-completed]]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let [config (Test/config {:focus :fast}) \
                           context (Test/context config)] \
                       [(get config :runner) \
                        (get (get config :options) :focus) \
                        (IPointer/ptr-context context) \
                        (get context :id) \
                        (get (get context :config) :runner)])"
                )
                .unwrap(),
            "[:code.test :fast :test :test :code.test]"
        );
        assert!(runtime
            .eval_text("(Test/config {:runner :native})")
            .unwrap_err()
            .contains("runner is owned by the runtime"));
        assert_eq!(
            runtime
                .eval_text(
                    "(let [equal (Test/result \"equal\" 7 7 (Test/compare 7 7)) \
                           different (Test/result \"different\" 7 8 (Test/compare 7 8))] \
                       [(Test/passed? equal) \
                        (Test/passed? different) \
                        (Test/actual different) \
                        (Test/expected different) \
                        (Test/failure-count different) \
                        (Test/failure? (Test/failure different 0)) \
                        (count (Test/failures different)) \
                        (count (Test/failure-seq different))])"
                )
                .unwrap(),
            "[true false 7 8 1 true 1 1]"
        );
        assert!(runtime
            .eval_text("(Test/passed? {:status :error})")
            .unwrap_err()
            .contains("expects a Result"));
        assert_eq!(
            runtime
                .eval_text(
                    "(let [leaf (fn [code] \
                                  {:failure/code code :failure/path [] :failure/in [] \
                                   :failure/actual nil :failure/expected nil \
                                   :failure/message \"failure\" :failure/context {} \
                                   :failure/children []}) \
                           left (leaf :left) right (leaf :right) \
                           parent {:failure/code :parent :failure/path [] :failure/in [] \
                                   :failure/actual nil :failure/expected nil \
                                   :failure/message \"parent\" :failure/context {} \
                                   :failure/children [left right]} \
                           result (Result/create :success false {:failures [parent]})] \
                       [(vec (map :failure/code (Test/failure-seq result))) \
                        (Test/failure-count result) \
                        (:failure/code (Test/failure result 1)) \
                        (Test/failure? parent) \
                        (Test/failure? (assoc parent :failure/children [{}]))])"
                )
                .unwrap(),
            "[[:left :right] 2 :right true false]"
        );

        assert_eq!(
            runtime
                .eval_text("(Test/run [{:name \"one\" :test (fn [] (+ 1 1)) :expected 2}])")
                .unwrap(),
            "[#hara/Result[:success true nil {:failures [] :test {:name \"one\" :actual 2 :expected 2}}]]"
        );
        let cumulative = runtime
            .eval_text("(Test/run [{:name \"two\" :test (fn [] (throw \"boom\")) :expected 2}])")
            .unwrap();
        assert!(cumulative.contains(":name \"one\""), "{cumulative}");
        assert!(cumulative.contains(":name \"two\""), "{cumulative}");
        assert!(cumulative.contains("#hara/Result[:error"), "{cumulative}");
        assert_eq!(
            runtime
                .eval_text("(Test/run [])")
                .unwrap()
                .matches(":name")
                .count(),
            2
        );
        let malformed = runtime.eval_text("(Test/run [{} 1])").unwrap();
        assert_eq!(
            malformed.matches("#hara/Result[:error").count(),
            3,
            "{malformed}"
        );

        let mut checked_runtime = Runtime::new();
        let checked = checked_runtime
            .eval_text(
                "(Test/run [{:name \"checked\" :meta {:refer (quote demo/value)} \
                              :test (fn [] 7) :expected odd?}] \
                   (fn [thunk expected] \
                     (let [actual (thunk)] \
                       (Test/result \"checker\" actual :predicate \
                         (Test/compare (expected actual) true)))))",
            )
            .unwrap();
        assert!(checked.contains(":name \"checked\""), "{checked}");
        assert!(checked.contains("#hara/Result[:success true"), "{checked}");
        assert!(checked.contains(":meta {:refer demo/value}"), "{checked}");
        let local_failures = checked_runtime
            .eval_text(
                "(Test/run [{:name \"throws\" :test (fn [] 1) :expected 1} \
                             {:name \"continues\" :test (fn [] 2) :expected 2}] \
                   (fn [thunk expected] (throw \"checker boom\")))",
            )
            .unwrap();
        assert!(
            local_failures.contains(":name \"throws\""),
            "{local_failures}"
        );
        assert!(
            local_failures.contains(":name \"continues\""),
            "{local_failures}"
        );
        assert_eq!(
            local_failures.matches("#hara/Result[:error").count(),
            2,
            "{local_failures}"
        );
        let malformed_check = checked_runtime
            .eval_text(
                "(Test/run [{:name \"malformed\" :test (fn [] 1) :expected 1}] \
                   (fn [thunk expected] true))",
            )
            .unwrap();
        assert!(malformed_check.contains("check function must return a Result"));

        runtime.set_test_runner("native").unwrap();
        assert_eq!(
            runtime
                .eval_text("[(get (Test/catalog) :runner) (get (Test/config) :runner)]")
                .unwrap(),
            "[:native :native]"
        );
    }

    #[test]
    fn native_test_result_api_shared_corpus_passes() {
        let mut runtime = Runtime::new();
        let output = runtime
            .eval_text(include_str!(
                "../../../lib/test-fixtures/std/native/test_result_api.hal"
            ))
            .unwrap();
        assert_eq!(output.matches("#hara/Result[:success true").count(), 3);
        assert!(!output.contains("#hara/Result[:success false"), "{output}");
        assert!(!output.contains("#hara/Result[:error"), "{output}");
    }

    #[test]
    fn keyword_symbol_constructors_and_namespaced_protocol_match_java() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(keyword \"answer\")").unwrap(),
            ":answer"
        );
        assert_eq!(
            runtime.eval_text("(keyword \"core\" \"answer\")").unwrap(),
            ":core/answer"
        );
        assert_eq!(runtime.eval_text("(symbol \"answer\")").unwrap(), "answer");
        assert_eq!(
            runtime.eval_text("(symbol \"core\" \"answer\")").unwrap(),
            "core/answer"
        );
        assert_eq!(
            runtime
                .eval_text("(INamespaced/name :core/answer)")
                .unwrap(),
            "\"answer\""
        );
        assert_eq!(
            runtime
                .eval_text("(INamespaced/namespace (symbol \"core\" \"answer\"))")
                .unwrap(),
            "\"core\""
        );
        assert_eq!(
            runtime
                .eval_text("(INamespaced/namespace :answer)")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime.eval_text("(namespace :core/answer)").unwrap(),
            "\"core\""
        );
        assert_eq!(
            runtime.eval_text("(name :core/answer)").unwrap(),
            "\"answer\""
        );
        assert_eq!(
            runtime
                .eval_text("(name (symbol \"core\" \"answer\"))")
                .unwrap(),
            "\"answer\""
        );
        assert!(runtime
            .eval_text("(keyword \"a/b/c\")")
            .unwrap_err()
            .contains("one slash"));
        assert!(runtime
            .eval_text("(symbol 1)")
            .unwrap_err()
            .contains("expects a name or namespace and name"));
    }

    #[test]
    fn foundation_compiler_support_functions_are_available_at_root() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(reduce-kv (fn [out key value] (assoc out key (+ value 1))) {} {:a 1 :b 2})"
                )
                .unwrap(),
            "{:a 2 :b 3}"
        );
        assert_eq!(
            runtime
                .eval_text("(select-keys {:a 1 :b 2} [:b :missing])")
                .unwrap(),
            "{:b 2}"
        );
        assert_eq!(
            runtime.eval_text("(merge {:a 1 :b 2} {:b 3})").unwrap(),
            "{:a 1 :b 3}"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "[(meta (Base/vec (with-meta (vector 1) {:tag :vector}))) \
                      (meta (vec (with-meta (vector 1) {:tag :vector}))) \
                      (meta (set (with-meta #{1} {:tag :set})))]"
                )
                .unwrap(),
            "[{:tag :vector} {:tag :vector} {:tag :set}]"
        );
        assert_eq!(
            runtime.eval_text("(fn? (deref (resolve 'inc)))").unwrap(),
            "true"
        );
        assert_eq!(
            runtime.eval_text("(nil? (resolve 'missing))").unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(ex-native-type (ex-info \"broken\" {:phase :test}))")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime
                .eval_text("(ex-native-type (ex :file/read {}))")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime
                .eval_text("(ex-class (ex :file/read {:ex/class :ex.class/io}))")
                .unwrap(),
            ":ex.class/io"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let [error (ex :file/read {} :ex/message \"missing\" :ex/class :ex.class/io)] \
                       [(:ex/code (ex-data error)) (ex-message error) (ex-class error)])"
                )
                .unwrap(),
            "[:file/read \"missing\" :ex.class/io]"
        );
        assert_eq!(
            runtime.eval_text("(ex-class (ex :file/read {}))").unwrap(),
            "nil"
        );
        assert_eq!(
            runtime.eval_text("(ex-class (ex :not-found {}))").unwrap(),
            ":ex.class/not-found"
        );
        assert_eq!(
            runtime.eval_text("(ex-class (ex :generic {}))").unwrap(),
            ":ex.class/internal"
        );
        assert_eq!(
            runtime
                .eval_text("(:ex/code (ex-data (ex :not-found {})))")
                .unwrap(),
            ":hara/not-found"
        );
        assert!(runtime
            .eval_text("(ex :not-found {:ex/class :ex.class/io})")
            .is_err());
        assert!(runtime.eval_text("(ex :unknown {})").is_err());
        assert!(runtime
            .eval_text("(ex :file/read {:ex/class :io})")
            .is_err());
        assert!(runtime.eval_text("(ex-native-type 42)").is_err());
    }

    #[test]
    fn reader_vectors_use_java_tuple_arity_selection() {
        let mut env = HashMap::new();
        let small = core::eval(&kernel::parse("[1 2 3]").unwrap(), &mut env).unwrap();
        let large = core::eval(&kernel::parse("[1 2 3 4 5 6]").unwrap(), &mut env).unwrap();
        assert!(matches!(small, core::Value::Tuple(_)));
        assert!(matches!(large, core::Value::Vector(_)));

        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(nth [1 2 3] 1)").unwrap(), "2");
        assert_eq!(runtime.eval_text("(conj [1 2 3] 4)").unwrap(), "[1 2 3 4]");
        assert_eq!(
            runtime
                .eval_text(
                    "(loop [values [] n 0]
                       (if (< n 32)
                         (recur (conj values n) (+ n 1))
                         [(count values) (first values) (nth values 31)]))"
                )
                .unwrap(),
            "[32 0 31]"
        );
        let promoted = core::eval(
            &kernel::parse("(conj (conj (conj (conj [0 1 2 3 4] 5) 6) 7) 8)").unwrap(),
            &mut env,
        )
        .unwrap();
        assert!(matches!(promoted, core::Value::Vector(values) if values.len() == 9));
        assert_eq!(
            runtime
                .eval_text("(ILookup/lookup (IObjType/meta (IObjType/with-meta [1] {:doc \"tuple\"})) :doc)")
                .unwrap(),
            "\"tuple\""
        );
    }

    #[test]
    fn reader_maps_and_sets_preserve_java_hash_order() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("{:b 2 :a 1}").unwrap(), "{:b 2 :a 1}");
        assert_eq!(runtime.eval_text("(keys {:b 2 :a 1})").unwrap(), "[:b :a]");
        assert_eq!(runtime.eval_text("#{:b :a}").unwrap(), "#{:b :a}");
        assert_eq!(
            runtime
                .eval_text("(conj (dissoc {:a 1 :b 2} :a) [:a 3])")
                .unwrap(),
            "{:b 2 :a 3}"
        );
    }

    #[test]
    fn collection_operations_are_values() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(count [1 2 3])").unwrap(), "3");
        assert_eq!(runtime.eval_text("(get {\"a\" 9} \"a\")").unwrap(), "9");
        assert_eq!(runtime.eval_text("(nth (conj [1] 2) 1)").unwrap(), "2");
        assert_eq!(
            runtime.eval_text(r#"(conj {"a" 1} ["b" 2])"#).unwrap(),
            r#"{"a" 1 "b" 2}"#
        );
        assert_eq!(
            runtime
                .eval_text(r#"(get (conj {"a" 1} ["a" 9]) "a")"#)
                .unwrap(),
            "9"
        );
        assert_eq!(
            runtime.eval_text(r#"(dissoc {"a" 1 "b" 2} "a")"#).unwrap(),
            r#"{"b" 2}"#
        );
        assert_eq!(
            runtime
                .eval_text(r#"(dissoc {"a" 1 "b" 2} "a" "b")"#)
                .unwrap(),
            "{}"
        );
        assert_eq!(runtime.eval_text("(cons 0 [1 2])").unwrap(), "(0 1 2)");
        assert_eq!(runtime.eval_text("(= :ready :ready)").unwrap(), "true");
    }

    #[test]
    fn persistent_vectors_and_lists_keep_previous_values() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("[(first '()) (first (vec [])) (last '()) (last (vec []))]")
                .unwrap(),
            "[nil nil nil nil]"
        );
        assert_eq!(runtime.eval_text("(conj '(1) 2)").unwrap(), "(2 1)");
        assert_eq!(runtime.eval_text("(drop 1 '(a b c d))").unwrap(), "(b c d)");
        assert_eq!(
            runtime
                .eval_text("(let (source [1 2]) (get (conj source 3) 2))")
                .unwrap(),
            "3"
        );
        assert_eq!(
            runtime
                .eval_text("(let (source [1 2]) (count source))")
                .unwrap(),
            "2"
        );
        assert!(runtime
            .eval_text("(conj (rest [1 2]) 2)")
            .unwrap_err()
            .contains("IConj/conj expects a collection"));
        assert_eq!(
            runtime.eval_text("(vec (cons 0 (rest [1 2])))").unwrap(),
            "[0 2]"
        );
        assert_eq!(
            runtime
                .eval_text("(let (source (rest [1 2])) (count source))")
                .unwrap(),
            "1"
        );
    }

    struct RangeExtension;

    impl core::ExtensionProvider for RangeExtension {
        fn name(&self) -> &str {
            "range"
        }

        fn install(&self, protocols: &mut core::ProtocolRegistry) {
            protocols.register_when(
                "IIter",
                "iter",
                |value| {
                    matches!(value, core::Value::Extension(value)
                    if value.provider == "range" && value.type_name == "range")
                },
                |arguments| match arguments.first() {
                    Some(core::Value::Extension(value))
                        if value.provider == "range" && value.type_name == "range" =>
                    {
                        Ok(core::iterator_from_values(
                            (0..value.handle)
                                .map(|index| core::Value::Number(index as i64))
                                .collect(),
                        ))
                    }
                    _ => Err("range/IIter does not accept this value".into()),
                },
            );
        }

        fn construct(
            &self,
            type_name: &str,
            arguments: &[core::Value],
        ) -> Result<core::Value, String> {
            if type_name != "range" {
                return Err("range/type-not-found".into());
            }
            let count = match arguments.first() {
                Some(core::Value::Number(count)) if *count >= 0 => *count as u64,
                _ => return Err("range expects a non-negative count".into()),
            };
            Ok(core::Value::Extension(core::ExtensionValue {
                provider: "range".into(),
                type_name: "range".into(),
                handle: count,
            }))
        }
    }

    fn protocol_identity(arguments: &[core::Value]) -> Result<core::Value, String> {
        arguments
            .first()
            .cloned()
            .ok_or_else(|| "missing receiver".into())
    }

    fn protocol_custom_iterator(_arguments: &[core::Value]) -> Result<core::Value, String> {
        Ok(core::iterator_from_values(vec![
            core::Value::Number(7),
            core::Value::Number(8),
        ]))
    }

    #[test]
    fn promise_constructors_and_composition() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(deref (promise/new (fn [resolve reject] (resolve 42))))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime.eval_text("(deref (promise (fn [] 40)))").unwrap(),
            "40"
        );
        assert_eq!(
            runtime.eval_text("(deref (promise/from 42))").unwrap(),
            "42"
        );
        assert_eq!(
            runtime.eval_text("(promise? (promise/from 1))").unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(promise? 1)").unwrap(), "false");
        assert_eq!(
            runtime
                .eval_text("(deref (promise/then (promise (fn [] 40)) (fn [x] (+ x 2))))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (promise/catch (promise (fn [] (throw :bad))) (fn [error] 7)))")
                .unwrap(),
            "7"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (promise/finally (promise (fn [] 4)) (fn [] 99)))")
                .unwrap(),
            "4"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(Arr/get (deref (promise/all [(promise (fn [] 1)) 2 (promise (fn [] 3))])) 1)"
                )
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (promise (fn [] (promise (fn [] 9)))))")
                .unwrap(),
            "9"
        );
        assert_eq!(
            runtime
                .eval_text("(deref (promise/delay 0 (fn [] 5)))")
                .unwrap(),
            "5"
        );
        assert!(runtime
            .eval_text("(promise/delay -1 (fn [] 1))")
            .unwrap_err()
            .contains("non-negative"));
        assert!(runtime
            .eval_text("(promise/new 1)")
            .unwrap_err()
            .contains("expects a function"));
    }
    #[test]
    fn promise_continuations_preserve_registration_order_and_late_delivery() {
        let promise = core::Promise::new();
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let first = events.clone();
        promise.on_settle(std::rc::Rc::new(move |_| first.borrow_mut().push(1)));
        let second = events.clone();
        promise.on_settle(std::rc::Rc::new(move |_| second.borrow_mut().push(2)));
        assert!(promise.resolve(core::Value::Number(7)));
        assert_eq!(*events.borrow(), vec![1, 2]);
        let late = events.clone();
        promise.on_settle(std::rc::Rc::new(move |_| late.borrow_mut().push(3)));
        assert_eq!(*events.borrow(), vec![1, 2, 3]);
        assert!(!promise.reject("late"));
    }

    #[test]
    fn promises_settle_once_and_adopt() {
        let pending = core::Promise::new();
        let adopted = core::Promise::new();
        assert_eq!(pending.state(), core::PromiseState::Pending);
        assert!(adopted.adopt(&pending));
        assert!(pending.resolve(core::Value::Number(7)));
        assert!(!pending.reject("late"));
        assert_eq!(
            adopted.state(),
            core::PromiseState::Fulfilled(core::Value::Number(7))
        );
    }

    #[test]
    fn marker_mutation_methods_cover_array_and_object_boundaries() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(let (a (array 2)) (do (Arr/push-first a 1) (Arr/push-last a 3) (Arr/insert a 1 9) (Arr/get a 1)))").unwrap(), "9");
        assert_eq!(
            runtime
                .eval_text(
                    "(let (a (array 1 2)) (do (Arr/pop-first a) (Arr/pop-last a) (count a)))"
                )
                .unwrap(),
            "0"
        );
        assert_eq!(
            runtime
                .eval_text(r#"(Obj/keys (object "a" 1 "b" 2))"#)
                .unwrap(),
            r#"(array "a" "b")"#
        );
        assert_eq!(
            runtime
                .eval_text(r#"(Obj/vals (object "a" 1 "b" 2))"#)
                .unwrap(),
            "(array 1 2)"
        );
        assert_eq!(
            runtime
                .eval_text(
                    r#"(let (o (object "a" 1)) (do (Obj/assign o (object "b" 2)) (Obj/get o "b")))"#
                )
                .unwrap(),
            "2"
        );
    }

    #[test]
    fn marker_static_contract_covers_results_identity_callbacks_and_rejections() {
        let mut runtime = Runtime::new();
        let cases = [
            ("(Arr/get (Arr/map (array 1 2 3) (fn [x] (* x 2))) 2)", "6"),
            (
                "(Arr/get (Arr/filter (array 1 2 3 4) (fn [x] (> x 2))) 0)",
                "3",
            ),
            ("(Arr/get (Arr/slice (array 1 2 3) 1) 1)", "3"),
            (
                "(Arr/fold-left (array 1 2 3) (fn [out x] (- out x)) 0)",
                "-6",
            ),
            (
                "(Arr/fold-right (array 1 2 3) (fn [x out] (- x out)) 0)",
                "2",
            ),
            ("(let [a (array 1)] (= a (Arr/push-last a 2)))", "true"),
            ("(let [a (array 1)] (= a (Arr/set a 0 2)))", "true"),
            ("(let [a (array 1)] (= a (Arr/insert a 1 2)))", "true"),
            ("(let [a (array 1)] (= a (Arr/clone a)))", "false"),
            (
                r#"(let [o (object "a" 1)] (= o (Obj/set o "a" 2)))"#,
                "true",
            ),
            (r#"(Obj/delete (object "a" 1) "a")"#, "1"),
            (r#"(Obj/delete (object "a" 1) "missing")"#, "nil"),
            (r#"(Arr/get (Obj/keys (object "a" 1)) 0)"#, r#""a""#),
            (r#"(Arr/get (Arr/get (Obj/pairs (object "a" 1)) 0) 1)"#, "1"),
            ("(iter-next (iter (array 7 8)))", "7"),
            (r#"(second (iter-next (iter (object "a" 7))))"#, "7"),
        ];
        for (source, expected) in cases {
            assert_eq!(runtime.eval_text(source).unwrap(), expected, "{source}");
        }

        let invalid = [
            ("(. [1 2] (get 0))", "array or object marker"),
            ("(. {} (get \"a\"))", "array or object marker"),
            ("(. 1 (get 0))", "array or object marker"),
            ("(Arr/unknown (array 1))", "unknown std.native.Arr method"),
            (
                r#"(Obj/unknown (object "a" 1))"#,
                "unknown std.native.Obj method",
            ),
            ("(Arr/set (array 1) 0)", "expects an index and value"),
            ("(Arr/clone (array 1) 1)", "expects no arguments"),
            (r#"(Obj/clone (object "a" 1) 1)"#, "expects no arguments"),
            (
                "(Arr/map (array 1) (fn [x y] x))",
                "function expects 2 arguments",
            ),
            ("(x:array 1)", "unbound symbol: x:array"),
            ("(x:object)", "unbound symbol: x:object"),
            ("(x:get nil 0)", "unbound symbol: x:get"),
            ("(x:set nil 0 1)", "unbound symbol: x:set"),
            (
                r#"(host-symbol "java.lang.String")"#,
                "unbound symbol: host-symbol",
            ),
            (r#"(host-get nil "value")"#, "unbound symbol: host-get"),
            (r#"(host-call nil "run")"#, "unbound symbol: host-call"),
        ];
        for (source, message) in invalid {
            assert!(
                runtime.eval_text(source).unwrap_err().contains(message),
                "{source}"
            );
        }
    }
    #[test]
    fn marker_arrays_and_objects_use_static_native_calls() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(count (array 1 2 3))").unwrap(), "3");
        assert_eq!(runtime.eval_text("(Arr/get (array 1 2) 1)").unwrap(), "2");
        assert_eq!(
            runtime
                .eval_text("(let (a (array 1 2)) (do (Arr/set a 1 7) (Arr/get a 1)))")
                .unwrap(),
            "7"
        );
        assert_eq!(
            runtime
                .eval_text("(let (a (array 1)) (do (Arr/push-last a 2) (count a)))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text(r#"(Obj/get (object "answer" 41) "answer")"#)
                .unwrap(),
            "41"
        );
        assert_eq!(
            runtime
                .eval_text(
                    r#"(let (o (object)) (do (Obj/set o "answer" 42) (Obj/get o "answer")))"#
                )
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text(r#"(Obj/has? (object "answer" 41) "answer")"#)
                .unwrap(),
            "true"
        );
        assert!(runtime
            .eval_text("(. (array 1) (get 0))")
            .unwrap_err()
            .contains("use Arr/ or Obj/ functions"));
    }

    #[test]
    fn array_and_object_native_types_are_available_without_foundation_bootstrap() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(let [a (Arr/new 1 2) o (Obj/new \"answer\" 41)] \
                       (Arr/set a 1 7) \
                       (Obj/set o \"answer\" 42) \
                       [(Arr/get a 1) (Obj/get o \"answer\")])",
                )
                .unwrap(),
            "[7 42]"
        );
    }

    #[test]
    fn strings_and_bytes_support_utf8_copy_and_slice() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text(r#"(str "hello" " " "world")"#).unwrap(),
            "\"hello world\""
        );
        assert_eq!(runtime.eval_text(r#"(str/length "a😀b")"#).unwrap(), "3");
        assert_eq!(
            runtime.eval_text(r#"(type (str/char-at "a" 0))"#).unwrap(),
            ":std.native.Character"
        );
        assert_eq!(
            runtime.eval_text(r#"(str/char-at " " 0)"#).unwrap(),
            "\\space"
        );
        assert_eq!(
            runtime.eval_text(r#"(str/char-at "a😀b" 1)"#).unwrap(),
            "\\😀"
        );
        assert!(runtime
            .eval_text(r#"(str/char-at "a" 1)"#)
            .unwrap_err()
            .contains("str/char-at index out of bounds"));
        assert_eq!(
            runtime.eval_text(r#"(str/slice "a😀b" 1 2)"#).unwrap(),
            "\"😀\""
        );
        assert_eq!(
            runtime.eval_text(r#"(str/index-of "a😀b" "b")"#).unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text(r#"(str/last-index-of "😀a😀" "😀")"#)
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime.eval_text(r#"(str/pad-left "x" 3 "😀")"#).unwrap(),
            "\"😀😀x\""
        );
        assert_eq!(
            runtime.eval_text(r#"(str/trim "  hara  ")"#).unwrap(),
            "\"hara\""
        );
        assert_eq!(
            runtime
                .eval_text(r#"(str/decode-utf8 (str/encode-utf8 "hé"))"#)
                .unwrap(),
            "\"hé\""
        );
        assert_eq!(
            runtime
                .eval_text("(bytes/slice (bytes 1 2 3) 1 3)")
                .unwrap(),
            "(bytes 2 3)"
        );
        assert_eq!(runtime.eval_text("(let (source (bytes 1 2)) (let (copy (bytes/copy source)) (do (bytes/set copy 0 9) (bytes/get source 0))))").unwrap(), "1");
    }

    #[test]
    fn byte_buffers_preserve_signed_storage_and_unsigned_reads() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(bytes 1 2 -3)").unwrap(),
            "(bytes 1 2 -3)"
        );
        assert_eq!(
            runtime.eval_text("(bytes/get (bytes 1 2 -3) 2)").unwrap(),
            "253"
        );
        assert_eq!(runtime.eval_text("(bytes/u8 -1)").unwrap(), "255");
        assert_eq!(runtime.eval_text("(bytes/s8 255)").unwrap(), "-1");
        assert_eq!(
            runtime
                .eval_text("(let (b (bytes 1 2)) (do (bytes/set b 0 9) (bytes/get b 0)))")
                .unwrap(),
            "9"
        );
        assert_eq!(runtime.eval_text("(bytes/get (bytes 1) 4 7)").unwrap(), "7");
        assert_eq!(
            runtime.eval_text("(bytes/count (bytes 1 2 -3))").unwrap(),
            "3"
        );
    }

    #[test]
    fn bytes_and_bits_cover_conversion_copy_and_overflow_boundaries() {
        let mut runtime = Runtime::new();
        let cases = [
            ("(bytes/u8 -128)", "128"),
            ("(bytes/u8 255)", "255"),
            ("(bytes/s8 -128)", "-128"),
            ("(bytes/s8 128)", "-128"),
            ("(bytes/s8 255)", "-1"),
            ("(bytes/get (bytes -128 0 127 255) 0)", "128"),
            ("(bytes/get (bytes -128 0 127 255) 3)", "255"),
            ("(bytes/slice (bytes 1 2 3) 1)", "(bytes 2 3)"),
            ("(bytes/slice (bytes 1 2 3) 1 1)", "(bytes)"),
            (
                "(let [b (bytes 0)] (count [(bytes/set b 0 255) (bytes/get b 0)]))",
                "2",
            ),
            ("(bit-not -2147483648)", "2147483647"),
            ("(bit-not 2147483647)", "-2147483648"),
            ("(bit-and -2147483648 2147483647)", "0"),
            ("(bit-or -2147483648 1)", "-2147483647"),
            ("(bit-xor -1 2147483647)", "-2147483648"),
            ("(bit-shift-left 1 0)", "1"),
            ("(bit-shift-left 1 31)", "2147483648"),
            ("(bit-shift-left 2147483647 1)", "4294967294"),
            ("(bit-shift-right -2147483648 31)", "-1"),
            ("(bit-shift-right 2147483647 31)", "0"),
            ("(bit-shift-left 2147483648 0)", "2147483648"),
            (
                "(bit-shift-right (bit-shift-left 1 80) 32)",
                "281474976710656",
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(runtime.eval_text(source).unwrap(), expected, "{source}");
        }

        let invalid = [
            ("(bytes -129)", "range -128..255"),
            ("(bytes 256)", "range -128..255"),
            ("(bytes/u8 -129)", "range -128..255"),
            ("(bytes/s8 256)", "range -128..255"),
            ("(bytes/get (bytes 1) 1)", "out of bounds"),
            ("(bytes/set (bytes 1) 1 0)", "out of bounds"),
            ("(bytes/slice (bytes 1 2) 2 1)", "out of bounds"),
            ("(bytes/slice (bytes 1 2) 0 3)", "out of bounds"),
            ("(str/decode-utf8 (bytes 255))", "invalid UTF-8"),
            ("(bit-shift-left 1 -1)", "non-negative"),
        ];
        for (source, message) in invalid {
            assert!(
                runtime.eval_text(source).unwrap_err().contains(message),
                "{source}"
            );
        }

        assert_eq!(
            runtime
                .eval_text("(let [source (bytes 1 2 3) copy (bytes/copy source)] (do (bytes/set copy 0 9) (bytes/get source 0)))")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text("(let [source (bytes 1 2 3) part (bytes/slice source 0 2)] (do (bytes/set part 0 9) (bytes/get source 0)))")
                .unwrap(),
            "1"
        );
    }
    #[test]
    fn iterator_aliases_and_combinators_match_core_shapes() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(nth (map (fn [x] (* x 2)) [1 2]) 0)")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(nth (filter (fn [x] (= x 2)) [1 2 3]) 0)")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(nth (take 1 (drop 1 [1 2 3])) 0)")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime.eval_text("(nth (nth (zip [1] [2]) 0) 1)").unwrap(),
            "2"
        );
        assert_eq!(
            runtime.eval_text("(first (drop 2 (cycle [1 2])))").unwrap(),
            "1"
        );
        assert_eq!(runtime.eval_text("(first (concat [1] [2]))").unwrap(), "1");
    }

    #[test]
    fn seq_boundaries_and_source_aware_transforms_match_design() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(vector? (map inc [1 2 3]))").unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(first (map inc [1 2 3]))").unwrap(), "2");
        assert_eq!(
            runtime.eval_text("(first ((map inc) [1 2 3]))").unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(first ((map inc) (seq [1 2 3])))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(first (seq (map inc) [1 2 3]))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(first ((comp (map inc) (map inc)) [1 2 3]))")
                .unwrap(),
            "3"
        );
        assert_eq!(
            runtime
                .eval_text("(first (seq (comp (map inc) (map inc)) [1 2 3]))")
                .unwrap(),
            "3"
        );
    }

    #[test]
    fn issue_200_nil_terminates_sequences_and_iterator_lookahead_is_exact() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(nil? (seq nil)) \
                      (nil? (seq [])) \
                      (nil? (rest nil)) \
                      (nil? (rest [])) \
                      (nil? (rest [1])) \
                      (seq? (rest [1 2])) \
                      (vec (rest [1])) \
                      (vec (rest [1 2]))]"
                )
                .unwrap(),
            "[true true true true true true [] [2]]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let [it (Iter/iter-drop 1 [1])] \
                       [(iter-next? it) (iter-next? it) (nil? (seq it))])"
                )
                .unwrap(),
            "[false false true]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let [it (Iter/iter-map inc [1])] \
                       [(iter-next? it) (iter-next? it) (iter-next it) (iter-next? it)])"
                )
                .unwrap(),
            "[true true 2 false]"
        );
    }

    #[test]
    fn issue_200_finite_generated_iterators_materialize_and_failures_propagate() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(last (Iter/iter-take 2 [1 2 3])) \
                      (vec (reverse (Iter/iter-take 2 [1 2 3]))) \
                      (vec (Iter/iter-zip [1] (repeat 0))) \
                      (vec (Iter/iter-interleave [1] (repeat 0)))]"
                )
                .unwrap(),
            "[2 [2 1] [[1 0]] [1 0]]"
        );
        assert!(runtime
            .eval_text("(count (Iter/iter-map (fn [x] (throw \"boom\")) [1]))")
            .unwrap_err()
            .contains("boom"));
        assert!(runtime
            .eval_text("(count (Iter/iter-map (fn [x] (throw \"weekend\")) [1]))")
            .unwrap_err()
            .contains("weekend"));
        assert_eq!(
            runtime
                .eval_text(
                    "[(seq? (seq [1])) \
                      (iter? (seq [1])) \
                      (vec (cons 0 (rest [1 2]))) \
                      (vec (Iter/iter-take 4 (cons 0 (repeat 1))))]"
                )
                .unwrap(),
            "[true false [0 2] [0 1 1 1]]"
        );
        assert!(runtime
            .eval_text("(cycle [])")
            .unwrap_err()
            .contains("cycle expects a non-empty source"));
    }

    #[test]
    fn seq_is_a_reusable_frozen_view_with_independent_iterators() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(let [xs (seq [1 2 3])] \
                       [(seq? xs) (iter? xs) (first xs) (first xs) \
                        (first (rest xs)) (vec xs) (vec xs)])"
                )
                .unwrap(),
            "[true false 1 1 2 [1 2 3] [1 2 3]]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let [xs (seq [1 2 3]) left (iter xs) right (iter xs)] \
                       [(iter-next left) (iter-next right) \
                        (iter-next left) (iter-next right)])"
                )
                .unwrap(),
            "[1 1 2 2]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let [calls (atom 0) \
                           xs (seq (Iter/iter-map \
                             (fn [value] (do (swap! calls inc) value)) [1 2]))] \
                       [(deref calls) (first xs) (deref calls)
                        (first xs) (deref calls)])"
                )
                .unwrap(),
            "[1 1 1 1 1]"
        );
        assert_eq!(
            runtime.eval_text("(seq (Iter/iter-range 20))").unwrap(),
            "(0 1 2 3 4 5 6 7 8 9 ...)"
        );
    }

    #[test]
    fn iterators_are_closeable_and_support_map_filter() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(let (it (iter [1 2])) (iter-next it))")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text("(let (it (iter [1 2])) (do (iter-next it) (iter-next it)))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime.eval_text("(iter-next? (iter [1]))").unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(let (it (iter [1])) (do (Iter/iter-close it) (iter-next? it)))")
                .unwrap(),
            "false"
        );
        assert_eq!(
            runtime
                .eval_text("(let (it (Iter/iter-cycle [1 2])) (do (iter-next it) (Iter/iter-close it) (iter-next? it)))")
                .unwrap(),
            "false"
        );
        assert_eq!(
            runtime
                .eval_text("(let (it (Iter/iter-zip [1 2] [3 4])) (do (Iter/iter-close it) (iter-next? it)))")
                .unwrap(),
            "false"
        );
        assert_eq!(
            runtime
                .eval_text("(iter-next (Iter/iter-map (fn [x] (* x 2)) [1 2]))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(iter-next (Iter/iter-filter (fn [x] (= x 2)) [1 2 3]))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(receiver-category (iter [1]))")
                .unwrap_err(),
            "unbound symbol: receiver-category"
        );
    }

    #[test]
    fn evaluator_protocol_calls_cover_collections_and_bytes() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(ICount/count [1 2 3])").unwrap(), "3");
        assert_eq!(
            runtime.eval_text("(INth/nth (bytes 1 -3) 1)").unwrap(),
            "-3"
        );
        assert_eq!(
            runtime
                .eval_text(r#"(ILookup/lookup {"a" 9} "a")"#)
                .unwrap(),
            "9"
        );
        assert_eq!(
            runtime.eval_text(r#"(has? {"a" nil} "a")"#).unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(has? [10 20] 1)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(has? [10 20] 10)").unwrap(), "false");
        assert_eq!(
            runtime
                .eval_text(r#"(IAssoc/assoc {"a" 9} "b" 10)"#)
                .unwrap(),
            r#"{"a" 9 "b" 10}"#
        );
        assert_eq!(
            runtime.eval_text("(IAssoc/assoc [1 2 3] 1 9)").unwrap(),
            "[1 9 3]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(IReduce/reduce (seq [1 2 3]) (fn [left right] (+ left right)) 0)",
                )
                .unwrap(),
            "6"
        );
        assert_eq!(
            runtime.eval_text("(IFind/find (seq [10 20]) 1)").unwrap(),
            "[1 20]"
        );
        assert_eq!(runtime.eval_text(r#"(IConj/conj [1] 2)"#).unwrap(), "[1 2]");
        assert_eq!(
            runtime
                .eval_text(r#"(IDissoc/dissoc {"a" 9 "b" 10} "a")"#)
                .unwrap(),
            r#"{"b" 10}"#
        );
        runtime.protocols.register_when(
            "IIter",
            "iter",
            |value| matches!(value, core::Value::Number(99)),
            protocol_custom_iterator,
        );
        assert_eq!(runtime.eval_text("(iter-next (iter 99))").unwrap(), "7");
        assert!(runtime.has_protocol_method("IAssoc", "assoc"));
        assert!(runtime
            .eval_text("(ICount/count 1)")
            .unwrap_err()
            .contains("protocol/unsupported-receiver"));
    }

    #[test]
    fn portable_type_descriptors_cover_named_and_collection_values() {
        let mut runtime = Runtime::new();
        runtime.eval_text("(ns type.runtime)").unwrap();
        for (source, expected) in [
            ("nil", ":std.native.Nil"),
            ("1", ":std.native.Long"),
            ("9223372036854775808", ":std.native.BigInteger"),
            (":key", ":std.native.Keyword"),
            ("(symbol \"hara/name\")", ":std.native.Symbol"),
            ("[]", ":std.native.Tuple"),
            ("(list)", ":std.native.List"),
            ("(std.native.Algo/queue)", ":std.native.Queue"),
            ("(vector)", ":std.native.Vector"),
            ("(hash-map)", ":std.native.HashMap"),
            ("{}", ":std.native.HashMap"),
            ("(std.native.Algo/sorted-map)", ":std.native.SortedMap"),
            ("(std.native.Algo/trie)", ":std.native.Trie"),
            ("(hash-set)", ":std.native.HashSet"),
            ("#{}", ":std.native.OrderedSet"),
            ("(std.native.Algo/sorted-set)", ":std.native.SortedSet"),
            ("(bytes)", ":std.native.ByteBuffer"),
            ("(array)", ":std.native.Array"),
            ("(object)", ":std.native.Object"),
            ("(atom 0)", ":std.native.Atom"),
            ("(ns-create (quote example))", ":std.native.Namespace"),
            ("#\"x\"", ":std.native.RegExp"),
        ] {
            assert_eq!(
                runtime.eval_text(&format!("(type {source})")).unwrap(),
                expected
            );
        }
        assert_eq!(
            runtime.eval_text("(type (type []))").unwrap(),
            ":std.native.Keyword"
        );
        assert_eq!(
            runtime
                .eval_text("[(type [1 2 3 4 5 6 7 8]) (type [1 2 3 4 5 6 7 8 9])]")
                .unwrap(),
            "[:std.native.Tuple :std.native.Vector]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(ns geometry) (defstruct Point [x y]) (defmutable Cursor [x y]) \
                     [(type (Point 1 2)) (type (Cursor 1 2)) (type Point) (type Cursor)]",
                )
                .unwrap(),
            "[:geometry.Point :geometry.Cursor :std.native.StructType :std.native.MutableType]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "[(vector? []) (tuple? []) (pair? [1 2]) \
                     (tuple? [1 2 3 4 5 6 7 8]) (tuple? [1 2 3 4 5 6 7 8 9]) \
                     (vector? [1 2 3 4 5 6 7 8 9]) (pair? (vector 1 2)) \
                     (pair? (list 1 2))]",
                )
                .unwrap(),
            "[true true true true false true false false]"
        );
        assert!(runtime
            .eval_text("(type)")
            .unwrap_err()
            .contains("one value"));
    }

    #[test]
    fn typed_schema_values_separate_data_origins_and_var_contracts() {
        let mut runtime = Runtime::core();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns schema.runtime) \
                     (def description [:int]) \
                     (defn ^{:schema #'description} customer-name [customer] (:name customer)) \
                     (def snapshot-description [:int]) \
                     (defn ^{:schema #'snapshot-description} snapshot-name [customer] (:name customer)) \
                     (def snapshot-description [:string]) \
                     (let [from-var (std.native.Schema/compile #'description) \
                           from-value (std.native.Schema/compile description) \
                           direct (std.native.Schema/compile [:int])] \
                     [(schema? direct) (= from-var from-value direct) \
                        (schema? direct) (std.native.Schema/kind direct) \
                        (= #'description (std.native.Schema/origin from-var)) \
                        (= from-var (std.native.Schema/of #'customer-name)) \
                        (= direct (std.native.Schema/of #'snapshot-name)) \
                        (= direct (std.native.Schema/compile {:kind :primitive :children [:int]})) \
                        (= [:int] (std.native.Schema/form direct)) \
                        (map? (std.native.Schema/ast direct)) \
                        (= direct (std.native.Schema/compile direct)) \
                        (= direct (std.native.Schema/compile :int)) \
                        (nil? (std.native.Schema/of #'description))])",
                )
                .unwrap(),
            "[true true true :primitive true true true true true true true true true]"
        );
        assert!(runtime
            .eval_text("(std.native.Schema/compile #'customer-name)")
            .is_err());
        assert!(runtime
            .eval_text("(std.native.Schema/compile customer-name)")
            .is_err());
        assert!(runtime
            .eval_text("(std.native.Schema/of customer-name)")
            .is_err());
        assert_eq!(
            runtime
                .eval_bytecode_native(
                    "[(type (std.native.Schema/compile [:int])) \
                      (std.native.Schema/kind (std.native.Schema/compile [:int]))]",
                )
                .unwrap(),
            "[:std.native.SchemaType :primitive]"
        );
        assert_eq!(
            runtime
                .eval_bytecode_native(
                    "(def description [:int]) \
                     (defn ^{:schema #'description} customer-name [customer] (:name customer)) \
                     (def description [:string]) \
                     [(std.native.Schema/kind \
                        (std.native.Schema/of #'customer-name)) \
                      (= (std.native.Schema/of #'customer-name) \
                         (std.native.Schema/compile [:int]))]",
                )
                .unwrap(),
            "[:primitive true]"
        );
    }

    #[test]
    fn protocol_registry_dispatches_by_protocol_and_method() {
        let mut registry = core::ProtocolRegistry::new();
        registry.register_when(
            "IIdentity",
            "identity",
            |value| matches!(value, core::Value::Number(_)),
            protocol_identity,
        );
        assert!(core::ProtocolRegistry::core()
            .contains("std.protocol.iassoc.IAssoc", "assoc"));
        assert!(registry.contains("IIdentity", "identity"));
        assert_eq!(
            registry
                .invoke("IIdentity", "identity", &[core::Value::Number(7)])
                .unwrap(),
            core::Value::Number(7)
        );
        assert!(registry
            .invoke("IIdentity", "missing", &[])
            .unwrap_err()
            .contains("missing protocol method"));
        assert_eq!(
            core::receiver_category(&core::Value::Vector(Default::default())),
            "vector"
        );
    }

    #[test]
    fn protocol_registry_matchers_interlock_satisfaction_dispatch_and_overrides() {
        let mut registry = core::ProtocolRegistry::new();
        registry.register_when(
            "IProbe",
            "probe",
            |value| matches!(value, core::Value::Number(_)),
            |_| Ok(core::Value::Keyword("number".into())),
        );
        registry.register_when(
            "IProbe",
            "probe",
            |value| matches!(value, core::Value::String(_)),
            |_| Ok(core::Value::Keyword("string".into())),
        );
        let protocol = core::GuestProtocol {
            name: "IProbe".into(),
            methods: HashMap::from([("probe".into(), 1)]),
            parents: Vec::new(),
        };
        assert!(registry.satisfies(&protocol, &core::Value::Number(1)));
        assert!(registry.satisfies(&protocol, &core::Value::String("x".into())));
        assert!(!registry.satisfies(&protocol, &core::Value::Bool(true)));
        assert_eq!(
            registry
                .invoke("IProbe", "probe", &[core::Value::Number(1)])
                .unwrap(),
            core::Value::Keyword("number".into())
        );
        assert_eq!(
            registry
                .invoke("IProbe", "probe", &[core::Value::String("x".into())])
                .unwrap(),
            core::Value::Keyword("string".into())
        );
        registry.register_when(
            "IProbe",
            "probe",
            |value| matches!(value, core::Value::Number(_)),
            |_| Ok(core::Value::Keyword("override".into())),
        );
        assert_eq!(
            registry
                .invoke("IProbe", "probe", &[core::Value::Number(1)])
                .unwrap(),
            core::Value::Keyword("override".into())
        );
    }

    #[test]
    fn functions_support_variadic_rest_parameters() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("((fn [x & rest] (+ x (count rest))) 40 1 2)")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text("(do (defn collect [x & rest] (count rest)) (collect 1 2 3 4))")
                .unwrap(),
            "3"
        );
        assert!(runtime
            .eval_text("((fn [x & rest] x))")
            .unwrap_err()
            .contains("at least 1"));
    }

    #[test]
    fn underscore_bindings_discard_values_and_may_repeat() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[((fn [_ _] 42) 1 2) \
                     (let [_ 1 _ 2] 42) \
                     (let [[_ _] [1 2]] 42) \
                     (let [{_ :ignored} {:ignored 1}] 42)]",
                )
                .unwrap(),
            "[42 42 42 42]"
        );
    }

    #[test]
    fn issue_133_cases_run_from_the_shared_core_language_conformance_corpus() {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> &'a Form {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing :{key}"))
        }

        let Some(corpus) = repo_text("01-lang/001-language/draft/conformance/core.edn") else {
            return;
        };
        let manifest = kernel::parse_forms(&corpus).unwrap().remove(0);
        let Form::Map(manifest) = manifest else {
            panic!("Core-language conformance corpus must be a map")
        };
        let Form::Vector(cases) = entry(&manifest, "cases") else {
            panic!("Core-language conformance :cases must be a vector")
        };
        let ids = [
            "function/closure-capture",
            "function/fixed-arity",
            "function/variadic-arity",
            "function/multiple-arities",
            "function/arity-error",
            "binding/let-sequential",
            "binding/sequential-destructuring",
            "binding/destructuring-foundation-shadowing",
            "binding/map-destructuring",
            "binding/missing-destructuring",
            "binding/nil-map-default",
            "definition/doc-metadata",
            "definition/schema-metadata",
            "definition/arglists-metadata",
            "sequence/empty-is-nil",
            "sequence/non-empty-rest",
            "sequence/frozen-view",
            "sequence/lazy-cons",
            "sequence/reject-conj",
            "iterator/exact-lookahead",
            "iterator/generated-exhaustion",
            "iterator/shortest-source-finite",
            "iterator/nil-requires-conversion",
            "iterator/native-combinator-qualified",
            "iterator/root-combinator-unbound",
            "iterator/empty-cycle-rejected",
            "runtime/recur-outside-target",
            "runtime/recur-arity",
            "error/catch-exception-value",
            "error/catch-order",
            "error/catch-code",
            "error/catch-code-vector",
            "error/exception-message-fallback",
            "error/exception-provenance-line",
            "error/exception-provenance-throw-column",
            "error/exception-provenance-throw-count",
            "error/exception-provenance-rethrow-count",
            "error/exception-cause",
            "error/exception-reject-arbitrary-throw",
            "error/exception-reject-reserved-code",
            "error/unmatched-catch",
            "error/finally-normal",
            "error/finally-unwind",
            "namespace/config-intrinsics-all",
            "namespace/named-selects-definition-scope",
            "namespace/anonymous-reuses-current-scope",
            "namespace/anonymous-applies-config",
            "namespace/anonymous-config-override",
            "namespace/anonymous-config-expose",
            "namespace/anonymous-config-expose-omits-unlisted",
            "namespace/anonymous-config-intrinsic-alias",
            "namespace/anonymous-config-intrinsic-exclude",
            "namespace/config-blank-override-conflict",
            "namespace/config-override-expose-conflict",
            "namespace/config-blank-type",
            "namespace/anonymous-rejects-name",
            "namespace/builtins-are-not-vars-or-native-type",
            "namespace/config-exclude-and-alias",
            "namespace/config-exclude-removes-alias",
            "namespace/standalone-intrinsics-invalid",
            "namespace/standalone-builtins-invalid",
            "namespace/duplicate-config",
            "namespace/unknown-config-key",
            "namespace/unknown-intrinsics-option",
            "namespace/unknown-intrinsic-library",
            "namespace/exclude-alias-conflict",
            "namespace/alias-collision",
            "namespace/blank-suppresses-referral",
            "namespace/blank-keeps-special-forms-and-aliases",
            "namespace/builtins-config-is-internal",
        ];

        for id in ids {
            let case = cases
                .iter()
                .find(|case| {
                    matches!(
                        case,
                        Form::Map(entries)
                            if matches!(entry(entries, "id"), Form::Keyword(name) if name == id)
                    )
                })
                .unwrap_or_else(|| panic!("missing conformance case :{id}"));
            let Form::Map(case) = case else {
                unreachable!()
            };
            let Form::String(source) = entry(case, "source") else {
                panic!(":{id} source must be a string")
            };
            let Form::Map(expect) = entry(case, "expect") else {
                panic!(":{id} expect must be a map")
            };
            let mut runtime = Runtime::new();
            if expect
                .iter()
                .any(|(key, _)| matches!(key, Form::Keyword(name) if name == "error"))
            {
                assert!(runtime.eval_text(source).is_err(), ":{id} should fail");
            } else {
                let expected = match entry(expect, "value") {
                    Form::Number(value) => value.to_string(),
                    Form::BigInteger(value) => value.to_string(),
                    Form::String(value) => format!("{value:?}"),
                    Form::Bool(value) => value.to_string(),
                    Form::Keyword(value) => format!(":{value}"),
                    Form::Nil => "nil".to_owned(),
                    value => panic!(":{id} has unsupported expected value {value:?}"),
                };
                let actual = runtime
                    .eval_text(source)
                    .unwrap_or_else(|error| panic!(":{id} unexpectedly failed: {error}"));
                assert_eq!(actual, expected, ":{id}");
            }
        }
    }

    #[test]
    fn portable_exception_cases_run_from_the_shared_conformance_corpus() {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
        }

        let Some(corpus) = repo_text("01-lang/001-language/draft/conformance/exceptions.edn")
        else {
            return;
        };
        let manifest = kernel::parse_forms(&corpus).unwrap().remove(0);
        let Form::Map(manifest) = manifest else {
            panic!("Exception conformance corpus must be a map")
        };
        let Some(Form::Vector(cases)) = entry(&manifest, "cases") else {
            panic!("Exception conformance :cases must be a vector")
        };

        for case in cases {
            let Form::Map(case) = case else {
                panic!("Exception conformance cases must be maps")
            };
            let Some(Form::Keyword(id)) = entry(case, "id") else {
                panic!("Exception conformance case is missing :id")
            };
            let Some(Form::String(source)) = entry(case, "source") else {
                panic!(":{id} source must be a string")
            };
            let Some(Form::Map(expect)) = entry(case, "expect") else {
                panic!(":{id} expect must be a map")
            };
            let mut runtime = Runtime::new();
            if entry(expect, "error").is_some() {
                let error = match runtime.eval_text(source) {
                    Ok(value) => panic!(":{id} should fail, returned {value}"),
                    Err(error) => error,
                };
                if let Some(Form::String(message)) = entry(expect, "message") {
                    assert!(
                        error.contains(message),
                        ":{id} should contain {message:?}, actual: {error:?}"
                    );
                }
            } else {
                let expected = match entry(expect, "value") {
                    Some(Form::Number(value)) => value.to_string(),
                    value => panic!(":{id} has unsupported expected value {value:?}"),
                };
                let actual = runtime
                    .eval_text(source)
                    .unwrap_or_else(|error| panic!(":{id} unexpectedly failed: {error}"));
                assert_eq!(actual, expected, ":{id}");
            }
        }
    }

    #[test]
    fn issue_134_module_scenarios_have_machine_readable_acceptance_data() {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
        }

        let Some(corpus) = repo_text("01-lang/001-language/draft/conformance/modules.edn") else {
            return;
        };
        let manifest = kernel::parse_forms(&corpus).unwrap().remove(0);
        let Form::Map(manifest) = manifest else {
            panic!("module conformance corpus must be a map")
        };
        let Some(Form::Vector(cases)) = entry(&manifest, "cases") else {
            panic!("module conformance :cases must be a vector")
        };
        assert!(cases.len() >= 20);
        let mut ids = HashSet::new();
        for case in cases {
            let Form::Map(case) = case else {
                panic!("module conformance cases must be maps")
            };
            let Some(Form::Keyword(id)) = entry(case, "id") else {
                panic!("module conformance case is missing :id")
            };
            assert!(ids.insert(id.clone()), "duplicate module case :{id}");
            assert!(
                matches!(entry(case, "area"), Some(Form::Keyword(_))),
                ":{id}"
            );
            assert!(
                matches!(entry(case, "scenario"), Some(Form::Keyword(_))),
                ":{id}"
            );
            assert!(matches!(entry(case, "expect"), Some(Form::Map(_))), ":{id}");
        }
    }

    #[test]
    fn module_ns_require_reload_executes_shared_spec_fixture() {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
            entries.iter().find_map(|(candidate, value)| {
                matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
            })
        }

        let case = module_case("module/ns-require-reload");
        let Some(Form::Map(fixture)) = entry(&case, "fixture") else {
            panic!(":module/ns-require-reload must declare :fixture")
        };
        let Some(Form::Map(resource)) = entry(fixture, "resource") else {
            panic!("reload fixture must declare :resource")
        };
        let Some(Form::String(namespace)) = entry(resource, "namespace") else {
            panic!("reload resource must declare string :namespace")
        };
        let Some(Form::Map(revisions)) = entry(resource, "revisions") else {
            panic!("reload resource must declare :revisions")
        };
        let Some(Form::Vector(steps)) = entry(fixture, "steps") else {
            panic!("reload fixture must declare :steps")
        };

        let mut runtime = Runtime::new();
        for step in steps {
            let Form::Map(step) = step else {
                panic!("reload fixture steps must be maps")
            };
            let Some(Form::Keyword(operation)) = entry(step, "op") else {
                panic!("reload fixture step must declare :op")
            };
            match operation.as_str() {
                "resource/use" => {
                    let Some(Form::Keyword(revision)) = entry(step, "revision") else {
                        panic!(":resource/use must declare :revision")
                    };
                    let Some(Form::String(source)) = entry(revisions, revision) else {
                        panic!("missing reload resource revision :{revision}")
                    };
                    runtime.register_resource(namespace, source);
                }
                "eval" => {
                    let Some(Form::String(source)) = entry(step, "source") else {
                        panic!(":eval must declare string :source")
                    };
                    let Some(Form::Map(expect)) = entry(step, "expect") else {
                        panic!(":eval must declare :expect")
                    };
                    if let Some(Form::String(expected)) = entry(expect, "display") {
                        assert_eq!(
                            runtime.eval_text(source).unwrap_or_else(|error| {
                                panic!("shared reload eval failed for {source}: {error}")
                            }),
                            *expected
                        );
                    } else if matches!(entry(expect, "error"), Some(Form::Bool(true))) {
                        runtime
                            .eval_text(source)
                            .expect_err("shared reload eval must fail");
                    } else if let Some(Form::String(marker)) = entry(expect, "error-contains") {
                        let error = runtime
                            .eval_text(source)
                            .expect_err("shared reload eval must fail");
                        assert!(error.contains(marker), "{error}");
                    } else {
                        panic!("unsupported shared reload expectation")
                    }
                }
                "assert/revision" => {
                    let Some(Form::Number(expected)) = entry(step, "expect") else {
                        panic!(":assert/revision must declare numeric :expect")
                    };
                    assert_eq!(
                        runtime.namespace_registry.module_revision(&namespace),
                        *expected as u64
                    );
                }
                other => panic!("unsupported shared reload operation :{other}"),
            }
        }
    }

    #[test]
    fn callable_var_scenarios_execute_from_shared_spec() {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
            entries.iter().find_map(|(candidate, value)| {
                matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
            })
        }

        for id in [
            "namespace/callable-var-precedence",
            "namespace/callable-var-lexical-shadow",
            "namespace/callable-var-late-binding",
            "namespace/referred-var-shadowed",
        ] {
            let case = module_case(id);
            let Some(Form::String(setup)) = entry(&case, "setup") else {
                panic!(":{id} must declare string :setup")
            };
            let Some(Form::String(source)) = entry(&case, "source") else {
                panic!(":{id} must declare string :source")
            };
            let Some(Form::Map(expect)) = entry(&case, "expect") else {
                panic!(":{id} must declare :expect")
            };
            let mut runtime = Runtime::new();
            runtime
                .eval_text(setup)
                .unwrap_or_else(|error| panic!(":{id} setup failed: {error}"));
            if let Some(Form::String(expected)) = entry(expect, "display") {
                assert_eq!(
                    runtime
                        .eval_text(source)
                        .unwrap_or_else(|error| panic!(":{id} failed: {error}")),
                    *expected,
                    ":{id}"
                );
            } else if let Some(Form::String(marker)) = entry(expect, "error-contains") {
                let error = runtime
                    .eval_text(source)
                    .expect_err(&format!(":{id} must fail"));
                assert!(error.contains(marker), ":{id}: {error}");
            } else {
                panic!(":{id} has unsupported expectation")
            }
        }
    }

    #[test]
    fn issue_134_lazy_namespace_state_is_non_forcing_and_failure_is_sticky() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("lazy/non-forcing", "state"),
            Form::Keyword("unloaded".into())
        );
        assert_eq!(
            module_expect("lazy/non-forcing", "target-evaluated"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("lazy/qualified-force", "target-evaluations"),
            Form::Number(1)
        );
        assert_eq!(
            module_expect("lazy/failure-state", "state"),
            Form::Keyword("failed".into())
        );
        assert_eq!(
            module_expect("lazy/failure-state", "partial-state"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("lazy/explicit-retry", "ordinary-force-retries"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("lazy/explicit-retry", "reload-retries"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("module/reload-revision", "revision-increment"),
            Form::Number(1)
        );
        assert_eq!(
            module_expect("module/reload-rollback", "previous-revision-preserved"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("namespace/loading-state", "non-forcing"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("namespace/cross-namespace-alias-state", "owner-explicit"),
            Form::Bool(true)
        );

        let mut runtime = Runtime::new();
        runtime.register_resource(
            "example.lazy",
            "(ns example.lazy) (def observed-state (ns-state 'example.lazy)) (def answer 42)",
        );

        assert_eq!(
            runtime
                .eval_text("(require [example.lazy :as lazy :lazy true])")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime.eval_text("(ns-state 'example.lazy)").unwrap(),
            ":unloaded"
        );
        assert_eq!(
            runtime
                .eval_text("(get (ns-alias-state 'lazy) :state)")
                .unwrap(),
            ":unloaded"
        );
        assert_eq!(runtime.eval_text("lazy/answer").unwrap(), "42");
        assert_eq!(
            runtime.eval_text("lazy/observed-state").unwrap(),
            ":loading"
        );
        assert_eq!(
            runtime.eval_text("(ns-state 'example.lazy)").unwrap(),
            ":loaded"
        );
        assert_eq!(
            runtime.namespace_registry.module_revision("example.lazy"),
            1
        );

        runtime.register_resource("example.lazy", "(ns example.lazy) (def answer 43)");
        runtime
            .eval_text("(require [example.lazy :as lazy :reload true])")
            .unwrap();
        assert_eq!(runtime.eval_text("lazy/answer").unwrap(), "43");
        assert_eq!(
            runtime.namespace_registry.module_revision("example.lazy"),
            2
        );

        runtime.register_resource(
            "example.lazy",
            "(ns example.lazy) (def answer 99) (def reload-leaked-134 1) (throw :reload-failed)",
        );
        assert!(runtime
            .eval_text("(require [example.lazy :as lazy :reload true])")
            .is_err());
        assert_eq!(runtime.eval_text("lazy/answer").unwrap(), "43");
        assert_eq!(
            runtime.namespace_registry.module_revision("example.lazy"),
            2
        );
        assert_eq!(
            runtime.eval_text("(ns-state 'example.lazy)").unwrap(),
            ":loaded"
        );
        assert!(runtime
            .namespace_registry
            .find("example.lazy")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("reload-leaked-134"))
            .is_none());

        runtime.register_resource(
            "example.broken",
            "(ns example.broken) (def leaked 1) (throw :broken)",
        );
        runtime
            .eval_text("(require [example.broken :as broken :lazy true])")
            .unwrap();
        assert!(runtime.eval_text("broken/leaked").is_err());
        assert_eq!(
            runtime.eval_text("(ns-state 'example.broken)").unwrap(),
            ":failed"
        );
        assert_eq!(
            runtime
                .eval_text("(get (ns-alias-state 'broken) :state)")
                .unwrap(),
            ":failed"
        );
        assert!(runtime.namespace_registry.find("example.broken").is_none());
        assert_eq!(runtime.current_namespace(), "user");
        assert_eq!(
            runtime
                .namespace_registry
                .current()
                .lazy_target("broken")
                .map(|name| name.as_str().to_owned()),
            Some("example.broken".into())
        );

        runtime.register_resource("example.broken", "(ns example.broken) (def answer 42)");
        let sticky_error = runtime.eval_text("broken/answer").unwrap_err();
        assert!(
            sticky_error.contains("explicit reload"),
            "unexpected sticky lazy-load error: {sticky_error}"
        );
        assert!(
            sticky_error.contains("initial failure"),
            "sticky error should retain the initial failure detail: {sticky_error}"
        );
        runtime
            .eval_text("(require [example.broken :as broken :reload true])")
            .unwrap();
        assert_eq!(runtime.eval_text("broken/answer").unwrap(), "42");
        assert_eq!(
            runtime.eval_text("(ns-state 'example.broken)").unwrap(),
            ":loaded"
        );

        runtime.eval_text("(ns observer)").unwrap();
        assert_eq!(
            runtime
                .eval_text("(get (ns-alias-state 'user 'broken) :state)")
                .unwrap(),
            ":loaded"
        );

        let mut isolated = Runtime::new();
        assert_eq!(
            isolated.eval_text("(ns-state 'example.lazy)").unwrap(),
            ":unknown"
        );
    }

    #[test]
    fn issue_134_dependency_order_cycles_and_canonical_cache_are_transactional() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("module/canonical-cache", "duplicate-evaluation"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("module/dependency-order", "order"),
            Form::Keyword("dependency-first-source-order".into())
        );
        assert_eq!(
            module_expect("module/cycle-rollback", "partial-state"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("module/failure-rollback", "revision-increment"),
            Form::Bool(false)
        );

        let mut runtime = Runtime::new();
        runtime.register_resource("graph.dependency", "(ns graph.dependency) (def value 41)");
        runtime.register_resource(
            "graph.root",
            concat!(
                "(ns graph.root) ",
                "(require [graph.dependency :as dependency]) ",
                "(def answer (+ dependency/value 1))"
            ),
        );

        runtime
            .eval_text("(require [graph.root :as graph])")
            .unwrap();
        assert_eq!(runtime.eval_text("graph/answer").unwrap(), "42");
        assert_eq!(
            runtime
                .namespace_registry
                .module_revision("graph.dependency"),
            1
        );
        assert_eq!(runtime.namespace_registry.module_revision("graph.root"), 1);
        assert_eq!(
            runtime
                .namespace_registry
                .module_dependencies("graph.root")
                .iter()
                .map(|name| name.as_str().to_owned())
                .collect::<Vec<_>>(),
            vec!["graph.dependency"]
        );

        runtime
            .eval_text("(require [graph.root :as graph])")
            .unwrap();
        assert_eq!(runtime.namespace_registry.module_revision("graph.root"), 1);

        runtime.register_resource(
            "cycle.first",
            concat!(
                "(ns cycle.first) ",
                "(def leaked-first 1) ",
                "(require [cycle.second :as second])"
            ),
        );
        runtime.register_resource(
            "cycle.second",
            concat!(
                "(ns cycle.second) ",
                "(def leaked-second 2) ",
                "(require [cycle.first :as first])"
            ),
        );

        let cycle = runtime
            .eval_text("(require [cycle.first :as cycle])")
            .unwrap_err();
        assert!(cycle.contains("Cyclic namespace require"), "{cycle}");
        assert!(runtime.namespace_registry.find("cycle.first").is_none());
        assert!(runtime.namespace_registry.find("cycle.second").is_none());
        assert_eq!(runtime.namespace_registry.module_revision("cycle.first"), 0);
        assert_eq!(
            runtime.namespace_registry.module_revision("cycle.second"),
            0
        );
        assert!(runtime
            .namespace_registry
            .module_dependencies("cycle.first")
            .is_empty());
        assert!(runtime
            .namespace_registry
            .module_dependencies("cycle.second")
            .is_empty());

        runtime.register_resource(
            "failure.root",
            concat!(
                "(ns failure.root) ",
                "(require [graph.dependency :as dependency]) ",
                "(def leaked dependency/value) ",
                "(throw :failure)"
            ),
        );
        assert!(runtime
            .eval_text("(require [failure.root :as failure])")
            .is_err());
        assert!(runtime.namespace_registry.find("failure.root").is_none());
        assert!(runtime
            .namespace_registry
            .module_dependencies("failure.root")
            .is_empty());
    }

    #[test]
    fn issue_134_with_ns_uses_target_globals_and_restores_the_caller() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("namespace/with-ns-success", "caller-restored"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("namespace/with-ns-failure", "caller-restored"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect(
                "namespace/with-ns-lexical-isolation",
                "caller-locals-visible"
            ),
            Form::Bool(false)
        );

        let mut runtime = Runtime::new();
        runtime
            .eval_text("(ns target) (def answer 41) (ns user)")
            .unwrap();
        assert_eq!(
            runtime
                .eval_text("(with-ns 'target (def answer 42) answer)")
                .unwrap(),
            "42"
        );
        assert_eq!(runtime.current_namespace(), "user");
        assert_eq!(runtime.eval_text("target/answer").unwrap(), "42");

        assert!(runtime
            .eval_text("(with-ns 'target (throw :with-ns-failed))")
            .is_err());
        assert_eq!(runtime.current_namespace(), "user");

        assert!(runtime
            .eval_text("(let [caller-local 42] (with-ns 'target caller-local))")
            .is_err());
        assert_eq!(runtime.current_namespace(), "user");
    }

    #[test]
    fn issue_134_facade_vars_copy_roots_and_metadata_without_sharing_identity() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("namespace/facade-var-copy", "same-var"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("namespace/facade-var-copy", "copied-root"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("namespace/facade-var-copy", "copied-metadata"),
            Form::Bool(true)
        );

        let mut runtime = Runtime::new();
        runtime
            .eval_text("(ns source) (def ^{:doc \"copied\"} answer 41)")
            .unwrap();
        runtime.eval_text("(ns target)").unwrap();
        assert_eq!(
            runtime.eval_text("(deref (var source/answer))").unwrap(),
            "41"
        );
        runtime
            .eval_text("(intern-var 'target 'answer (var source/answer))")
            .unwrap();
        let source = runtime
            .namespace_registry
            .find("source")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("answer"))
            .unwrap();
        let target = runtime
            .namespace_registry
            .find("target")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("answer"))
            .unwrap();

        assert!(!source.same_identity(&target));
        assert_eq!(source.deref_value(), target.deref_value());
        assert_eq!(source.metadata(), target.metadata());
    }

    #[test]
    fn issue_134_aliases_and_refers_share_live_var_identity() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("namespace/alias-var-identity", "same-var"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("namespace/alias-var-identity", "live-root"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("namespace/refer-var-identity", "same-var"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("namespace/refer-var-identity", "live-root"),
            Form::Bool(true)
        );

        let mut runtime = Runtime::new();
        runtime.register_resource("identity.source", "(ns identity.source) (def answer 41)");
        runtime
            .eval_text("(require [identity.source :as source :refer [answer]])")
            .unwrap();
        let source = runtime
            .namespace_registry
            .find("identity.source")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("answer"))
            .unwrap();
        let alias = runtime
            .namespace_registry
            .resolve(&crate::lang::data::Symbol::parse("source/answer"))
            .unwrap();
        let referred = runtime
            .namespace_registry
            .find("user")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("answer"))
            .unwrap();
        assert!(source.same_identity(&alias));
        assert!(source.same_identity(&referred));
        source.reset_value(core::Value::Number(42));
        assert_eq!(runtime.eval_text("source/answer").unwrap(), "42");
        assert_eq!(runtime.eval_text("answer").unwrap(), "42");
    }

    #[test]
    fn issue_134_macro_reload_only_changes_new_compilations() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("macro/reload-new-compilation", "existing-call-target"),
            Form::Keyword("unchanged".into())
        );
        assert_eq!(
            module_expect("macro/reload-new-compilation", "new-compilation"),
            Form::Keyword("new-expansion".into())
        );

        let mut runtime = Runtime::new();
        runtime.register_resource(
            "reload.macros",
            "(ns reload.macros) (defmacro answer [] 41)",
        );
        runtime
            .eval_text(
                "(require [reload.macros :refer-macros [answer]]) \
                 (def compiled-before (macroexpand '(answer)))",
            )
            .unwrap();
        assert_eq!(runtime.eval_text("compiled-before").unwrap(), "41");

        runtime.register_resource(
            "reload.macros",
            "(ns reload.macros) (defmacro answer [] 42)",
        );
        runtime
            .eval_text("(require [reload.macros :reload true :refer-macros [answer]])")
            .unwrap();
        assert_eq!(runtime.eval_text("compiled-before").unwrap(), "41");
        assert_eq!(runtime.eval_text("(answer)").unwrap(), "42");
    }

    #[test]
    fn issue_134_session_namespace_module_and_macro_state_is_isolated() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("session/namespace-isolation", "vars-shared"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("session/namespace-isolation", "modules-shared"),
            Form::Bool(false)
        );
        assert_eq!(
            module_expect("session/namespace-isolation", "macros-shared"),
            Form::Bool(false)
        );

        let mut kernel = SessionKernel::new();
        let alpha = session_id("alpha");
        let beta = session_id("beta");
        kernel.register_resource(
            "session.module",
            "(ns session.module) (defmacro chosen [] 41) (def answer 41)",
        );
        kernel.create_session(alpha.clone()).unwrap();
        kernel.create_session(beta.clone()).unwrap();
        kernel
            .eval(
                &alpha,
                "(do (require [session.module :as module :refer-macros [chosen]]) \
                     (def local-answer (chosen)) nil)",
            )
            .unwrap();
        assert_eq!(kernel.eval(&alpha, "local-answer").unwrap(), "41");
        assert!(kernel.eval(&beta, "local-answer").is_err());
        assert_eq!(
            kernel
                .eval(&alpha, "(boolean (Base/resolve 'user/local-answer))")
                .unwrap(),
            "true"
        );
        assert_eq!(
            kernel
                .eval(&beta, "(boolean (Base/resolve 'user/local-answer))")
                .unwrap(),
            "false"
        );
        assert_eq!(
            kernel
                .session(&alpha)
                .unwrap()
                .module_revision("session.module")
                .unwrap(),
            1
        );
        assert_eq!(
            kernel
                .session(&beta)
                .unwrap()
                .module_revision("session.module")
                .unwrap(),
            0
        );
        assert!(kernel.eval(&beta, "(chosen)").is_err());
    }

    #[test]
    fn issue_134_source_and_hir_have_value_metadata_and_error_parity() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("module/source-hir-parity", "same-value"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("module/source-hir-parity", "same-var-metadata"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("module/source-hir-parity", "same-error-category"),
            Form::Bool(true)
        );

        use crate::kernel::halc::encode_halc_module;

        let source = "(ns parity.demo) (defn value \"answer\" [] 42) (value)";
        let forms = kernel::parse_forms(source).unwrap();
        let artifact = encode_halc_module("parity.demo", "parity/demo.hal", source, forms).unwrap();

        let mut source_runtime = Runtime::new();
        let mut hir_runtime = Runtime::new();
        assert_eq!(source_runtime.eval_text(source).unwrap(), "42");
        assert_eq!(hir_runtime.eval_halc(&artifact).unwrap(), "42");

        let source_var = source_runtime
            .namespace_registry
            .find("parity.demo")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("value"))
            .unwrap();
        let hir_var = hir_runtime
            .namespace_registry
            .find("parity.demo")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("value"))
            .unwrap();
        assert_eq!(source_var.metadata(), hir_var.metadata());

        let failing_source = "(throw :parity-failed)";
        let failing_artifact = encode_halc_module(
            "parity.failure",
            "parity/failure.hal",
            failing_source,
            kernel::parse_forms(failing_source).unwrap(),
        )
        .unwrap();
        let source_error = source_runtime.eval_text(failing_source).unwrap_err();
        let hir_error = hir_runtime.eval_halc(&failing_artifact).unwrap_err();
        assert!(source_error.contains("thrown: :parity-failed"));
        assert!(hir_error.contains("thrown: :parity-failed"));
    }

    #[test]
    fn issue_134_runtime_profile_declares_deterministic_resource_precedence() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("module/resource-precedence", "deterministic"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("module/resource-precedence", "declared-by-runtime-profile"),
            Form::Bool(true)
        );
        assert_eq!(
            module_runtime_profile("rust", "resource-order"),
            Form::Vector(vec![
                Form::Keyword("loaded-native-namespace".into()),
                Form::Keyword("registered-resource".into()),
                Form::Keyword("registered-extension".into()),
            ])
        );

        let mut runtime = Runtime::new();
        runtime.extensions.install(RangeExtension);
        runtime.register_resource(
            "range",
            "(def resource-precedence-marker 42) resource-precedence-marker",
        );
        assert_eq!(runtime.require_resource("range").unwrap(), "42");
        assert_eq!(runtime.require_resource("range").unwrap(), ":loaded");
        assert_eq!(
            runtime.eval_text("resource-precedence-marker").unwrap(),
            "42"
        );
    }

    #[test]
    fn issue_134_sessions_unwind_bindings_and_transfer_only_immutable_data() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("session/dynamic-unwind", "binding-session-local"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("session/dynamic-unwind", "restored-after-error"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("session/immutable-transfer", "immutable-data"),
            Form::Bool(true)
        );
        for kind in [
            "functions",
            "vars",
            "mutable-references",
            "streams",
            "sockets",
            "host-handles",
        ] {
            assert_eq!(
                module_expect("session/reject-live-transfer", kind),
                Form::Bool(false)
            );
        }

        let mut kernel = SessionKernel::new();
        let alpha = session_id("alpha");
        let beta = session_id("beta");
        kernel.create_session(alpha.clone()).unwrap();
        kernel.create_session(beta.clone()).unwrap();
        assert_eq!(
            kernel
                .eval(&alpha, "(do (def ^:dynamic *answer* 1) nil)")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            kernel
                .eval(&beta, "(do (def ^:dynamic *answer* 10) nil)")
                .unwrap(),
            "nil"
        );
        assert!(kernel
            .eval(&alpha, "(binding [*answer* 2] (throw :binding-failed))")
            .is_err());
        assert_eq!(kernel.eval(&alpha, "*answer*").unwrap(), "1");
        assert_eq!(kernel.eval(&beta, "*answer*").unwrap(), "10");

        assert_eq!(
            kernel
                .eval(&alpha, "{:answer [1 2 {:nested #{:immutable}}]}")
                .unwrap(),
            "{:answer [1 2 {:nested #{:immutable}}]}"
        );
        for source in [
            "(fn [value] value)",
            "(var *answer*)",
            "(atom 1)",
            "(iter [1 2 3])",
        ] {
            let error = kernel.eval(&alpha, source).unwrap_err();
            assert!(
                error.contains("SESSION_TRANSFER_REJECTED"),
                "{source} unexpectedly produced {error}"
            );
        }
        assert!(!core::session_transferable(&core::Value::Extension(
            core::ExtensionValue {
                provider: "socket".into(),
                type_name: "Socket".into(),
                handle: 1,
            }
        )));
    }

    #[test]
    fn issue_134_retained_repl_state_survives_errors_and_multiline_forms() {
        if repo_text("01-lang/001-language/draft/conformance/modules.edn").is_none() {
            return;
        }
        assert_eq!(
            module_expect("repl/retained-state", "namespace-retained"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("repl/retained-state", "multiline"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("repl/error-recovery", "session-survives"),
            Form::Bool(true)
        );
        assert_eq!(
            module_expect("repl/error-recovery", "namespace-restored"),
            Form::Bool(true)
        );

        let mut kernel = SessionKernel::new();
        let repl = session_id("repl");
        kernel.create_session(repl.clone()).unwrap();
        assert_eq!(
            kernel
                .eval(
                    &repl,
                    "(ns retained.repl)\n(def answer\n  (+ 40\n     2))\nnil"
                )
                .unwrap(),
            "nil"
        );
        assert!(kernel.eval(&repl, "missing-symbol").is_err());
        assert_eq!(kernel.session_namespace(&repl).unwrap(), "retained.repl");
        assert_eq!(kernel.eval(&repl, "answer").unwrap(), "42");
    }

    #[test]
    fn issue_134_host_facades_are_loaded_session_local_and_non_transferable() {
        if repo_text("00-unsorted/runtime/draft/host-runtime.edn").is_none() {
            return;
        }
        for id in [
            "host/type-identity",
            "host/session-local-facade",
            "host/namespace-loaded",
            "host/no-live-transfer",
            "host/rejected-ex-info",
        ] {
            assert!(!host_conformance_case(id).is_empty());
        }

        let mut first = Runtime::new();
        let second = Runtime::new();
        assert_eq!(
            first
                .eval_text("(= Host std.native.Host std.foundation/Host)")
                .unwrap(),
            "true"
        );
        assert_eq!(
            first.eval_text("(ns-state 'std.native)").unwrap(),
            ":loaded"
        );
        assert_eq!(
            first.eval_text("(ns-state 'std.native.Host)").unwrap(),
            ":loaded"
        );

        let first_host = first
            .namespace_registry
            .find("std.native")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("Host"))
            .unwrap()
            .deref_value();
        let second_host = second
            .namespace_registry
            .find("std.native")
            .unwrap()
            .resolve(&crate::lang::data::Symbol::parse("Host"))
            .unwrap()
            .deref_value();
        let (core::Value::NativeType(first_host), core::Value::NativeType(second_host)) =
            (first_host, second_host)
        else {
            panic!("Host must be a native façade descriptor")
        };
        assert!(!Rc::ptr_eq(&first_host, &second_host));

        let mut kernel = SessionKernel::new();
        let host_transfer = session_id("host-transfer");
        kernel.create_session(host_transfer.clone()).unwrap();
        let error = kernel.eval(&host_transfer, "Host").unwrap_err();
        assert!(error.contains("SESSION_TRANSFER_REJECTED"), "{error}");

        assert_eq!(
            first
                .eval_text(
                    "(try
                       (deref (Host/call \"missing\" \"missing\" []))
                       (catch error
                         [(ex-message error)
                          (get (ex-data error) :ex/code)]))"
                )
                .unwrap(),
            "[\"std.native.Host/call requires capability :host-call\" :native/capability-denied]"
        );
        assert_eq!(
            first
                .eval_text(
                    "(deref
                       (promise/catch
                         (Host/call \"missing\" \"missing\" [])
                         (fn [error]
                           (get (ex-data error) :ex/code))))"
                )
                .unwrap(),
            ":native/capability-denied"
        );
        assert_eq!(
            kernel
                .eval(
                    &host_transfer,
                    "(try
                       (deref (Host/call \"missing\" \"missing\" []))
                       (catch error
                         (get (ex-data error) :ex/code)))"
                )
                .unwrap(),
            ":native/capability-denied"
        );
    }

    #[test]
    fn throw_and_try_catch_finally_are_host_neutral() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(try (throw (ex :test/failed {})) (catch error (:ex/code (ex-data error))))"
                )
                .unwrap(),
            ":test/failed"
        );
        assert_eq!(runtime.eval_text("(try 42 (finally 0))").unwrap(), "42");
        assert_eq!(
            runtime
                .eval_text("(try (throw (ex :test/failed {})) (catch error (str (:ex/code (ex-data error)) :handled)))")
                .unwrap(),
            "\":test/failed:handled\""
        );
        assert!(runtime
            .eval_text("(throw (ex :test/failed {}))")
            .unwrap_err()
            .contains(":test/failed"));
    }

    #[test]
    fn def_binds_values_in_the_current_environment() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(def player 1)").unwrap(),
            "#'user/player"
        );
        assert_eq!(
            runtime.eval_text("(= (def player 1) #'player)").unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(deref (def player 1))").unwrap(), "1");
        assert_eq!(
            runtime
                .eval_text("(do (def answer 41) (+ answer 1))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text("(do (def answer 42) (deref (var answer)))")
                .unwrap(),
            "42"
        );
        assert!(runtime
            .eval_text("(deref 42)")
            .unwrap_err()
            .contains("deref expects a var"));
        assert_eq!(
            runtime
                .eval_text("(do (def answer 1) (def answer 42) answer)")
                .unwrap(),
            "42"
        );
        assert!(runtime
            .eval_text("(def 1 2)")
            .unwrap_err()
            .contains("def name must be a symbol"));
    }

    #[test]
    fn anonymous_namespace_form_reuses_the_current_session_namespace() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(ns+)").unwrap(), "nil");
        assert_eq!(runtime.current_namespace(), "user");
        assert_eq!(
            runtime.eval_text("(ns+) (def player 1)").unwrap(),
            "#'user/player"
        );
        assert!(runtime
            .eval_text("(ns+ public.name)")
            .unwrap_err()
            .contains("does not accept a namespace name"));
    }

    #[test]
    fn vars_preserve_identity_and_support_root_mutation() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(do (def answer 1) (= (var answer) (var answer)))")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(do (def answer 1) (set! answer 42) (deref (var answer)))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(do (def answer 1) (let (v (var answer)) (do (set! answer 7) (deref v))))"
                )
                .unwrap(),
            "7"
        );
        assert_eq!(runtime.eval_text("(do (def answer 1) (defn add [x y] (+ x y)) (alter-var-root (var answer) add 40) answer)").unwrap(), "41");
        assert_eq!(
            runtime
                .eval_text("(do (def answer 1) ((fn [value] (var-sym value)) (var answer)))")
                .unwrap(),
            "user/answer"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(do (def answer 1) ((fn [value] (std.foundation/var-sym value)) (var answer)))",
                )
                .unwrap(),
            "user/answer"
        );
        assert_eq!(
            runtime.eval_text("(assoc [1 2 3] 0 :x)").unwrap(),
            "[:x 2 3]"
        );
        assert_eq!(
            runtime.eval_text("(assoc [1 2 3] 3 :x)").unwrap(),
            "[1 2 3 :x]"
        );
        assert_eq!(
            runtime.eval_text("(assoc [1 2 3] 5 :x)").unwrap_err(),
            "assoc index out of bounds"
        );
        assert_eq!(
            runtime.eval_text("(set! missing 1)").unwrap_err(),
            "unbound var: missing"
        );
    }

    #[test]
    fn functions_capture_lexical_values_and_support_defn() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("((fn [x] (+ x 1)) 41)").unwrap(), "42");
        assert_eq!(
            runtime
                .eval_text("(let (inc (fn [x] (+ x 1))) (inc 41))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text("(do (defn add1 [x] (+ x 1)) (add1 41))")
                .unwrap(),
            "42"
        );
        assert_eq!(runtime.eval_text("(do (defn factorial [n] (if (<= n 1) 1 (* n (factorial (dec n))))) (factorial 5))").unwrap(), "120");
        assert_eq!(
            runtime
                .eval_text("(let (x 40) (let (f (fn [y] (+ x y))) (f 2)))")
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn quote_lists_and_do_match_core_shapes() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("'(1 2)").unwrap(), "(1 2)");
        assert_eq!(runtime.eval_text("(count '(1 2 3))").unwrap(), "3");
        assert_eq!(runtime.eval_text("(nth (cons 0 '(1 2)) 0)").unwrap(), "0");
        assert_eq!(runtime.eval_text("(do 1 2 3)").unwrap(), "3");
    }

    #[test]
    fn arbitrary_precision_bit_operations_match_core_contract() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(bit-and 6 3)").unwrap(), "2");
        assert_eq!(runtime.eval_text("(bit-or 1 2)").unwrap(), "3");
        assert_eq!(runtime.eval_text("(bit-xor 7 3)").unwrap(), "4");
        assert_eq!(runtime.eval_text("(bit-not 0)").unwrap(), "-1");
        assert_eq!(runtime.eval_text("(bit-shift-right -4 1)").unwrap(), "-2");
        assert_eq!(
            runtime.eval_text("(bit-shift-left 1 31)").unwrap(),
            "2147483648"
        );
        assert!(runtime
            .eval_text("(bit-shift-left 1 -1)")
            .unwrap_err()
            .contains("distance must be a non-negative integer"));
    }

    #[test]
    fn core_language_numeric_and_truth_predicates_are_available() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(inc 41)").unwrap(), "42");
        assert_eq!(runtime.eval_text("(dec 43)").unwrap(), "42");
        assert_eq!(runtime.eval_text("(zero? 0)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(pos? 1)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(neg? -1)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(even? 4)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(odd? 3)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(long? 1)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(long? 1.0)").unwrap(), "false");
        assert_eq!(
            runtime
                .eval_text("(long? 9223372036854775808)")
                .unwrap(),
            "false"
        );
        assert_eq!(
            runtime
                .eval_text("(bigint? 9223372036854775808)")
                .unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(bigint? 1)").unwrap(), "false");
        assert_eq!(runtime.eval_text("(integer? 1)").unwrap(), "true");
        assert_eq!(
            runtime
                .eval_text("(integer? 9223372036854775808)")
                .unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(integer? 1.0)").unwrap(), "false");
        assert_eq!(runtime.eval_text("(nil? nil)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(true? true)").unwrap(), "true");
        assert_eq!(runtime.eval_text("(false? false)").unwrap(), "true");
    }

    #[test]
    fn tool_cli_handlers_treat_default_host_as_a_value() {
        let cli_root = std::path::Path::new(env!("HARA_SOURCE_ROOT")).join("../lib/src/tool/cli");
        for file in [
            "asset.hal",
            "extension.hal",
            "host.hal",
            "identity.hal",
            "language.hal",
            "lint.hal",
            "package.hal",
            "project_command.hal",
            "snapshot.hal",
            "spec.hal",
            "tap.hal",
        ] {
            let source = std::fs::read_to_string(cli_root.join(file)).unwrap();
            assert!(
                !source.contains("(work/default-host)"),
                "{file} calls the default host value as a function"
            );
        }
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn bytecode_vm_ifn_applicability_matches_interpreter_and_predicates() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_bytecode_native(
                    "(do
                       (defstruct VmInvokable [value])
                       (defstruct VmPlain [value])
                       (defmutable VmMutableInvokable [value])
                       (defmutable VmMutablePlain [value])
                       (extend-type VmInvokable IFn
                         (invoke [self] (:value self)))
                       (extend-type VmMutableInvokable IFn
                         (invoke [self] (:value self)))
                       (let [function (fn [value] value)
                             pointer (pointer {:context :kernel :id \"ROOT\"})
                             invokable (VmInvokable 7)
                             mutable-invokable (VmMutableInvokable 8)]
                         [[(fn? function) (satisfies? IFn function)
                           (function 1)]
                          [(fn? :key) (satisfies? IFn :key)
                           (:key {:key 2})]
                          [(fn? {:key 3}) (satisfies? IFn {:key 3})
                           ({:key 3} :key)]
                          [(fn? #{:key}) (satisfies? IFn #{:key})
                           (#{:key} :key)]
                          [(fn? VmInvokable) (satisfies? IFn VmInvokable)]
                          [(fn? pointer) (satisfies? IFn pointer)]
                          [(fn? invokable) (satisfies? IFn invokable)
                           (invokable)]
                          [(fn? mutable-invokable)
                           (satisfies? IFn mutable-invokable)
                           (mutable-invokable)]
                          [(fn? (VmPlain 1))
                           (satisfies? IFn (VmPlain 1))]
                          [(fn? (VmMutablePlain 1))
                           (satisfies? IFn (VmMutablePlain 1))]
                          [(fn? 42) (satisfies? IFn 42)]]))",
                )
                .unwrap(),
            "[[true true 1] [true true 2] [true true 3] [true true :key] \
             [true true] [true true] [true true 7] [true true 8] \
             [false false] [false false] [false false]]"
        );
    }

    #[test]
    fn namespace_use_refers_public_values() {
        let mut runtime = Runtime::new();
        runtime.register_resource(
            "native-use-fixture",
            "(ns native-use-fixture) (def answer 42)",
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(ns native-use-probe (:use native-use-fixture))\n\
                     answer",
                )
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn evaluated_regex_values_can_reenter_code_paths() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(eval (list 'identity #\"a+\"))")
                .unwrap(),
            "#\"a+\""
        );
    }

    #[test]
    fn empty_regexp_split_matches_jvm_character_partitioning() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(str/split \"+%?\" #\"\")").unwrap(),
            "[\"+\" \"%\" \"?\"]"
        );
    }

    #[test]
    fn native_string_calls_terminate_without_reentering_the_hal_facade() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns native-string-dispatch-probe\n\
                       (:require [std.foundation.string :as str]))\n\
                     [(str/blank? nil)\n\
                      (String/encode-utf8 \"f\")\n\
                      (str/encode-utf8 \"f\")]",
                )
                .unwrap(),
            "[true (bytes 102) (bytes 102)]"
        );
    }

    #[test]
    fn qualified_hal_facades_do_not_override_native_type_aliases() {
        let mut runtime = development_runtime();
        assert_eq!(
            runtime
                .eval_text(
                    "(ns qualified-facade-probe (:require [std.foundation.string :as str]))\n\
                     [(str/blank? nil)\n\
                      (str/to-fixed 1.25 2)\n\
                      (str (String/encode-utf8 \"f\"))\n\
                      (str (str/encode-utf8 \"f\"))]",
                )
                .unwrap(),
            "[true \"1.25\" \"(bytes 102)\" \"(bytes 102)\"]"
        );
    }

    #[test]
    fn guest_types_satisfy_and_dispatch_native_work_protocols() {
        let mut runtime = Runtime::new();
        let source =
            repo_text("01-lang/001-language/draft/conformance/fixtures/protocol_behavioral.hal")
                .expect("the specs-owned behavioral protocol corpus must be available");
        runtime.eval_text(&source).unwrap();
        let methods = runtime.eval_text("(capability-protocol-results)").unwrap();
        let receivers = runtime
            .eval_text("(protocol-capability-receiver-results)")
            .unwrap();
        assert!(!methods.contains(":pass false"), "{methods}");
        assert_eq!(methods.matches(":pass true").count(), 20);
        assert!(!receivers.contains(":pass false"), "{receivers}");
        assert_eq!(receivers.matches(":pass true").count(), 8);
    }

    #[test]
    fn agent_protocol_cross_runtime_fixture_runs_on_rust_runtime() {
        std::thread::Builder::new()
            .name("agent-protocol-cross-runtime".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let root = std::path::Path::new(env!("HARA_SOURCE_ROOT"));
                let source =
                    std::fs::read_to_string(root.join("../lib/test/work/agent_protocol_test.hal"))
                        .expect("the shared agent protocol fixture must be available");
                let mut runtime = development_runtime();
                runtime.eval_text(&source).unwrap();
                let output = runtime.eval_text("(map Result/success? results)").unwrap();
                assert_eq!(output, "[true true true true true true true]");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn agent_protocol_production_surface_has_no_retired_protocols() {
        fn visit(path: &std::path::Path, retired: &[&str], violations: &mut Vec<String>) {
            if path.is_dir() {
                for entry in std::fs::read_dir(path).unwrap() {
                    visit(&entry.unwrap().path(), retired, violations);
                }
                return;
            }
            if !path.is_file()
                || path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("tests.rs"))
            {
                return;
            }
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                return;
            };
            if !matches!(extension, "hal" | "java" | "rs") {
                return;
            }
            let source = std::fs::read_to_string(path).unwrap();
            for name in retired {
                if source.contains(name) {
                    violations.push(format!("{} contains {name}", path.display()));
                }
            }
        }

        let source_root = std::path::Path::new(env!("HARA_SOURCE_ROOT"));
        let roots = [
            source_root.join("../lib/src"),
            source_root.join("../lib/src-lang"),
            source_root.join("../java/src/main"),
            source_root.join("src"),
        ];
        let retired = [
            "IWorkAgent",
            "IAgentRuntime",
            "IAgentStore",
            "IAgentHost",
            "IAgentRun",
            "IAgentRef",
            "IAgentObserver",
            "IAgentMachine",
        ];
        let mut violations = Vec::new();
        for root in roots {
            visit(&root, &retired, &mut violations);
        }
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn rust_native_work_handles_are_available_to_guest_hara() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(let [host (std.native.Work/default-host)
                           run (IWorkHost/work-submit
                                host
                                (fn [work input options id] input)
                                42
                                {:id \"guest-native-run\"})]
                       [(satisfies? IWorkHost host)
                        (satisfies? IWorkRun run)
                        (IWorkRef/work-id run)
                        (IWorkRun/work-status run)
                        (deref (IWorkRun/work-result run))
                        (IWorkRun/work-status run)
                        (IClosed/closed? run)])",
                )
                .unwrap(),
            "[true true \"guest-native-run\" :queued 42 :completed true]"
        );
    }

    #[test]
    fn rust_native_scope_helpers_are_ordinary_work_native_functions() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(let [run (IWorkHost/work-submit
                                 (std.native.Work/default-host)
                                 :payload
                                 42
                                 {:id \"guest-scope-functions\"
                                  :work/execute
                                  (fn [work input options id]
                                    [(IWorkRef/work-id (std.native.Work/current-run))
                                     (std.native.Work/cancelled?)
                                     (std.native.Work/on-close (fn [run] nil))
                                     input])})]
                       (deref (IWorkRun/work-result run)))",
                )
                .unwrap(),
            "[\"guest-scope-functions\" false true 42]"
        );
    }

    #[test]
    fn map_iteration_and_find_return_canonical_pair_tuples() {
        let mut runtime = Runtime::core();
        assert_eq!(
            runtime
                .eval_text(
                    "[(type (first {:a 1}))
                      (type (IFind/find {:a 1} :a))
                      (count (first {:a 1}))
                      (count (IFind/find {:a 1} :a))]",
                )
                .unwrap(),
            "[:std.native.Tuple :std.native.Tuple 2 2]"
        );
    }

    #[test]
    fn shared_native_value_protocol_matrix_passes() {
        let mut runtime = Runtime::new();
        let source =
            repo_text("01-lang/001-language/draft/conformance/fixtures/protocol_behavioral.hal")
                .expect("the specs-owned behavioral protocol corpus must be available");
        runtime.eval_text(&source).unwrap();
        let result = runtime
            .eval_text("(protocol-native-value-results)")
            .unwrap();
        assert!(!result.contains(":pass false"), "{result}");
        assert_eq!(result.matches(":pass true").count(), 15);
    }

    #[test]
    fn code_translate_resolves_required_namespace_aliases() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                code_translate_resolves_required_namespace_aliases_body();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    fn code_translate_resolves_required_namespace_aliases_body() {
        let mut runtime = Runtime::core();
        for (namespace, _, source) in EMBEDDED_HAL_RESOURCES {
            runtime.register_resource(namespace, source);
        }
        // tool.migrate.project.* and its std.block/std.lib.zip dependencies are not
        // embedded bootstrap namespaces; register them from repository sources.
        let lib_src = std::path::Path::new(env!("HARA_SOURCE_ROOT"))
            .join("..")
            .join("lib")
            .join("src");
        for entry in std::fs::read_dir(&lib_src)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
        {
            register_lib_tree(&mut runtime, &lib_src, &entry);
        }
        let foundation = EMBEDDED_HAL_RESOURCES
            .iter()
            .find(|(namespace, _, _)| *namespace == "std.foundation")
            .expect("embedded std.foundation source")
            .2;
        runtime.eval_text(foundation).unwrap();
        runtime.eval_text("(ns user)").unwrap();
        runtime.eval_text("(require 'tool.migrate.rule)").unwrap();
        assert_eq!(
            runtime
                .eval_text(
                    r#"(str "std.foundation"
                              (str/slice "std.lib.foundation/T"
                                         (count "std.lib.foundation")
                                         (count "std.lib.foundation/T")))"#,
                )
                .unwrap(),
            r#""std.foundation/T""#
        );
        assert_eq!(
            runtime
                .eval_text(
                    r#"(let [root (tool.migrate.common.block/block-input "[std.lib.foundation/T]")
                              rule {:rewrite {:op :replace-token-prefix :text "std.foundation"}}
                              match {:source "std.lib.foundation/T"
                                     :match/text "std.lib.foundation"
                                     :path [0 0]}]
                          (tool.migrate.common.block/render
                           (tool.migrate.rule/apply-match root rule match)))"#,
                )
                .unwrap(),
            r#""[std.foundation/T]""#
        );
        assert_eq!(
            runtime
                .eval_text(
                    r#"(get
                         (tool.migrate.rule/translate-source
                          "(ns demo (:require [std.lib.collection :as c] [std.lib.walk :as walk] [std.native.Json :as json]))\n[c/map-keys walk/prewalk-replace json/read]"
                          {:mode :safe})
                         :output)"#,
                )
                .unwrap(),
            r#""(ns demo (:require [std.lib.collection :as c] [std.lib.walk :as walk] ))\n[std.foundation/map-keys std.foundation/prewalk-replace Json/read]""#
        );
        assert_eq!(
            runtime
                .eval_text(
                    r#"(get
                         (tool.migrate.rule/translate-source
                          "(ns demo (:require [std.lib.foundation :as foundation]))\n[std.lib.foundation/T foundation/F]"
                          {:mode :review})
                         :output)"#,
                )
                .unwrap(),
            r#""(ns demo (:require [std.foundation :as foundation]))\n[std.foundation/T std.foundation/F]""#
        );
        assert_eq!(
            runtime
                .eval_text("(count tool.migrate.rule/+ruleset+)")
                .unwrap(),
            "117"
        );
        for declaration in core::native_declarations() {
            let native_type = declaration.name;
            let expression = format!(
                r#"(let [output
                         (get
                          (tool.migrate.rule/translate-source
                           "(ns demo (:require [std.native.{native_type} :as native]))\n[native/probe std.native.{native_type}/probe]"
                           {{:mode :safe}})
                          :output)]
                     [(str/includes? output "{native_type}/probe")
                      (str/includes? output "native/probe")
                      (str/includes? output "std.native.")])"#
            );
            assert_eq!(
                runtime.eval_text(&expression).unwrap(),
                "[true false false]",
                "native static translation failed for {native_type}"
            );
        }

        // The tool.migrate.project native type list must equal the closed native.edn
        // inventory (both spell the canonical RegExp).
        if let Some(contract_source) =
            repo_text("01-lang/001-language/draft/conformance/native.edn")
        {
            let contract = kernel::parse_forms(&contract_source).unwrap().remove(0);
            let Form::Map(contract) = contract else {
                panic!("native contract must be a map")
            };
            let types = contract
                .iter()
                .find_map(|(key, value)| {
                    matches!(key, Form::Keyword(name) if name == "types").then_some(value)
                })
                .expect(":types present");
            let Form::Vector(types) = types else {
                panic!(":types must be a vector")
            };
            let mut contract_names = types
                .iter()
                .map(|entry| {
                    let Form::Map(entry) = entry else {
                        panic!("native type entries must be maps")
                    };
                    entry
                        .iter()
                        .find_map(|(key, value)| match (key, value) {
                            (Form::Keyword(k), Form::Symbol(n)) if k == "name" => Some(n.clone()),
                            _ => None,
                        })
                        .expect("native :name")
                })
                .collect::<Vec<_>>();
            contract_names.sort();
            let expected = format!(
                "[{}]",
                contract_names
                    .iter()
                    .map(|name| format!("\"{name}\""))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            assert_eq!(
                runtime
                    .eval_text("(vec (sort tool.migrate.rules/+native-static-types+))",)
                    .unwrap(),
                expected,
                "tool.migrate.rules/+native-static-types+ differs from native.edn"
            );
        }
    }

    #[test]
    fn core_sequence_navigation_ranges_and_quantifiers() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(second [10 20 30])").unwrap(), "20");
        assert_eq!(runtime.eval_text("(not-empty [])").unwrap(), "nil");
        assert_eq!(runtime.eval_text("(not-empty [1])").unwrap(), "[1]");
        assert_eq!(runtime.eval_text("(range 3)").unwrap(), "(0 1 2)");
        assert_eq!(
            runtime
                .eval_text("(vector? (map (fn [x] (+ x 1)) [1 2 3]))")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(first (map (fn [x] (+ x 1)) [1 2 3]))")
                .unwrap(),
            "2"
        );
        assert_eq!(runtime.eval_text("(count (range 2 5))").unwrap(), "3");
        assert_eq!(runtime.eval_text("(count (repeat 4 :x))").unwrap(), "4");
        assert_eq!(
            runtime
                .eval_text("(every? (fn [x] (pos? x)) [1 2 3])")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(any? (fn [x] (= x 2)) [1 2 3])")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn map_and_zip_support_multiple_collections() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(nth (map (fn [x y] (+ x y)) [1 2] [10 20]) 1)")
                .unwrap(),
            "22"
        );
        assert_eq!(
            runtime
                .eval_text("(count (map (fn [x y z] (+ x (+ y z))) [1 2] [10 20] [100 200]))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(nth (zip [1 2] [:a :b] [true false]) 0)")
                .unwrap(),
            "[1 :a true]"
        );
        assert_eq!(
            runtime.eval_text("(count (zip [1 2 3] [:a :b]))").unwrap(),
            "2"
        );
    }

    #[test]
    fn lazy_iterator_generators_are_bounded_by_consumers() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(count ((take 4) (repeat :x)))").unwrap(),
            "4"
        );
        assert_eq!(
            runtime.eval_text("(first ((drop 3) (repeat :x)))").unwrap(),
            ":x"
        );
        assert_eq!(
            runtime
                .eval_text("(Iter/iter-finite? (repeat :x))")
                .unwrap(),
            "false"
        );
        assert_eq!(
            runtime
                .eval_text("(count ((take 3) (repeatedly (constantly 7))))")
                .unwrap(),
            "3"
        );
        assert_eq!(
            runtime
                .eval_text("(count ((take 5) (iterate (fn [x] (+ x 2)) 0)))")
                .unwrap(),
            "5"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(count ((take 3) ((take-while (fn [x] (< x 10))) (iterate (fn [x] (+ x 2)) 0))))"
                )
                .unwrap(),
            "3"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(first ((take 2) ((drop-while (fn [x] (< x 4))) (iterate (fn [x] (+ x 2)) 0))))"
                )
                .unwrap(),
            "4"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(first ((drop 3) ((take 4) ((map (fn [x] (* x 2))) (iterate (fn [x] (+ x 1)) 0)))))"
                )
                .unwrap(),
            "6"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(first ((take 2) ((filter (fn [x] (even? x))) (iterate (fn [x] (+ x 1)) 0))))"
                )
                .unwrap(),
            "0"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(first ((drop 3) ((take 4) ((mapcat (fn [x] [x x])) (iterate (fn [x] (+ x 1)) 0)))))"
                )
                .unwrap(),
            "1"
        );
        assert_eq!(runtime.eval_text("(first ((take 2) ((keep (fn [x] (if (even? x) (* x 10) nil))) (iterate (fn [x] (+ x 1)) 0))))").unwrap(), "0");
        assert_eq!(
            runtime
                .eval_text(
                    "(first ((drop 2) ((take 3) (Iter/iter-zip (iterate (fn [x] (+ x 1)) 0) (repeat :x)))))"
                )
                .unwrap(),
            "[2 :x]"
        );
        assert_eq!(
            runtime
                .eval_text("(first ((drop 3) ((take 4) (Iter/iter-interleave (iterate (fn [x] (+ x 1)) 0) (repeat :x)))))")
                .unwrap(),
            ":x"
        );
        assert_eq!(
            runtime
                .eval_text("(first ((drop 2) ((take 3) ((partition-all 2) (iterate (fn [x] (+ x 1)) 0)))))")
                .unwrap(),
            "[4 5]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(first ((drop 1) ((take 2) ((partition 2) (iterate (fn [x] (+ x 1)) 0)))))"
                )
                .unwrap(),
            "[2 3]"
        );
        assert_eq!(
            runtime
                .eval_text("(first ((take 4) (iterate (fn [x] (+ x 2)) 0)))")
                .unwrap(),
            "0"
        );
        assert_eq!(
            runtime.eval_text("(first (drop 1 (repeat :x)))").unwrap(),
            ":x"
        );
        assert_eq!(
            runtime
                .eval_text("(first (rest (iterate (fn [x] (+ x 1)) 0)))")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text("(first (drop 3 (take 4 (iterate (fn [x] (+ x 2)) 0))))")
                .unwrap(),
            "6"
        );
    }

    #[test]
    fn function_combinators_capture_values_and_functions() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("((constantly 42) 1 2 3)").unwrap(), "42");
        assert_eq!(
            runtime
                .eval_text("((complement (fn [x] (> x 2))) 1)")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("((comp (fn [x] (+ x 1)) (fn [x] (* x 2))) 20)")
                .unwrap(),
            "41"
        );
        assert_eq!(
            runtime
                .eval_text("((comp (fn [x] (+ x 1)) (fn [x] (+ x 1)) (fn [x] (+ x 1))) 39)")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime.eval_text("((comp inc inc inc inc) 38)").unwrap(),
            "42"
        );
    }

    #[test]
    fn public_map_doto_and_set_helpers_are_portable() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "[(map-keys (fn [x] (+ x 1)) {1 :a 2 :b}) \
                     (map-vals (fn [x] (+ x 1)) {:a 1 :b 2}) \
                     (let [calls (atom 0) \
                           value (doto \
                                   (do (swap! calls (fn [x] (+ x 1))) (atom [])) \
                                   (swap! (fn [values item] (conj values item)) 1) \
                                   (swap! (fn [values item] (conj values item)) 2))] \
                       [(deref calls) (deref value)])]"
                )
                .unwrap(),
            "[{2 :a 3 :b} {:a 2 :b 3} [1 [1 2]]]"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "[(union #{1 2} #{2 3}) \
                        (intersection #{1 2 3} #{2 3 4} #{3 5}) \
                        (difference #{1 2 3} #{2} #{3}) \
                        (subset? #{1 2} #{1 2 3}) \
                        (superset? #{1 2 3} #{1 2}) \
                        (= #{1 3} (set (filter odd? #{1 2 3 4})))]"
                )
                .unwrap(),
            "[#{1 2 3} #{3} #{1} true true true]"
        );
    }

    #[test]
    fn nested_associative_helpers_match_core_language_shapes() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(get-in {:a {:b 42}} [:a :b])").unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text("(get-in (object :a (object :b 42)) [:a :b])")
                .unwrap(),
            "42"
        );
        assert_eq!(runtime.eval_text("(get (object :a 7) :a)").unwrap(), "7");
        assert_eq!(
            runtime
                .eval_text("(get-in {:a {:b 42}} [:a :missing])")
                .unwrap(),
            "nil"
        );
        assert_eq!(
            runtime
                .eval_text("(get-in (assoc-in {} [:a :b] 42) [:a :b])")
                .unwrap(),
            "42"
        );
        assert_eq!(runtime.eval_text("(get {:a 3} :a)").unwrap(), "3");
        assert_eq!(
            runtime
                .eval_text("(get (update {:a 3} :a (fn [x] (+ x 2))) :a)")
                .unwrap(),
            "5"
        );
        assert_eq!(
            runtime
                .eval_text("(get-in (update-in {:a {:b 3}} [:a :b] (fn [x y] (+ x y)) 4) [:a :b])")
                .unwrap(),
            "7"
        );
        assert_eq!(
            runtime.eval_text("(get (assoc {} :a 1 :b 2) :b)").unwrap(),
            "2"
        );
    }

    #[test]
    fn streaming_transforms_follow_the_primary_source_mode() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = Runtime::new();
                assert_eq!(
                    runtime
                        .eval_text("[(seq? (drop 1 (iterate inc 0))) (first (drop 1 (iterate inc 0))) (first (drop 1 (iterate inc 0)))]")
                        .unwrap(),
                    "[true 1 1]"
                );
                assert_eq!(
                    runtime
                        .eval_text("(let [output (drop 1 (iter [1 2 3]))] [(iter? output) (iter-next output) (iter-next output)])")
                        .unwrap(),
                    "[true 2 3]"
                );
                assert_eq!(
                    runtime
                        .eval_text("[(vector? ((map inc) [1 2 3])) (seq? ((map inc) (seq [1 2 3]))) (iter? ((map inc) (iter [1 2 3])))]")
                        .unwrap(),
                    "[true true true]"
                );
                assert_eq!(
                    runtime
                        .eval_text("(->> (iterate inc 0) (drop 1) (take-while (fn [value] (< value 5))) (filter (fn [value] (= 0 (mod value 2)))) first)")
                        .unwrap(),
                    "2"
                );
                assert_eq!(runtime.eval_text("(drop 1 '(a b c d))").unwrap(), "(b c d)");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn opaque_extensions_use_compact_tagged_display() {
        let value = core::Value::Extension(core::ExtensionValue {
            provider: "math.tensor".into(),
            type_name: "tensor".into(),
            handle: 42,
        });
        assert_eq!(value.display(), "#ht[:handle 42]");
    }
    #[test]
    fn iterator_combinators_cover_core_shapes() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(count (take-while (fn [x] (< x 3)) (range 5)))")
                .unwrap(),
            "3"
        );
        assert_eq!(
            runtime
                .eval_text("(count (drop-while (fn [x] (< x 3)) (range 5)))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(count (mapcat (fn [x] [x x]) [1 2]))")
                .unwrap(),
            "4"
        );
        assert_eq!(
            runtime
                .eval_text("(count (keep (fn [x] (if (even? x) (* x 10) nil)) [1 2 3 4]))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime
                .eval_text("(count (partition-all 2 [1 2 3]))")
                .unwrap(),
            "2"
        );
        assert_eq!(
            runtime.eval_text("(count (partition 2 [1 2 3]))").unwrap(),
            "1"
        );
        assert_eq!(
            runtime.eval_text("(count (interpose :x [1 2 3]))").unwrap(),
            "5"
        );
        assert_eq!(
            runtime
                .eval_text("(count (interleave [1 2] [:a :b]))")
                .unwrap(),
            "4"
        );
        assert_eq!(
            runtime
                .eval_text("(count (partition-pair [1 2 3]))")
                .unwrap(),
            "1"
        );
    }

    #[test]
    fn arithmetic() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("[(+ 19 23) (* 2 3 4) (- 10) (/ 2)]")
                .unwrap(),
            "[42 24 -10 0]"
        );
        assert!(runtime.eval_text("(+)").unwrap_err().contains("expects"));
        assert!(runtime.eval_text("(*)").unwrap_err().contains("expects"));
        assert!(runtime
            .eval_text("(apply + [])")
            .unwrap_err()
            .contains("expects"));
        assert!(runtime
            .eval_text("(apply * [])")
            .unwrap_err()
            .contains("expects"));
        assert!(runtime.eval_text("(-)").unwrap_err().contains("expects"));
        assert!(runtime.eval_text("(/)").unwrap_err().contains("expects"));
        assert!(runtime
            .eval_text("(% 3)")
            .unwrap_err()
            .contains("unbound symbol: %"));
        assert_eq!(runtime.eval_text("(-> 1 (+ % %))").unwrap(), "2");
        assert_eq!(runtime.eval_text("(->> 3 (+ % %))").unwrap(), "6");
    }

    #[test]
    fn integer_arithmetic_promotes_machine_integer_overflow() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(inc 9223372036854775807)").unwrap(),
            "9223372036854775808"
        );
        assert_eq!(
            runtime.eval_text("(dec -9223372036854775808)").unwrap(),
            "-9223372036854775809"
        );
        assert_eq!(
            runtime.eval_text("(* 9223372036854775807 2)").unwrap(),
            "18446744073709551614"
        );
        assert_eq!(
            runtime
                .eval_text("(+ 9223372036854775808 -9223372036854775807)")
                .unwrap(),
            "1"
        );
        assert_eq!(
            runtime
                .eval_text("(long? (inc 9223372036854775807))")
                .unwrap(),
            "false"
        );
        assert_eq!(
            runtime
                .eval_text("(bigint? (inc 9223372036854775807))")
                .unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(integer? (inc 9223372036854775807))")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn floating_literals_parse_as_floats() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(+ 0.1 0.2)").unwrap(),
            "(double 0.30000000000000004)"
        );
        assert_eq!(runtime.eval_text("(/ 1.0 8.0)").unwrap(), "(double 0.125)");
        assert_eq!(
            runtime.eval_text("(/ 1.0 3.0)").unwrap(),
            "(double 0.3333333333333333)"
        );
        assert_eq!(runtime.eval_text("(double 1.5)").unwrap(), "(double 1.5)");
    }

    #[test]
    fn arbitrary_integer_bits_and_checked_long_conversion_are_stable() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(bit-shift-left 1 80)").unwrap(),
            "1208925819614629174706176"
        );
        assert!(runtime
            .eval_text("(long 9223372036854775808)")
            .unwrap_err()
            .contains("outside signed 64-bit range"));
    }

    #[test]
    fn declare_noop() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(declare x)").unwrap(), "nil");
    }

    #[test]
    fn recur_cannot_escape_loop_or_function_boundaries() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(recur 1)").unwrap_err(),
            "recur must be inside loop"
        );
        assert_eq!(
            runtime.eval_text("((fn [] (recur 1)))").unwrap_err(),
            "recur must be inside loop"
        );
    }

    #[test]
    fn loop_supports_binding_vectors_and_multiple_recur_values() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(loop [x 0 y 1] (if (< x 4) (recur (+ x 1) (+ y x)) y))")
                .unwrap(),
            "7"
        );
        assert!(runtime
            .eval_text("(loop [x 0 y 1] (recur 2))")
            .unwrap_err()
            .contains("loop recur arity mismatch"));
    }

    #[test]
    fn loop_and_recur_support_tail_recursive_bootstrap_forms() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(loop (x 0) (if (< x 5) (recur (+ x 1)) x))")
                .unwrap(),
            "5"
        );
        assert_eq!(
            runtime
                .eval_text("(loop (x 1) (do (if (< x 3) (recur (* x 2)) x)))")
                .unwrap(),
            "4"
        );
    }

    #[test]
    fn let_accepts_binding_vectors_and_multiple_sequential_pairs() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(let [x 19 y 23] (+ x y))").unwrap(),
            "42"
        );
        assert_eq!(
            runtime.eval_text("(let (x 19 y (+ x 23)) y)").unwrap(),
            "42"
        );
        assert!(runtime
            .eval_text("(let [x 1 y] y)")
            .unwrap_err()
            .contains("name/value pairs"));
    }

    #[test]
    fn letfn_supports_local_recursion_mutual_recursion_and_scope_restoration() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(letfn [(sum [n acc] (if (= n 0) acc (sum (- n 1) (+ acc n))))] (sum 5 0))",
                )
                .unwrap(),
            "15"
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(letfn [(even-local? [n] (if (= n 0) true (odd-local? (- n 1)))) (odd-local? [n] (if (= n 0) false (even-local? (- n 1))))] [(even-local? 8) (odd-local? 7)])",
                )
                .unwrap(),
            "[true true]"
        );
        assert!(runtime.eval_text("even-local?").is_err());
        assert!(runtime
            .eval_text("(letfn [(f [x] x) (f [x] x)] (f 1))")
            .unwrap_err()
            .contains("Duplicate letfn name"));
    }

    #[test]
    fn read_forms_uses_the_capability_gated_file_provider() {
        let mut runtime = Runtime::new();
        runtime.install_memory_file_provider("/");
        runtime
            .eval_text(
                "(deref (file/write \"/sample.hal\" (bytes 40 110 115 32 116 121 112 101 100 46 115 97 109 112 108 101 41 10 40 100 101 102 32 118 97 108 117 101 32 52 50 41)))",
            )
            .unwrap();
        assert_eq!(
            runtime
                .eval_text("(count (read-forms \"/sample.hal\"))")
                .unwrap(),
            "2"
        );
        assert!(runtime
            .eval_text("(read-forms \"typed/sample.clj\")")
            .unwrap_err()
            .contains(".hal or .hrl"));
    }

    #[test]
    fn conditional_and_let() {
        let mut runtime = Runtime::new();
        // Var display is namespace-qualified, matching the JVM runtime
        // (issue #223).
        assert_eq!(
            runtime.eval_text("(defn rank [score] score)").unwrap(),
            "#'user/rank"
        );
        assert_eq!(
            runtime
                .eval_text("(let (x 19) (if true (+ x 23) 0))")
                .unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_text("(cond false \"gold\" (>= 70 50) \"silver\" :else \"bronze\")")
                .unwrap(),
            "\"silver\""
        );
        assert_eq!(runtime.eval_text("(cond false 1)").unwrap(), "nil");
        assert!(runtime
            .eval_text("(cond true 1 false)")
            .unwrap_err()
            .contains("test/expression pairs"));
    }

    #[test]
    fn lesson_definition_cases_run_from_the_core_language_conformance_corpus() {
        fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> &'a Form {
            entries
                .iter()
                .find_map(|(candidate, value)| match candidate {
                    Form::Keyword(name) if name == key => Some(value),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing :{key}"))
        }

        let Some(corpus) = repo_text("docs/docs/reference/l0-conformance.edn") else {
            return;
        };
        let manifest = kernel::parse_forms(&corpus).unwrap().remove(0);
        let Form::Map(manifest) = manifest else {
            panic!("conformance corpus must be a map")
        };
        let Form::Vector(cases) = entry(&manifest, "cases") else {
            panic!("conformance :cases must be a vector")
        };
        let mut runtime = Runtime::new();

        for id in ["compiler/defn-var", "runtime/cond-defined-function"] {
            let case = cases
                .iter()
                .find(|case| {
                    matches!(
                        case,
                        Form::Map(entries)
                            if matches!(entry(entries, "id"), Form::Keyword(name) if name == id)
                    )
                })
                .unwrap_or_else(|| panic!("missing conformance case :{id}"));
            let Form::Map(case) = case else {
                unreachable!()
            };
            let Form::String(source) = entry(case, "source") else {
                panic!(":{id} source must be a string")
            };
            let Form::Map(expect) = entry(case, "expect") else {
                panic!(":{id} expect must be a map")
            };
            let Form::String(expected) = entry(expect, "value") else {
                panic!(":{id} expected value must be a string")
            };
            let Form::Keyword(expected_type) = entry(expect, "type") else {
                panic!(":{id} expected type must be a keyword")
            };
            let expected = if expected_type == "string" {
                format!("{expected:?}")
            } else {
                expected.clone()
            };
            assert_eq!(runtime.eval_text(source).unwrap(), expected, ":{id}");
        }
    }

    #[test]
    fn errors_are_stable() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("unknown").unwrap_err(),
            "unbound symbol: unknown"
        );
    }

    #[test]
    fn mutable_collections_build_in_place_and_freeze_once() {
        let mut runtime = Runtime::new();
        let source = "(let [m (to-mutable {})]
                        (do
                          (loop [i 0]
                            (if (< i 500)
                              (do (assoc m i (+ i 1)) (recur (+ i 1)))
                              nil))
                          (let [p (to-persistent m)]
                            (+ (count p) (get p 499)))))";
        assert_eq!(runtime.eval_text(source).unwrap(), "1000");
        assert_eq!(
            runtime
                .eval_text("(let [m (to-mutable {:a 1})] (do (assoc m :b 2) (get m :b)))")
                .unwrap(),
            "2"
        );
        assert!(runtime
            .eval_text("(let [m (to-mutable {}) p (to-persistent m)] (do p (assoc m :late 1)))")
            .unwrap_err()
            .contains("mutable collection used after to-persistent"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_embedding_can_receive_values_without_display_serialization() {
        let mut runtime = Runtime::new();
        let value = runtime
            .eval_native_value("(do (def answer 42) {:answer #'answer})")
            .unwrap();
        let entries = core::map_entries(&value).expect("expected map");
        assert!(entries.iter().any(|(key, value)| matches!(
            (key, value),
            (core::Value::Keyword(name), core::Value::Var(var))
                if name.as_str() == "answer" && var.deref_value() == core::Value::Number(42)
        )));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_error_traces_are_opt_in_and_nested() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_native("(+ 19 23)").unwrap(), "42");
        assert_eq!(
            runtime.eval_native("unknown").unwrap_err(),
            "unbound symbol: unknown"
        );
        let error = runtime
            .eval_native_traced("(do (defn inner [] (/ 1 0)) (defn outer [] (inner)) (outer))")
            .unwrap_err();
        assert!(error.contains("[hara stack]"));
        assert!(error.contains("at user/inner"));
        assert!(error.contains("at user/outer"));
        assert_eq!(error.matches("[hara stack]").count(), 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_error_traces_include_namespace_and_source_location() {
        let mut runtime = Runtime::new();
        runtime.register_resource(
            "trace.source",
            "(ns trace.source)\n\n(defn inner [] missing)\n\n(inner)",
        );
        let error = runtime
            .eval_native_traced("(require 'trace.source)")
            .unwrap_err();
        assert!(error.contains("at trace.source/inner (trace.source:5:1)"));
    }
    #[test]
    fn runtime_metadata_round_trips_through_protocols_and_reader_literals() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(ILookup/lookup (IObjType/meta (IObjType/with-meta [1] {:doc \"vector\"})) :doc)").unwrap(),
            "\"vector\""
        );
        assert_eq!(
            runtime
                .eval_text("(ILookup/lookup (IObjType/meta (quote ^{:doc \"quoted\"} [1])) :doc)")
                .unwrap(),
            "\"quoted\""
        );
        assert_eq!(
            runtime
                .eval_text(
                    "(let [handler (IObjType/with-meta (fn [value] value) {:handler/id :handler})] \
                     [(ILookup/lookup (IObjType/meta handler) :handler/id) \
                      (handler 42) \
                      (fn? handler)])"
                )
                .unwrap(),
            "[:handler 42 true]"
        );
    }
    #[test]
    fn typed_vars_preserve_definition_metadata_and_dynamic_binding_scope() {
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_text("(do (def ^:dynamic *answer* 1) (binding [*answer* 42] (binding [*answer* 43] *answer*)))").unwrap(), "43");
        assert_eq!(runtime.eval_text("*answer*").unwrap(), "1");
        assert_eq!(
            runtime
                .eval_text("(do (ns binding.consumer) (binding [user/*answer* 44] user/*answer*))")
                .unwrap(),
            "44"
        );
        assert_eq!(runtime.eval_text("user/*answer*").unwrap(), "1");
        runtime.eval_text("(ns user)").unwrap();
        assert_eq!(
            runtime
                .eval_text("(ILookup/lookup (IObjType/meta (var *answer*)) :dynamic)")
                .unwrap(),
            "true"
        );
        assert_eq!(runtime.eval_text("(do (def ^{:doc \"answer doc\"} answer 42) (ILookup/lookup (IObjType/meta (var answer)) :doc))").unwrap(), "\"answer doc\"");
        assert!(runtime
            .eval_text("(do (def plain 1) (binding [plain 2] plain))")
            .unwrap_err()
            .contains("dynamic Var"));
        let err = runtime
            .eval_text("(do (def ^:dynamic *left* 1) (binding [*left* 2 plain 3] *left*))")
            .unwrap_err();
        eprintln!("ERROR: {err}");
        assert!(err.contains("dynamic Var") || err.contains("name must be"));
        assert_eq!(runtime.eval_text("*left*").unwrap(), "1");
    }
    #[test]
    fn coroutine_introspection_works_in_cli_path() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(std.foundation.coroutine/status (std.foundation.coroutine/create (fn [x] x)))"
                )
                .unwrap(),
            ":suspended"
        );
        assert_eq!(
            runtime.eval_text("(std.foundation.coroutine/coroutine? (std.foundation.coroutine/create (fn [] 1)))").unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .eval_text("(std.foundation.coroutine/coroutine? 42)")
                .unwrap(),
            "false"
        );
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(def c (std.foundation.coroutine/create (fn [] 1))) (std.foundation.coroutine/status (std.foundation.coroutine/close c))").unwrap(),
            ":dead"
        );
        assert!(runtime
            .eval_text("(std.foundation.coroutine/resume c)")
            .unwrap_err()
            .contains("cannot resume a dead coroutine"));
        assert!(runtime
            .eval_text("(std.foundation.coroutine/yield 1)")
            .unwrap_err()
            .contains("coroutine/yield used outside of a coroutine"));
        assert_eq!(
            runtime
                .eval_text("(std.foundation.coroutine/await (promise/run (fn [] 1)))")
                .unwrap(),
            "1"
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn coroutine_resume_without_suspension_works_in_traced_path() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_native_traced("(def c (std.foundation.coroutine/create (fn [] 1))) (std.foundation.coroutine/resume c)")
                .unwrap(),
            "1"
        );
        assert!(runtime
            .eval_native_traced("(std.foundation.coroutine/yield 1)")
            .unwrap_err()
            .contains("fiber evaluator"));
        assert!(runtime
            .eval_native_traced("(std.foundation.coroutine/await (promise (fn [] 1)))")
            .unwrap_err()
            .contains("fiber evaluator"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_error_traces_preserve_coroutine_frames_across_yield() {
        let mut runtime = Runtime::new();
        runtime
            .eval_native_traced(
                "(def c (co/create (fn []\n  (co/yield 1)\n  missing)))",
            )
            .unwrap();
        assert_eq!(runtime.eval_native_traced("(co/resume c)").unwrap(), "1");
        let error = runtime.eval_native_traced("(co/resume c)").unwrap_err();
        assert!(error.contains("[hara stack]"));
        assert!(error.contains("at user/<anonymous> (user:1:1)"));
    }
    #[test]
    fn fiber_cli_path_evaluates_coroutine_resume_and_yield() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime.eval_text("(do (def coroutine (co/create (fn [x] (let [y (co/yield (* x 2))] (+ y 1))))) (co/resume coroutine 21))").unwrap(),
            "42"
        );
        assert_eq!(runtime.eval_text("(co/resume coroutine 20)").unwrap(), "21");
    }
    #[test]
    fn binding_forms_evaluate_multiple_body_expressions() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(let [a (array 1 2 3)] (Arr/push-last a 4) (Arr/get a 3))")
                .unwrap(),
            "4"
        );
        assert_eq!(
            runtime
                .eval_text("(loop [n 0] (+ n 1) (if (< n 2) (recur (+ n 1)) n))")
                .unwrap(),
            "2"
        );
    }
    #[test]
    fn fiber_cli_path_awaits_promise_inside_coroutine() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text(
                    "(def coroutine (co/create (fn [] (co/await (promise/run (fn [] 42)))))) (co/resume coroutine)"
                )
                .unwrap(),
            "42"
        );
    }
    #[test]
    fn coroutine_builtin_alias_does_not_require_the_foundation_namespace() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(co/status (co/create (fn [x] x)))")
                .unwrap(),
            ":suspended"
        );
        assert_eq!(
            runtime
                .eval_text("(co/coroutine? (co/create (fn [] 1)))")
                .unwrap(),
            "true"
        );
    }
    #[test]
    fn coroutine_default_alias_is_co() {
        let mut runtime = Runtime::new();
        assert_eq!(
            runtime
                .eval_text("(require 'std.foundation.coroutine) (co/status (co/create (fn [] 1)))")
                .unwrap(),
            ":suspended"
        );
    }
    #[test]
    fn eval_halc_runs_encoded_library() {
        use crate::kernel::halc::encode_halc_module;

        let source = "(ns demo)\n\
                      (def Address [:map [:street :str]])\n\
                      (def Customer [:map [:address #'Address]])\n\
                      (defn ^{:schema #'Customer} identity-customer [customer] customer)\n\
                      (identity-customer 42)";
        let forms = kernel::parse_forms(source).unwrap();
        let artifact = encode_halc_module("demo", "demo.hal", source, forms).unwrap();
        let mut runtime = Runtime::new();
        assert_eq!(runtime.eval_halc(&artifact).unwrap(), "42");
        assert!(runtime.halc_schema("demo/Address").is_some());
        assert!(runtime.halc_schema("demo/Customer").is_some());
        assert!(matches!(
            runtime.halc_function_type("demo/identity-customer"),
            Some(kernel::SchemaType::Map(fields)) if fields.len() == 1
        ));
        assert_eq!(
            runtime
                .halc_function_schema("demo/identity-customer")
                .unwrap()
                .to_string(),
            "(var demo/Customer)"
        );
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn halc_lowers_to_typed_bytecode_without_source_reconstruction() {
        use crate::kernel::halc::encode_halc_module;

        let source = "(ns typed.demo)\n\
                      (def Customer [:map [:id :int]])\n\
                      (def IdentityCustomer [:fn [#'Customer] #'Customer])\n\
                      (defn ^{:schema #'IdentityCustomer} identity-customer [customer] customer)\n\
                      (identity-customer 42)";
        let halc = encode_halc_module(
            "typed.demo",
            "typed/demo.hal",
            source,
            kernel::parse_forms(source).unwrap(),
        )
        .unwrap();
        let mut runtime = Runtime::new();
        let bytecode = runtime.compile_halc_bytecode_artifact(&halc).unwrap();
        let program = vm::decode_program(&bytecode).unwrap();
        assert!(matches!(
            program.function_types.get("typed.demo/identity-customer"),
            Some(kernel::SchemaType::Reference(name)) if name == "typed.demo/IdentityCustomer"
        ));
        assert!(matches!(
            program.schema_types.get("typed.demo/IdentityCustomer"),
            Some(kernel::SchemaType::Function(arities)) if arities.len() == 1
        ));
        assert!(matches!(
            program
                .inferred_function_types
                .get("typed.demo/identity-customer"),
            Some(kernel::SchemaType::Function(arities))
                if *arities[0].output == kernel::SchemaType::Reference("typed.demo/Customer".into())
        ));
        let identity_prototype = program
            .functions
            .iter()
            .position(|prototype| prototype.name.as_deref() == Some("identity-customer"))
            .unwrap() as u16;
        assert!(matches!(
            program.function_schema(identity_prototype),
            Some(kernel::SchemaType::Function(arities)) if arities.len() == 1
        ));
        assert_eq!(runtime.eval_bytecode_artifact(&bytecode).unwrap(), "42");
        assert!(runtime
            .halc_inferred_function_type("typed.demo/identity-customer")
            .is_some());

        let mismatch_source = "(ns typed.bad)\n\
                               (def Unary [:fn [:int] :int])\n\
                               (defn ^{:schema #'Unary} wrong [left right] left)";
        let mismatch_halc = encode_halc_module(
            "typed.bad",
            "typed/bad.hal",
            mismatch_source,
            kernel::parse_forms(mismatch_source).unwrap(),
        )
        .unwrap();
        assert!(runtime
            .compile_halc_bytecode_artifact(&mismatch_halc)
            .unwrap_err()
            .contains("function schema for wrong has no 2-argument arity"));
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn halc_bytecode_lowering_applies_namespace_requires_before_compilation() {
        use crate::kernel::halc::encode_halc_module;

        let source = "(ns typed.consumer (:require [typed.dependency :refer [answer]]))\n\
                      (defn read-answer [] answer)\n\
                      (read-answer)";
        let halc = encode_halc_module(
            "typed.consumer",
            "typed/consumer.hal",
            source,
            kernel::parse_forms(source).unwrap(),
        )
        .unwrap();
        let mut runtime = Runtime::new();
        runtime.register_resource("typed.dependency", "(ns typed.dependency) (def answer 42)");

        let bytecode = runtime.compile_halc_bytecode_artifact(&halc).unwrap();
        assert_eq!(runtime.eval_bytecode_artifact(&bytecode).unwrap(), "42");
    }

    #[test]
    #[ignore = "requires a Truffle-compiled foundation HALC artifact"]
    fn truffle_compiled_foundation_halc_loads_with_foundation_semantics() {
        let artifact = std::env::var("HARA_TRUFFLE_FOUNDATION_HALC")
            .or_else(|_| std::env::var("HARA_TRUFFLE_FOUNDATION_HIR"))
            .expect("HARA_TRUFFLE_FOUNDATION_HALC must point to the compiled artifact");
        let bytes = std::fs::read(&artifact).expect("read Truffle-compiled foundation HALC");
        let mut runtime = Runtime::new();

        assert_eq!(runtime.eval_halc(&bytes).unwrap(), "<fn>");
        assert_eq!(runtime.eval_native("((comp inc inc) 40)").unwrap(), "42");
    }

    #[test]
    fn foundation_environment_facade_inspects_without_loading_registered_namespaces() {
        let mut runtime = Runtime::new();
        runtime.register_resource("example.unloaded", "(ns example.unloaded) (def answer 42)");
        runtime.eval_native("(def local-value 7)").unwrap();

        assert_eq!(
            runtime
                .eval_native(
                    "(get (std.foundation/ns-info 'example.unloaded) :namespace/state)",
                )
                .unwrap(),
            ":unloaded"
        );
        assert_eq!(
            runtime
                .eval_native("(std.foundation/resolve 'example.unloaded/answer)")
                .unwrap(),
            "nil"
        );
        assert!(runtime
            .eval_native("(std.foundation/ns-vars)")
            .unwrap()
            .contains("local-value"));
        assert_eq!(
            runtime
                .eval_native("(get (std.foundation/ns-info 'example.unloaded) :namespace/state)")
                .unwrap(),
            ":unloaded"
        );
        assert_eq!(
            runtime
                .eval_native("example.unloaded/answer")
                .unwrap_err()
                .contains("unbound"),
            true
        );
    }

    #[test]
    fn package_facade_uses_the_existing_host_capability_boundary() {
        let mut runtime = Runtime::new();
        let lock = format!(
            "{{:lock/format \"0.0.0-alpha\" :packages {{\"hara:example/package\" {{:version \"1.0.0\" :tap \"hara\" :registry-commit \"{}\" :identity-revision \"{}\" :archive-sha256 \"sha256:{}\" :namespaces [example.unloaded]}}}}}}",
            "a".repeat(40),
            "b".repeat(40),
            "c".repeat(64)
        );
        runtime.register_package_lock(&lock).unwrap();
        runtime.install_native_host_handler(Rc::new(|service, operation, arguments| {
            assert_eq!(service, "package");
            assert!(operation == "ensure" || operation == "unload");
            assert!(!arguments.is_empty());
            let promise = core::Promise::new();
            promise.resolve(if operation == "ensure" {
                core::Value::Keyword("ready".into())
            } else {
                core::Value::Vector(PVector::from(vec![core::Value::String(
                    "hara:example/package".into(),
                )]))
            });
            Ok(core::Value::Promise(promise))
        }));
        assert_eq!(
            runtime
                .eval_native("(get (Package/find 'example.unloaded) :package/state)")
                .unwrap(),
            ":available"
        );
        assert_eq!(
            runtime.eval_native("(ns-state 'example.unloaded)").unwrap(),
            ":unloaded"
        );
        let missing = runtime
            .eval_native("(require 'example.unloaded)")
            .unwrap_err();
        assert!(missing.contains("package/not-installed"), "{missing}");
        assert_eq!(
            runtime.eval_native("(ns-state 'example.unloaded)").unwrap(),
            ":unloaded"
        );
        runtime.register_resource("example.unloaded", "(ns example.unloaded) (def answer 42)");
        assert_eq!(
            runtime
                .eval_native("(deref (Package/ensure 'example.unloaded))")
                .unwrap(),
            ":ready"
        );
        assert_eq!(
            runtime
                .eval_native("(Package/load 'example.unloaded)")
                .unwrap(),
            "example.unloaded"
        );
        assert_eq!(
            runtime.eval_native("example.unloaded/answer").unwrap(),
            "42"
        );
        assert_eq!(
            runtime
                .eval_native("(deref (Package/unload 'example.unloaded {:cascade false}))")
                .unwrap(),
            "[\"hara:example/package\"]"
        );
        assert_eq!(
            runtime
                .eval_native("(Package/state 'example.unloaded)")
                .unwrap(),
            ":available"
        );
    }

    #[cfg(feature = "evaluation-journal")]
    #[test]
    fn evaluation_journal_uses_the_real_macro_and_invocation_paths() {
        let mut runtime = Runtime::new();
        let trace =
            runtime.eval_native_journal("(defn observed [x] x) (if-not false (observed 5))");

        assert_eq!(trace.schema, crate::journal::SCHEMA);
        assert_eq!(trace.result.as_ref().unwrap().display, "5");
        assert!(trace.events.iter().any(|event| {
            event.kind == crate::journal::JournalEventKind::MacroExpand
                && event.function.as_deref() == Some("if-not")
        }));
        assert!(trace.events.iter().any(|event| {
            event.kind == crate::journal::JournalEventKind::OperationEnter
                && event.function.as_deref() == Some("observed")
                && event
                    .values
                    .first()
                    .is_some_and(|value| value.display == "5")
        }));
        assert!(trace.events.iter().any(|event| {
            event.kind == crate::journal::JournalEventKind::OperationReturn
                && event.function.as_deref() == Some("observed")
                && event
                    .values
                    .first()
                    .is_some_and(|value| value.display == "5")
        }));
    }
}

#[test]
fn native_bytes_calls_terminate_without_reentering_the_hal_facade() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval(
                "(let [native (Bytes/new 1 -1)]\n\
                 [(Bytes/count native)\n\
                  (Bytes/get native 1)\n\
                  (bytes/count native)\n\
                  (bytes/u8 -1)])"
            )
            .unwrap()
            .to_string(),
        "[2 255 2 255]"
    );
}
