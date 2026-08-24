use std::cell::Cell;
use std::rc::Rc;

use hara_wasm::extension::{ExtensionManifest, Promise, Value, WasmAbi, WasmExtensionProvider};
use hara_wasm::wasm_binding::{generate_hta_adapter, WasmInterface};
use hara_wasm::Runtime;

const START_WASM: &[u8] = b"\0asm\x01\0\0\0\x08\x01\0";

const CORE_MANIFEST: &str = r#"
{:namespace "fixture.math"
 :version "1.0.0"
 :provider :wasm
 :module "math.wasm"
 :abi :core.v1
 :exports {"add" {:args [:integer :integer] :returns :integer :async false}}
 :capabilities []}"#;

const ASYNC_MANIFEST: &str = r#"
{:namespace "fixture.async"
 :version "1.0.0"
 :provider :wasm
 :module "async.wasm"
 :abi :hta.v1
 :exports {"later" {:args [:integer] :returns :integer :async true}}
 :capabilities [:clock]}"#;

#[derive(Clone)]
struct FixtureProvider {
    abi: WasmAbi,
    capabilities: Vec<String>,
    starts: Rc<Cell<usize>>,
    invokes: Rc<Cell<usize>>,
    shutdowns: Rc<Cell<usize>>,
    cancellations: Rc<Cell<usize>>,
    return_promise: bool,
}

impl FixtureProvider {
    fn core() -> Self {
        Self {
            abi: WasmAbi::CoreV1,
            capabilities: Vec::new(),
            starts: Rc::new(Cell::new(0)),
            invokes: Rc::new(Cell::new(0)),
            shutdowns: Rc::new(Cell::new(0)),
            cancellations: Rc::new(Cell::new(0)),
            return_promise: false,
        }
    }
}

impl WasmExtensionProvider for FixtureProvider {
    fn supports(&self, abi: WasmAbi) -> bool {
        self.abi == abi
    }

    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }

    fn start(&self, _manifest: &ExtensionManifest) -> Result<(), String> {
        self.starts.set(self.starts.get() + 1);
        Ok(())
    }

    fn invoke(
        &self,
        _manifest: &ExtensionManifest,
        _export: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        self.invokes.set(self.invokes.get() + 1);
        let result = arguments
            .iter()
            .map(|value| match value {
                Value::Number(value) => Ok(*value),
                _ => Err("fixture/type: expected integer".to_owned()),
            })
            .sum::<Result<i64, _>>()?;
        if self.return_promise {
            let promise = Promise::new();
            promise.resolve(Value::Number(result));
            Ok(Value::Promise(promise))
        } else {
            Ok(Value::Number(result))
        }
    }

    fn cancel(&self, _manifest: &ExtensionManifest, _request: u64) -> Result<(), String> {
        self.cancellations.set(self.cancellations.get() + 1);
        Ok(())
    }

    fn shutdown(&self, _manifest: &ExtensionManifest) {
        self.shutdowns.set(self.shutdowns.get() + 1);
    }
}

#[test]
fn ordinary_require_installs_alias_and_referred_exports_once() {
    let provider = FixtureProvider::core();
    let counts = provider.clone();
    {
        let mut runtime = Runtime::new();
        runtime
            .install_wasm_extension(CORE_MANIFEST, "fixture/manifest", provider)
            .unwrap();
        assert_eq!(
            runtime
                .eval_native(
                    "(ns fixture.user
                       (:require [fixture.math :as math :refer [add]]))
                     [(math/add 19 23) (add 20 22)]"
                )
                .unwrap(),
            "[42 42]"
        );
        assert_eq!(counts.starts.get(), 1);
        assert_eq!(counts.invokes.get(), 2);
        assert_eq!(counts.shutdowns.get(), 0);
        runtime.cancel_wasm_extension("fixture.math", 7).unwrap();
        assert_eq!(counts.cancellations.get(), 1);
        assert_eq!(
            runtime
                .eval_native("(ns fixture.other (:require [fixture.math :as math])) (math/add 1 2)")
                .unwrap(),
            "3"
        );
        assert_eq!(counts.starts.get(), 1);
    }
    assert_eq!(counts.shutdowns.get(), 1);
}

#[test]
fn capabilities_abi_arity_and_async_contracts_fail_stably() {
    let denied = FixtureProvider {
        abi: WasmAbi::HtaV1,
        capabilities: Vec::new(),
        return_promise: true,
        ..FixtureProvider::core()
    };
    let denied_counts = denied.clone();
    let mut runtime = Runtime::new();
    runtime
        .install_wasm_extension(ASYNC_MANIFEST, "fixture/async", denied)
        .unwrap();
    assert_eq!(
        runtime
            .eval_native("(ns denied (:require [fixture.async :as async]))")
            .unwrap_err(),
        "extension/denied: fixture.async requires capability :clock"
    );
    assert_eq!(denied_counts.starts.get(), 0);

    let provider = FixtureProvider::core();
    let mut runtime = Runtime::new();
    runtime
        .install_wasm_extension(CORE_MANIFEST, "fixture/core", provider)
        .unwrap();
    assert!(runtime
        .eval_native("(ns arity (:require [fixture.math :refer [add]])) (add 1)")
        .unwrap_err()
        .contains("expects 2 arguments"));

    let immediate = FixtureProvider {
        abi: WasmAbi::HtaV1,
        capabilities: vec!["clock".into()],
        ..FixtureProvider::core()
    };
    let mut runtime = Runtime::new();
    runtime
        .install_wasm_extension(ASYNC_MANIFEST, "fixture/async", immediate)
        .unwrap();
    assert_eq!(
        runtime
            .eval_native("(ns async (:require [fixture.async :as ext])) (ext/later 42)")
            .unwrap_err(),
        "extension/protocol: asynchronous export fixture.async/later must return a promise"
    );
}

#[test]
fn hta_async_export_returns_a_hara_promise() {
    let provider = FixtureProvider {
        abi: WasmAbi::HtaV1,
        capabilities: vec!["clock".into()],
        return_promise: true,
        ..FixtureProvider::core()
    };
    let mut runtime = Runtime::new();
    runtime
        .install_wasm_extension(ASYNC_MANIFEST, "fixture/async", provider)
        .unwrap();
    assert_eq!(
        runtime
            .eval_native(
                "(ns async (:require [fixture.async :as ext :refer [later]]))
                 [(deref (ext/later 19)) (deref (later 23))]"
            )
            .unwrap(),
        "[19 23]"
    );
}

#[test]
fn duplicate_namespaces_and_unsupported_abis_are_rejected() {
    let mut runtime = Runtime::new();
    runtime
        .install_wasm_extension(CORE_MANIFEST, "fixture/core", FixtureProvider::core())
        .unwrap();
    assert_eq!(
        runtime
            .install_wasm_extension(CORE_MANIFEST, "fixture/core", FixtureProvider::core())
            .unwrap_err(),
        "extension/ambiguous: namespace already registered: fixture.math"
    );
    let unsupported = FixtureProvider {
        abi: WasmAbi::HtaV1,
        ..FixtureProvider::core()
    };
    assert_eq!(
        Runtime::new()
            .install_wasm_extension(CORE_MANIFEST, "fixture/core", unsupported)
            .unwrap_err(),
        "extension/unsupported: provider does not support CoreV1"
    );
}

#[test]
fn hta_adapter_rejects_wrapped_modules_with_start_functions() {
    let interface = WasmInterface::parse(
        r#"
        (wasm/interface
         {:schema "hara.wasm-interface/0-alpha"
          :namespace math.scalar
          :module "math.wasm"
          :exports
          {sum {:wasm/export "add"
                :async true
                :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                            {:name right :hara/type :i64 :wasm/type :i64}]
                :returns {:hara/type :i64 :wasm/type :i64}}}})
        "#,
        "fixture",
    )
    .unwrap();

    let error = generate_hta_adapter(START_WASM, &interface).unwrap_err();
    assert!(error.starts_with("wasm-adapter/start-denied"));
}
