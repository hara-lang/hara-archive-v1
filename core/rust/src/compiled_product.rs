//! Immutable identities for compiled Hara products.
//!
//! The product layer deliberately contains no evaluator or filesystem policy.
//! It gives compiler targets, runtimes, and browser hosts one deterministic
//! description of the bytes they exchange.

use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const COMPILED_PRODUCT_MANIFEST_SCHEMA: &str = "hara.compiled-product.manifest/0-alpha";

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompiledProductKind {
    HbcModule,
    HbcPackage,
    WholeWasm,
}

impl CompiledProductKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HbcModule => "hbc-module",
            Self::HbcPackage => "hbc-package",
            Self::WholeWasm => "whole-wasm",
        }
    }

    pub const fn format(self) -> &'static str {
        match self {
            Self::HbcModule => "HBC0",
            Self::HbcPackage => "HBX0",
            Self::WholeWasm => "HNW0",
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ProductCacheKey {
    pub kind: CompiledProductKind,
    pub source_digest: String,
    pub module_digests: Vec<String>,
    pub compiler_id: String,
    pub abi_version: String,
    pub options_digest: String,
}

impl ProductCacheKey {
    pub fn new(
        kind: CompiledProductKind,
        source_digest: impl Into<String>,
        compiler_id: impl Into<String>,
        abi_version: impl Into<String>,
        options: impl AsRef<[u8]>,
    ) -> Self {
        Self::with_module_digests(
            kind,
            source_digest,
            compiler_id,
            abi_version,
            options,
            Vec::new(),
        )
    }

    pub fn with_module_digests(
        kind: CompiledProductKind,
        source_digest: impl Into<String>,
        compiler_id: impl Into<String>,
        abi_version: impl Into<String>,
        options: impl AsRef<[u8]>,
        module_digests: Vec<String>,
    ) -> Self {
        Self {
            kind,
            source_digest: source_digest.into(),
            module_digests,
            compiler_id: compiler_id.into(),
            abi_version: abi_version.into(),
            options_digest: sha256_hex(options.as_ref()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledProductManifest {
    pub schema: String,
    pub product: CompiledProductKind,
    pub format: String,
    pub abi_version: String,
    pub compiler_id: String,
    pub source_digest: String,
    pub module_digests: Vec<String>,
    pub options_digest: String,
    pub artifact_digest: String,
    pub artifact_bytes: usize,
}

impl CompiledProductManifest {
    pub fn to_json(&self) -> serde_json::Value {
        let mut manifest = serde_json::json!({
            "schema": self.schema,
            "product": self.product.as_str(),
            "format": self.format,
            "abi-version": self.abi_version,
            "compiler-id": self.compiler_id,
            "source-digest": self.source_digest,
            "module-digests": self.module_digests,
            "options-digest": self.options_digest,
            "artifact-digest": self.artifact_digest,
            "artifact-bytes": self.artifact_bytes,
        });
        if self.product == CompiledProductKind::WholeWasm {
            manifest["entrypoint"] = serde_json::json!("hara_entry");
            manifest["error-global"] = serde_json::json!("hara_error");
            manifest["heap-global"] = serde_json::json!("hara_heap");
            manifest["import-module"] = serde_json::json!("hara");
        }
        manifest
    }

    pub fn cache_key(&self) -> ProductCacheKey {
        ProductCacheKey {
            kind: self.product,
            source_digest: self.source_digest.clone(),
            module_digests: self.module_digests.clone(),
            compiler_id: self.compiler_id.clone(),
            abi_version: self.abi_version.clone(),
            options_digest: self.options_digest.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledProduct {
    pub manifest: CompiledProductManifest,
    pub bytes: Vec<u8>,
}

impl CompiledProduct {
    pub fn new(
        product: CompiledProductKind,
        source_digest: impl Into<String>,
        module_digests: Vec<String>,
        compiler_id: impl Into<String>,
        abi_version: impl Into<String>,
        options: impl AsRef<[u8]>,
        bytes: Vec<u8>,
    ) -> Self {
        let compiler_id = compiler_id.into();
        let abi_version = abi_version.into();
        let options_digest = sha256_hex(options.as_ref());
        let manifest = CompiledProductManifest {
            schema: COMPILED_PRODUCT_MANIFEST_SCHEMA.into(),
            product,
            format: product.format().into(),
            abi_version,
            compiler_id,
            source_digest: source_digest.into(),
            module_digests,
            options_digest,
            artifact_digest: sha256_hex(&bytes),
            artifact_bytes: bytes.len(),
        };
        Self { manifest, bytes }
    }

    pub fn cache_key(&self) -> ProductCacheKey {
        self.manifest.cache_key()
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.manifest.artifact_bytes != self.bytes.len() {
            return Err("compiled product manifest byte length mismatch".into());
        }
        if self.manifest.artifact_digest != sha256_hex(&self.bytes) {
            return Err("compiled product manifest digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryProductCache {
    products: HashMap<ProductCacheKey, CompiledProduct>,
}

impl InMemoryProductCache {
    pub fn get(&self, key: &ProductCacheKey) -> Option<&CompiledProduct> {
        self.products.get(key)
    }

    pub fn insert(&mut self, product: CompiledProduct) -> Result<ProductCacheKey, String> {
        product.verify()?;
        let key = product.cache_key();
        self.products.insert(key.clone(), product);
        Ok(key)
    }

    pub fn remove(&mut self, key: &ProductCacheKey) -> Option<CompiledProduct> {
        self.products.remove(key)
    }

    pub fn len(&self) -> usize {
        self.products.len()
    }

    pub fn is_empty(&self) -> bool {
        self.products.is_empty()
    }

    pub fn clear(&mut self) {
        self.products.clear();
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::{CompiledProduct, CompiledProductKind, InMemoryProductCache};

    fn product(bytes: &[u8]) -> CompiledProduct {
        CompiledProduct::new(
            CompiledProductKind::HbcModule,
            "source-digest",
            vec!["module-digest".into()],
            "hara-test",
            "1",
            "{}",
            bytes.to_vec(),
        )
    }

    #[test]
    fn manifest_is_self_verifying_and_json_stable() {
        let product = product(b"HBC0");
        product.verify().unwrap();
        assert_eq!(product.manifest.format, "HBC0");
        assert_eq!(product.manifest.artifact_bytes, 4);
        assert_eq!(product.manifest.to_json()["product"], "hbc-module");
    }

    #[test]
    fn cache_reuses_exact_products_and_separates_target_keys() {
        let mut cache = InMemoryProductCache::default();
        let first = product(b"first");
        let key = cache.insert(first.clone()).unwrap();
        assert_eq!(cache.get(&key), Some(&first));
        assert_eq!(cache.len(), 1);

        let other = CompiledProduct::new(
            CompiledProductKind::WholeWasm,
            "source-digest",
            vec!["module-digest".into()],
            "hara-test",
            "1",
            "{}",
            b"first".to_vec(),
        );
        let other_key = cache.insert(other).unwrap();
        assert_ne!(key, other_key);
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_keys_include_module_dependencies() {
        let first = CompiledProduct::new(
            CompiledProductKind::HbcModule,
            "source-digest",
            vec!["module-a".into()],
            "hara-test",
            "1",
            "{}",
            b"first".to_vec(),
        );
        let second = CompiledProduct::new(
            CompiledProductKind::HbcModule,
            "source-digest",
            vec!["module-b".into()],
            "hara-test",
            "1",
            "{}",
            b"first".to_vec(),
        );

        assert_ne!(first.cache_key(), second.cache_key());
    }

    #[test]
    fn cache_rejects_tampered_products() {
        let mut product = product(b"valid");
        product.bytes.push(0);
        let mut cache = InMemoryProductCache::default();
        let error = cache.insert(product).unwrap_err();
        assert_eq!(error, "compiled product manifest byte length mismatch");
    }
}
