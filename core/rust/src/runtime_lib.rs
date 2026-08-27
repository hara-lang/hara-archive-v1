#![allow(clippy::too_many_lines)] // Temporary compatibility facade during Java-port split.
#[cfg(not(target_arch = "wasm32"))]
pub mod asset;
#[cfg(not(target_arch = "wasm32"))]
pub mod cli_app;
// Public embedding surface used by native hosts such as Hoplite. The module's
// value, protocol, promise, and host-call types form the runtime integration ABI.
#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
mod browser_wasm_provider;
mod clock;
pub mod compiled_product;
pub mod core;
mod direct_wasm;
pub mod extension;
pub mod file;
#[path = "file/interface.rs"]
pub mod filesystem;
#[path = "runtime/filesystem_bridge.rs"]
mod filesystem_bridge;
#[path = "runtime/filesystem_mount.rs"]
mod filesystem_mount;
#[path = "runtime/filesystem_adapter.rs"]
pub mod filesystem_runtime;
pub mod hta;
#[cfg(not(target_arch = "wasm32"))]
pub mod invoke_hta;
pub mod wasm_binding;
#[cfg(not(target_arch = "wasm32"))]
pub use invoke_hta::{InvokeHtaError, MAX_INVOKE_HTA_RESULT_BYTES};
#[cfg(not(target_arch = "wasm32"))]
pub mod identity_tool;
pub mod instrumentation;
pub mod interpreter_observation;
#[cfg(feature = "evaluation-journal")]
pub mod journal;
pub mod json;
pub mod kernel;
pub mod lang;
pub mod live_session;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_cli;
#[cfg(not(target_arch = "wasm32"))]
mod native_extension;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_link;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_module;
#[cfg(not(target_arch = "wasm32"))]
mod native_process;
mod numeric;
#[cfg(not(target_arch = "wasm32"))]
pub mod package;
pub mod package_catalog;
#[cfg(not(target_arch = "wasm32"))]
pub mod package_hta_loader;
#[cfg(not(target_arch = "wasm32"))]
pub mod package_manifest;
#[cfg(not(target_arch = "wasm32"))]
pub mod package_wasm_loader;
#[cfg(not(target_arch = "wasm32"))]
mod process_extension;
#[cfg(not(target_arch = "wasm32"))]
pub mod project;
#[cfg(not(target_arch = "wasm32"))]
pub mod resp;
pub mod snapshot;
#[cfg(not(target_arch = "wasm32"))]
pub mod snapshot_tool;
pub mod spec_registry;
#[cfg(not(target_arch = "wasm32"))]
pub mod tap;
pub mod task;
pub mod work;
#[path = "work/session.rs"]
mod work_session;
// Experimental staged bytecode VM (issue #195). Non-default feature; the
// default evaluator is untouched.
#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
pub mod direct_native;
#[cfg(feature = "bytecode-vm")]
#[path = "vm/schema_catalog.rs"]
pub mod hbc_schema_catalog;
#[cfg(feature = "bytecode-vm")]
#[path = "vm/schema_links.rs"]
pub mod hbc_schema_links;
#[cfg(feature = "tracing-jit")]
pub mod jit;
#[cfg(feature = "bytecode-vm")]
pub mod vm;
#[cfg(not(target_arch = "wasm32"))]
pub mod wasmtime_provider;
#[cfg(feature = "whole-wasm")]
pub mod whole_wasm;
use crate::kernel::Form;
use crate::lang::data::{OrderedMap as POrderedMap, Vector as PVector};
use crate::lang::protocol::INamespaced;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[cfg(feature = "raw-wasm")]
#[derive(Debug, Clone)]
pub struct JsValue;

#[cfg(feature = "raw-wasm")]
impl JsValue {
    fn from_str(_value: &str) -> Self {
        Self
    }
}

include!("runtime/execution_state.rs");
include!("runtime/model.rs");
include!("runtime/session_model.rs");
include!("runtime/sandbox_model.rs");
include!("runtime/session.rs");
include!("runtime/session_live.rs");
include!("runtime/session_instrumentation.rs");
include!("runtime/sandbox.rs");
include!("runtime/runtime.rs");
include!("runtime/bytecode.rs");
include!("runtime/evaluation.rs");
include!("runtime/wasm.rs");

/// Constructs the zero-authority Runtime profile used by an external secure
/// [`SandboxProvider`] evaluator process.
#[cfg(not(target_arch = "wasm32"))]
pub fn restricted_sandbox_runtime() -> Runtime {
    Runtime::sandbox()
}

/// Constructs the same restricted Runtime while restoring exactly the fully
/// qualified `std.native.Host/call` operation behind a caller-supplied handler.
#[cfg(not(target_arch = "wasm32"))]
pub fn restricted_sandbox_runtime_with_host(
    handler: Rc<dyn Fn(String, String, Vec<core::Value>) -> Result<core::Value, String>>,
) -> Runtime {
    let mut runtime = Runtime::sandbox();
    runtime
        .namespace_registry
        .find_or_create("std.native.Host")
        .intern_with_origin(
            "call",
            core::native_type_function_value("Host", "call")
                .expect("std.native.Host/call must have a direct implementation"),
            kernel::VarOrigin::RuntimePrimitive,
        );
    runtime.install_native_host_handler(handler);
    runtime
}

#[cfg(test)]
#[path = "runtime/filesystem_tests.rs"]
mod filesystem_runtime_tests;
include!("runtime/tests.rs");

#[cfg(test)]
mod native_behavioral_conformance_tests;
