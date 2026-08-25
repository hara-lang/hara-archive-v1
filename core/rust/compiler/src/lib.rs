//! Full Hara compilation surface, kept outside VM-only deployments.

pub use hara_runtime::compiled_product::{
    sha256_hex, CompiledProduct, CompiledProductKind, CompiledProductManifest,
    InMemoryProductCache, ProductCacheKey,
};
use hara_runtime::vm::{compile_source, encode_program};

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

    pub const fn product_identity(self) -> (CompiledProductKind, &'static str) {
        match self {
            Self::HbcModule => (CompiledProductKind::HbcModule, "hbc0"),
            Self::WholeWasm => (CompiledProductKind::WholeWasm, "hnw0/0"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledArtifact {
    target: CompileTarget,
    product: CompiledProduct,
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
    let hbc = compile_hbc(source)?;
    let (kind, abi_version) = target.product_identity();
    let module_digests = vec![sha256_hex(&hbc)];
    let key = ProductCacheKey::with_module_digests(
        kind,
        sha256_hex(source.as_bytes()),
        format!("hara-compiler/{}", env!("CARGO_PKG_VERSION")),
        abi_version,
        b"{}",
        module_digests,
    );
    if let Some(product) = cache.get(&key) {
        return Ok(CompiledArtifact {
            target,
            product: product.clone(),
        });
    }
    let bytes = match target {
        CompileTarget::HbcModule => hbc.clone(),
        CompileTarget::WholeWasm => compile_whole_wasm(&hbc)?,
    };
    let product = CompiledProduct::new(
        kind,
        sha256_hex(source.as_bytes()),
        vec![sha256_hex(&hbc)],
        format!("hara-compiler/{}", env!("CARGO_PKG_VERSION")),
        abi_version,
        b"{}",
        bytes,
    );
    cache.insert(product.clone())?;
    Ok(CompiledArtifact { target, product })
}

pub fn compile_product(source: &str, target: CompileTarget) -> Result<CompiledProduct, String> {
    let hbc = compile_hbc(source)?;
    let (product, abi_version) = target.product_identity();
    let bytes = match target {
        CompileTarget::HbcModule => hbc.clone(),
        CompileTarget::WholeWasm => compile_whole_wasm(&hbc)?,
    };
    Ok(CompiledProduct::new(
        product,
        sha256_hex(source.as_bytes()),
        vec![sha256_hex(&hbc)],
        format!("hara-compiler/{}", env!("CARGO_PKG_VERSION")),
        abi_version,
        b"{}",
        bytes,
    ))
}

fn compile_hbc(source: &str) -> Result<Vec<u8>, String> {
    let program = compile_source(source).map_err(|error| error.to_string())?;
    encode_program(&program)
}

#[cfg(feature = "full-wasm")]
fn compile_whole_wasm(hbc: &[u8]) -> Result<Vec<u8>, String> {
    hara_runtime::whole_wasm::compile_artifact_from_hbc(hbc)
}

#[cfg(not(feature = "full-wasm"))]
fn compile_whole_wasm(_hbc: &[u8]) -> Result<Vec<u8>, String> {
    Err("whole-wasm compilation requires the hara-compiler/full-wasm feature".into())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "full-wasm")]
    use super::sha256_hex;
    use super::{compile, CompileTarget, CompiledProductKind, InMemoryProductCache};

    #[test]
    fn compiler_output_executes_in_vm_only_crate() {
        let artifact = compile("(+ 19 23)", CompileTarget::HbcModule).unwrap();
        assert_eq!(hara_vm::execute(artifact.bytes()).unwrap().display(), "42");
    }

    #[test]
    fn targets_own_their_product_identity() {
        assert_eq!(
            CompileTarget::HbcModule.product_identity(),
            (CompiledProductKind::HbcModule, "hbc0")
        );
        assert_eq!(
            CompileTarget::WholeWasm.product_identity(),
            (CompiledProductKind::WholeWasm, "hnw0/0")
        );
    }

    #[test]
    fn explicit_hbc_target_preserves_the_bytecode_contract() {
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

    #[cfg(feature = "full-wasm")]
    #[test]
    fn whole_wasm_target_retains_the_canonical_hbc_input() {
        let source = "(+ 19 23)";
        let hbc = compile(source, CompileTarget::HbcModule).unwrap();
        let whole = compile(source, CompileTarget::WholeWasm).unwrap();
        let decoded = hara_runtime::whole_wasm::decode_artifact(whole.bytes()).unwrap();
        let retained = hara_runtime::vm::encode_program(&decoded.program).unwrap();

        assert_eq!(retained, hbc.bytes());
        assert_eq!(
            whole.manifest().module_digests,
            vec![sha256_hex(hbc.bytes())]
        );
    }
}
