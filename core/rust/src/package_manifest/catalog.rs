use super::{PackageCatalogAdmission, PackageManifestError};
use crate::Runtime;

pub(super) fn admit(
    format: &str,
    source: &str,
) -> Result<PackageCatalogAdmission, PackageManifestError> {
    if format != "std.typed.catalog/2" {
        return Err(PackageManifestError::new(
            "package/catalog-unsupported",
            format!("unsupported :schema/catalog :format {format}"),
        ));
    }
    let encoded = serde_json::to_string(source).map_err(|error| {
        PackageManifestError::new(
            "package/catalog-invalid",
            format!("cannot encode catalog for canonical Hara verification: {error}"),
        )
    })?;
    let expression = format!(
        "(do (require 'std.typed.catalog.document) (std.typed.catalog.document/verify-json {encoded}))"
    );
    let report = std::thread::Builder::new()
        .name("hara-package-catalog-admission".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut runtime = Runtime::new();
            for (namespace, resource) in [
                ("std.typed", include_str!("../../../lib/src/std/typed.hal")),
                (
                    "std.typed.catalog",
                    include_str!("../../../lib/src/std/typed/catalog.hal"),
                ),
                (
                    "std.typed.catalog.document",
                    include_str!("../../../lib/src/std/typed/catalog/document.hal"),
                ),
                (
                    "std.typed.explain",
                    include_str!("../../../lib/src/std/typed/explain.hal"),
                ),
                (
                    "std.typed.infer",
                    include_str!("../../../lib/src/std/typed/infer.hal"),
                ),
                (
                    "std.typed.registry",
                    include_str!("../../../lib/src/std/typed/registry.hal"),
                ),
                (
                    "std.typed.schema",
                    include_str!("../../../lib/src/std/typed/schema.hal"),
                ),
            ] {
                runtime.register_resource(namespace, resource);
            }
            runtime.eval_native(&expression)
        })
        .map_err(|error| {
            PackageManifestError::new(
                "package/catalog-invalid",
                format!("cannot start canonical std.typed catalog admission: {error}"),
            )
        })?
        .join()
        .map_err(|_| {
            PackageManifestError::new(
                "package/catalog-invalid",
                "canonical std.typed catalog admission panicked",
            )
        })?
        .map_err(|error| {
            PackageManifestError::new(
                "package/catalog-invalid",
                format!("canonical std.typed catalog admission failed: {error}"),
            )
        })?;
    Ok(PackageCatalogAdmission {
        format: format.to_owned(),
        report,
    })
}
