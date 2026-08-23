//! Restricted `.hal` interface contracts for portable Wasm extension bindings.
//!
//! Sources are parsed as data with the Hara reader. This module never evaluates
//! an interface, instantiates a module, or acquires host authority.

#[cfg(not(target_arch = "wasm32"))]
mod adapter;
mod canonical;
mod direct;
mod memory;
#[cfg(not(target_arch = "wasm32"))]
mod package;
mod parser;
#[cfg(not(target_arch = "wasm32"))]
mod runtime;
mod syntax;
pub mod wit;
mod wit_format;
mod wit_parser;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::extension::ExtensionExport;

pub use crate::direct_wasm::{
    DirectWasmFunctionExport, DirectWasmImport, DirectWasmImportKind, DirectWasmInspection,
    DirectWasmMemory,
};
#[cfg(not(target_arch = "wasm32"))]
pub use adapter::{
    generate_adapter, generate_hta_adapter, verify_hta_scalar, AdapterArtifact,
    ADAPTER_MANIFEST_SCHEMA,
};
pub use direct::{
    direct_inspection_source, direct_interface_skeleton, inspect_direct,
    DIRECT_WASM_INSPECTION_SCHEMA,
};
pub use memory::{
    MemoryArgumentPlan, MemoryBindingPlan, MemoryFunctionPlan, MemoryResultPlan,
    MEMORY_BINDING_SCHEMA,
};
#[cfg(not(target_arch = "wasm32"))]
pub use package::{
    bind_package, inspect_module, write_interface_skeleton, BindingTarget, BoundPackage,
    InspectionArtifact, DIRECT_WASM_BINDING_SCHEMA, DIRECT_WASM_BUILD_PRODUCT_SCHEMA,
    DIRECT_WASM_CONFORMANCE_SCHEMA,
};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::WasmtimeMemoryExecutor;
pub use wit::{
    import_wit, project_wit, WitDiagnostic, WitDiagnosticSeverity, WitImportArtifact,
    WitImportOptions, WitProjectionArtifact, WitProjectionOptions, WitRoute, WIT_IR_SCHEMA,
    WIT_MANIFEST_SCHEMA,
};

pub const WASM_INTERFACE_SCHEMA: &str = "hara.wasm-interface/0-alpha";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WasmValueType {
    I32,
    I64,
    F32,
    F64,
    Void,
}

impl WasmValueType {
    pub fn as_keyword(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Void => "void",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum HaraValueType {
    I32,
    I64,
    F32,
    F64,
    Boolean,
    String,
    Bytes,
    Record(String),
    Variant(String),
    Handle(String),
    Callback(String),
    Void,
}

impl HaraValueType {
    fn direct_wasm_type(&self) -> Option<WasmValueType> {
        match self {
            Self::I32 | Self::Boolean => Some(WasmValueType::I32),
            Self::I64 => Some(WasmValueType::I64),
            Self::F32 => Some(WasmValueType::F32),
            Self::F64 => Some(WasmValueType::F64),
            Self::Void => Some(WasmValueType::Void),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Borrowed,
    Caller,
    Callee,
    Transferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lowering {
    Direct,
    PointerLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifting {
    Direct,
    PointerLength,
    PackedI64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryContract {
    pub export: String,
    pub allocate: Option<String>,
    pub reallocate: Option<String>,
    pub release: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingParameter {
    pub name: String,
    pub hara_type: HaraValueType,
    pub wasm_type: WasmValueType,
    pub lowering: Option<Lowering>,
    pub ownership: Option<Ownership>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingResult {
    pub hara_type: HaraValueType,
    pub wasm_type: WasmValueType,
    pub lifting: Option<Lifting>,
    pub ownership: Option<Ownership>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorContract {
    pub convention: String,
    pub codes: BTreeMap<i64, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestPolicy {
    pub timeout_ms: Option<u64>,
    pub max_in_flight: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationPolicy {
    Cooperative,
    Abort,
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncPolicy {
    pub operation: String,
    pub request: RequestPolicy,
    pub cancellation: CancellationPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCallContract {
    pub methods: BTreeSet<String>,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackParameter {
    pub name: String,
    pub hara_type: HaraValueType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackContract {
    pub arguments: Vec<CallbackParameter>,
    pub returns: HaraValueType,
    pub reentrant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandleContract {
    pub tag: String,
    pub release: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingFunction {
    pub name: String,
    pub wasm_export: String,
    pub arguments: Vec<BindingParameter>,
    pub returns: BindingResult,
    pub asynchronous: bool,
    pub operation: Option<String>,
    pub request: Option<RequestPolicy>,
    pub cancellation: Option<CancellationPolicy>,
    pub errors: Option<ErrorContract>,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmInterface {
    pub schema: String,
    pub namespace: String,
    pub module: String,
    pub memory: Option<MemoryContract>,
    pub exports: Vec<BindingFunction>,
    pub capabilities: BTreeSet<String>,
    pub host_calls: BTreeMap<String, HostCallContract>,
    pub callbacks: BTreeMap<String, CallbackContract>,
    pub handles: BTreeMap<String, HandleContract>,
    pub resources: BTreeMap<String, HandleContract>,
}

impl WasmInterface {
    pub fn parse(source: &str, origin: &str) -> Result<Self, String> {
        parser::parse_interface(source, origin)
    }

    pub fn canonical_source(&self) -> String {
        canonical::source(self)
    }

    pub fn digest(&self) -> String {
        let digest = Sha256::digest(self.canonical_source().as_bytes());
        format!("sha256:{digest:x}")
    }

    pub fn hta_required(&self) -> bool {
        !self.host_calls.is_empty()
            || !self.callbacks.is_empty()
            || !self.handles.is_empty()
            || !self.resources.is_empty()
            || self.exports.iter().any(|export| {
                export.asynchronous
                    || export.operation.is_some()
                    || export.request.is_some()
                    || export.cancellation.is_some()
            })
    }

    pub fn direct_exports(&self) -> Vec<(String, ExtensionExport)> {
        self.exports
            .iter()
            .map(|export| {
                (
                    export.wasm_export.clone(),
                    ExtensionExport {
                        arguments: export
                            .arguments
                            .iter()
                            .map(|argument| argument.wasm_type.as_keyword().to_owned())
                            .collect(),
                        returns: export.returns.wasm_type.as_keyword().to_owned(),
                        asynchronous: false,
                        raw_export: None,
                    },
                )
            })
            .collect()
    }
}
