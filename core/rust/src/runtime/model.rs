/// Marks every namespace already materialized by the runtime bootstrap as loaded.
///
/// Embedding hosts receive only the namespace and protocol registries, not the
/// Runtime's source-provider state. A source fallback can therefore leave a
/// materialized Foundation namespace recorded as `Unloaded` even though all of
/// its Vars are present. Normalizing only materialized namespaces preserves
/// lazy package discovery while making the exported registry self-contained.
fn normalize_embedding_namespace_load_states(namespaces: &kernel::NamespaceRegistry<core::Value>) {
    for namespace in namespaces.all() {
        namespaces.set_load_state(
            namespace.name().as_str(),
            kernel::NamespaceLoadState::Loaded,
        );
    }
}

/// Builds the fully bootstrapped namespace registry used by native embedding hosts.
///
/// Hosts receive the same Foundation Vars, primitive values and protocol wiring
/// as a normal Hara runtime without depending on crate-private bootstrap helpers.
pub fn embedding_namespace_registry() -> kernel::NamespaceRegistry<core::Value> {
    let namespaces = Runtime::new().namespace_registry.clone();
    normalize_embedding_namespace_load_states(&namespaces);
    namespaces
}

#[cfg(test)]
mod embedding_namespace_tests {
    use super::*;

    #[test]
    fn normalization_marks_only_materialized_namespaces_loaded() {
        let namespaces = kernel::NamespaceRegistry::<core::Value>::new("user");
        namespaces.find_or_create("std.foundation");
        namespaces.set_load_state("std.foundation", kernel::NamespaceLoadState::Unloaded);
        namespaces.set_load_state("example.lazy", kernel::NamespaceLoadState::Unloaded);

        normalize_embedding_namespace_load_states(&namespaces);

        assert_eq!(
            namespaces.load_state("std.foundation"),
            Some(kernel::NamespaceLoadState::Loaded)
        );
        assert_eq!(
            namespaces.load_state("example.lazy"),
            Some(kernel::NamespaceLoadState::Unloaded)
        );
    }

    #[test]
    fn exported_registry_satisfies_bootstrap_requires_without_source_provider() {
        let namespaces = embedding_namespace_registry();
        let form = kernel::parse_forms(
            "(ns example.embedding (:require [std.foundation :refer :all] [std.foundation.coroutine :as coroutine]))",
        )
        .expect("parse embedding namespace declaration")
        .into_iter()
        .next()
        .expect("embedding namespace declaration");
        let mut environment = std::collections::HashMap::new();

        core::with_namespace_registry(&namespaces, || core::eval(&form, &mut environment))
            .expect("embedding registry must satisfy bootstrapped requires");

        assert_eq!(
            namespaces.load_state("std.foundation"),
            Some(kernel::NamespaceLoadState::Loaded)
        );
        assert_eq!(
            namespaces.load_state("std.foundation.coroutine"),
            Some(kernel::NamespaceLoadState::Loaded)
        );
        assert!(namespaces.find("example.embedding").is_some());
    }
}

#[cfg(not(feature = "raw-wasm"))]
use wasm_bindgen::prelude::*;

include!(concat!(env!("OUT_DIR"), "/embedded_hal.rs"));

const EAGER_HAL_RESOURCES: &[&str] = &[
    "std.foundation.string",
    "std.foundation.promise",
    "std.foundation.bytes",
    "std.foundation.coroutine",
    "std.foundation.pretty",
    "std.stream.duplex",
];

fn ignore_socket_event(_event: core::SocketEvent) {}

#[cfg(not(feature = "raw-wasm"))]
#[wasm_bindgen(start)]
pub fn init_wasm() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
}

/// Runs the shared instrumentation corpus inside the browser/Wasm runtime.
///
/// The returned report is produced by the runtime-owned instrumentation hub,
/// not by JavaScript projection logic. Hosts can therefore compare repeated
/// browser runs with the native Rust and Java reports byte-for-byte.
#[cfg(not(feature = "raw-wasm"))]
#[wasm_bindgen]
pub fn instrumentation_conformance(corpus: &str) -> Result<String, JsValue> {
    let corpus: serde_json::Value =
        serde_json::from_str(corpus).map_err(|error| JsValue::from_str(&error.to_string()))?;
    let report = crate::instrumentation::conformance::report(&corpus, "wasm")
        .map_err(|error| JsValue::from_str(&error))?;
    serde_json::to_string_pretty(&report).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen)]
pub struct PromiseHandle {
    promise: core::Promise,
}

#[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen)]
impl PromiseHandle {
    fn from_promise(promise: core::Promise) -> PromiseHandle {
        PromiseHandle { promise }
    }

    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(constructor))]
    pub fn new() -> PromiseHandle {
        PromiseHandle {
            promise: core::Promise::new(),
        }
    }

    pub fn state(&self) -> String {
        match self.promise.state() {
            core::PromiseState::Pending => "pending".into(),
            core::PromiseState::Fulfilled(_) => "fulfilled".into(),
            core::PromiseState::Rejected(_) => "rejected".into(),
        }
    }

    pub fn resolve(&self, value: &str) -> bool {
        self.promise.resolve(core::Value::String(value.into()))
    }

    pub fn reject(&self, error: &str) -> bool {
        self.promise.reject(error)
    }

    pub fn adopt(&self, other: &PromiseHandle) -> bool {
        self.promise.adopt(&other.promise)
    }

    pub fn value(&self) -> Result<String, JsValue> {
        match self.promise.state() {
            core::PromiseState::Pending => Err(JsValue::from_str("promise is pending")),
            core::PromiseState::Fulfilled(value) => Ok(value.display()),
            core::PromiseState::Rejected(error) => Err(JsValue::from_str(&error.message())),
        }
    }
}

#[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen)]
pub struct Runtime {
    execution: RuntimeExecutionState,
    test_runner: String,
    execution_backend: String,
    protocols: core::ProtocolRegistry,
    extensions: core::ExtensionRegistry,
    wasm_extensions: HashMap<String, extension::WasmExtension>,
    native_wasm_imports: HashMap<String, extension::WasmExtension>,
    providers: core::ProviderRegistry,
    package_catalog: core::PackageCatalog,
    resources: HashMap<String, String>,
    #[cfg(not(target_arch = "wasm32"))]
    source_paths: HashMap<String, std::path::PathBuf>,
    resource_overrides: HashSet<String>,
    #[cfg(feature = "bytecode-vm")]
    bytecode_resources: HashMap<String, (String, Vec<u8>)>,
    product_cache: RefCell<compiled_product::InMemoryProductCache>,
    loaded_resources: HashSet<String>,
    halc_schema_definitions: HashMap<String, Form>,
    halc_function_schemas: HashMap<String, Form>,
    halc_schema_types: HashMap<String, kernel::SchemaType>,
    halc_function_types: HashMap<String, kernel::SchemaType>,
    halc_inferred_function_types: HashMap<String, kernel::SchemaType>,
    namespace_registry: kernel::NamespaceRegistry<core::Value>,
    macros: Rc<RefCell<HashMap<(String, String), Rc<core::Function>>>>,
    generated_configs: HashMap<String, kernel::GeneratedNamespaceConfig>,
    #[cfg(feature = "evaluation-journal")]
    next_journal_id: u64,
    #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
    host_handler: Option<js_sys::Function>,
    #[cfg(not(target_arch = "wasm32"))]
    native_host_handler:
        Option<Rc<dyn Fn(String, String, Vec<core::Value>) -> Result<core::Value, String>>>,
    #[cfg(not(target_arch = "wasm32"))]
    native_modules: native_module::Registry,
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    direct_native: crate::direct_native::NativeEngine,
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    direct_native_multimethods: core::MultiMethodRegistry,
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    direct_native_source_cache: Option<SourceBytecodeCache>,
    #[cfg(not(target_arch = "wasm32"))]
    extension_roots: Vec<std::path::PathBuf>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // Namespace vars and the flattened environment retain native export
        // closures, and those closures retain the extension session. Release
        // the bindings before dropping the session owners so provider
        // shutdown is deterministic at the Runtime boundary.
        let namespaces = self.wasm_extensions.keys().cloned().collect::<Vec<_>>();
        for namespace in self.namespace_registry.all() {
            for (symbol, var) in namespace.mappings() {
                if var
                    .symbol()
                    .get_namespace()
                    .is_some_and(|owner| namespaces.iter().any(|extension| extension == owner))
                {
                    namespace.unmap(&symbol);
                }
            }
        }
        self.execution.clear();
        #[cfg(not(target_arch = "wasm32"))]
        self.native_wasm_imports.clear();
        self.wasm_extensions.clear();
    }
}
