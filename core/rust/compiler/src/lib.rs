//! Full Hara compilation surface, kept outside VM-only deployments.

pub use hara_vm::{Program, Value};
pub use hara_wasm::compiled_product::{
    sha256_hex, CompiledProduct, CompiledProductKind, CompiledProductManifest,
    InMemoryProductCache, ProductCacheKey,
};
pub use hara_wasm::vm::{compile_halc_module, compile_source, compile_source_with, CompileError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileTarget {
    HbcModule,
    WholeWasm,
}

impl CompileTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HbcModule => "hbc-module",
            Self::WholeWasm => "whole-wasm",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledArtifact {
    target: CompileTarget,
    product: CompiledProduct,
}

fn product_identity(target: CompileTarget) -> (CompiledProductKind, &'static str) {
    match target {
        CompileTarget::HbcModule => (CompiledProductKind::HbcModule, "hbc0"),
        CompileTarget::WholeWasm => (CompiledProductKind::WholeWasm, "hnw0/2"),
    }
}

impl CompiledArtifact {
    pub fn target(&self) -> CompileTarget {
        self.target
    }

    pub fn bytes(&self) -> &[u8] {
        &self.product.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.product.bytes
    }

    pub fn manifest(&self) -> &CompiledProductManifest {
        &self.product.manifest
    }
}

pub fn compile(source: &str, target: CompileTarget) -> Result<CompiledArtifact, String> {
    let product = compile_product(source, target)?;
    Ok(CompiledArtifact { target, product })
}

pub fn compile_cached(
    source: &str,
    target: CompileTarget,
    cache: &mut InMemoryProductCache,
) -> Result<CompiledArtifact, String> {
    let (kind, abi_version) = product_identity(target);
    let key = ProductCacheKey::new(
        kind,
        sha256_hex(source.as_bytes()),
        format!("hara-compiler/{}", env!("CARGO_PKG_VERSION")),
        abi_version,
        b"{}",
    );
    if let Some(product) = cache.get(&key) {
        return Ok(CompiledArtifact {
            target,
            product: product.clone(),
        });
    }
    let product = compile_product(source, target)?;
    cache.insert(product.clone())?;
    Ok(CompiledArtifact { target, product })
}

pub fn compile_product(source: &str, target: CompileTarget) -> Result<CompiledProduct, String> {
    let program = compile_source(source).map_err(|error| error.to_string())?;
    let (product, abi_version, bytes) = match target {
        CompileTarget::HbcModule => (
            CompiledProductKind::HbcModule,
            "hbc0",
            hara_wasm::vm::encode_program(&program)?,
        ),
        CompileTarget::WholeWasm => (
            CompiledProductKind::WholeWasm,
            "hnw0/2",
            compile_whole_wasm(&program)?,
        ),
    };
    Ok(CompiledProduct::new(
        product,
        sha256_hex(source.as_bytes()),
        vec![sha256_hex(source.as_bytes())],
        format!("hara-compiler/{}", env!("CARGO_PKG_VERSION")),
        abi_version,
        b"{}",
        bytes,
    ))
}

fn compile_bytes(source: &str, target: CompileTarget) -> Result<Vec<u8>, String> {
    let program = compile_source(source).map_err(|error| error.to_string())?;
    Ok(match target {
        CompileTarget::HbcModule => hara_wasm::vm::encode_program(&program)?,
        CompileTarget::WholeWasm => compile_whole_wasm(&program)?,
    })
}

pub fn compile_bytecode(source: &str) -> Result<Vec<u8>, String> {
    compile_bytes(source, CompileTarget::HbcModule)
}

#[cfg(feature = "full-wasm")]
pub fn compile_wasm(source: &str) -> Result<Vec<u8>, String> {
    compile_bytes(source, CompileTarget::WholeWasm)
}

#[cfg(feature = "full-wasm")]
fn compile_whole_wasm(program: &Program) -> Result<Vec<u8>, String> {
    hara_wasm::whole_wasm::compile_artifact(program)
}

#[cfg(not(feature = "full-wasm"))]
fn compile_whole_wasm(_program: &Program) -> Result<Vec<u8>, String> {
    Err("whole-wasm compilation requires the hara-compiler/full-wasm feature".into())
}

#[cfg(test)]
mod tests {
    use super::{
        compile, compile_bytecode, CompiledProductKind, CompileTarget, InMemoryProductCache,
    };

    #[test]
    fn compiler_output_executes_in_vm_only_crate() {
        let artifact = compile_bytecode("(+ 19 23)").unwrap();
        assert_eq!(hara_vm::execute(&artifact).unwrap().display(), "42");
    }

    #[test]
    fn explicit_hbc_target_preserves_the_legacy_bytecode_contract() {
        let artifact = compile("(+ 19 23)", CompileTarget::HbcModule).unwrap();
        assert_eq!(artifact.target(), CompileTarget::HbcModule);
        assert_eq!(hara_vm::execute(artifact.bytes()).unwrap().display(), "42");
        assert_eq!(artifact.manifest().product, CompiledProductKind::HbcModule);
        artifact.product.verify().unwrap();
    }

    #[test]
    fn cached_compilation_reuses_the_immutable_product() {
        let mut cache = InMemoryProductCache::default();
        let first =
            super::compile_cached("(+ 19 23)", CompileTarget::HbcModule, &mut cache).unwrap();
        let second =
            super::compile_cached("(+ 19 23)", CompileTarget::HbcModule, &mut cache).unwrap();
        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.manifest(), second.manifest());
        assert_eq!(cache.len(), 1);
    }

    #[cfg(not(feature = "full-wasm"))]
    #[test]
    fn whole_wasm_target_requires_its_explicit_feature() {
        let error = compile("(+ 19 23)", CompileTarget::WholeWasm).unwrap_err();
        assert!(error.contains("full-wasm"));
    }

    #[cfg(feature = "full-wasm")]
    #[test]
    fn whole_wasm_target_reports_its_product_identity() {
        let artifact = compile("(+ 19 23)", CompileTarget::WholeWasm).unwrap();
        assert_eq!(artifact.target(), CompileTarget::WholeWasm);
        assert!(!artifact.bytes().is_empty());
    }
}
