use super::*;

const JVM_SHA: &str = "sha256:c002b77f9f7b3b1b74771be2e5c75da33c6911c6f2d10689f69242cb184d9b3b";
const WASM_SHA: &str = "sha256:336154bf67f765f8f75d16a0accee61b5ee5f6a75b2a2905703df913bd550f3e";

fn requirements(
    target: &str,
    abi: &str,
    capabilities: &[&str],
    host_calls: &[&str],
) -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        supported_targets: [target.to_owned()].into_iter().collect(),
        supported_abis: [abi.to_owned()].into_iter().collect(),
        available_capabilities: capabilities
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        allowed_host_calls: host_calls.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn package_manifest() -> String {
    format!(
        r#"{{:harp/format "0.0.0-alpha"
 :package {{:identity "hara:example/provider"
           :version "1.0.0"
           :provenance {{:repository "https://github.com/example/provider"
                         :commit "0123456789abcdef0123456789abcdef01234567"}}}}
 :files {{"artifacts/provider.jar" {{:sha256 "{JVM_SHA}" :size 4}}
          "artifacts/provider.wasm" {{:sha256 "{WASM_SHA}" :size 4}}}}
 :flavors {{:jvm {{:variant/artifact
                    {{:artifact/type :jar
                     :artifact/path "artifacts/provider.jar"
                     :artifact/sha256 "{JVM_SHA}"
                     :artifact/target "java-21"
                     :artifact/abi "hara.provider.jvm.v1"
                     :artifact/entry-point "example.provider.HaraProvider"}}
                   :variant/required-capabilities #{{:db/connect}}
                   :variant/host-calls #{{}}
                   :variant/exports #{{:provider/open :provider/close}}
                   :variant/lifecycle {{:lifecycle/load :idempotent
                                        :lifecycle/close :idempotent
                                        :lifecycle/session-isolation true}}}}}}
 :wasm-imports {{:provider {{:variant/artifact
                              {{:artifact/type :wasm
                               :artifact/path "artifacts/provider.wasm"
                               :artifact/sha256 "{WASM_SHA}"
                               :artifact/target "wasm32-wasi-preview1"
                               :artifact/abi "core.v1"
                               :artifact/entry-point "provider_init"}}
                             :variant/required-capabilities #{{}}
                             :variant/host-calls #{{}}
                             :variant/exports #{{:provider/open :provider/close}}
                             :variant/lifecycle {{:lifecycle/load :idempotent
                                                  :lifecycle/close :idempotent
                                                  :lifecycle/session-isolation true}}}}}}
 :descriptor {{:operations [:provider/open :provider/close]}}}}"#
    )
}

#[test]
fn selects_host_flavor_and_shared_wasm_import_independently() {
    let manifest = PackageManifest::parse(&package_manifest()).unwrap();
    assert!(manifest
        .unsupported_host_flavors_warning()
        .unwrap()
        .contains("package/host-flavors-ignored"));
    let jvm_selection = manifest
        .select_flavor(
            "jvm",
            &requirements("java-21", "hara.provider.jvm.v1", &["db/connect"], &[]),
        )
        .unwrap();
    let PackageSelection::Variant(jvm) = &jvm_selection else {
        panic!("expected JVM flavor");
    };
    assert_eq!(jvm.artifact.artifact_type, PackageArtifactType::Jar);
    manifest
        .verify_artifact_bytes(&jvm_selection, b"jvm!")
        .unwrap();

    let wasm_selection = manifest
        .select_wasm_import(
            "provider",
            &requirements("wasm32-wasi-preview1", "core.v1", &[], &[]),
        )
        .unwrap();
    let PackageSelection::Variant(wasm) = &wasm_selection else {
        panic!("expected Wasm import");
    };
    assert_eq!(wasm.artifact.artifact_type, PackageArtifactType::Wasm);
    manifest
        .verify_artifact_bytes(&wasm_selection, b"wasm")
        .unwrap();
}

#[test]
fn portable_packages_remain_portable_and_missing_flavors_are_not_fallbacks() {
    let portable = PackageManifest::parse(
        r#"{:harp/format "0.0.0-alpha"
             :package {:identity "hara:example/portable" :version "1.0.0"}
             :files {"src/example/core.hal"
                     {:sha256 "sha256:b8ba2ec7e90713c1043778164af3250820943c2165c9f19fa29987e016aae5dd"
                      :size 4}}}"#,
    ).unwrap();
    assert_eq!(
        portable
            .select_flavor("jvm", &PackageRuntimeRequirements::default())
            .unwrap(),
        PackageSelection::Portable
    );

    let manifest = PackageManifest::parse(&package_manifest()).unwrap();
    let error = manifest
        .select_flavor("dotnet", &PackageRuntimeRequirements::default())
        .unwrap_err();
    assert_eq!(error.code, "package/missing-flavor");

    let mut direct_only = manifest.clone();
    direct_only.flavors.clear();
    let error = direct_only
        .select_flavor(
            "jvm",
            &requirements("java-21", "hara.provider.jvm.v1", &[], &[]),
        )
        .unwrap_err();
    assert_eq!(error.code, "package/missing-flavor");
}

#[test]
fn preflight_rejects_target_abi_and_capability_mismatches() {
    let manifest = PackageManifest::parse(&package_manifest()).unwrap();
    assert_eq!(
        manifest
            .select_flavor(
                "jvm",
                &requirements("java-17", "hara.provider.jvm.v1", &["db/connect"], &[])
            )
            .unwrap_err()
            .code,
        "package/target-mismatch"
    );
    assert_eq!(
        manifest
            .select_flavor(
                "jvm",
                &requirements("java-21", "hara.provider.jvm.v2", &["db/connect"], &[])
            )
            .unwrap_err()
            .code,
        "package/abi-mismatch"
    );
    assert_eq!(
        manifest
            .select_flavor(
                "jvm",
                &requirements("java-21", "hara.provider.jvm.v1", &[], &[])
            )
            .unwrap_err()
            .code,
        "package/capability-denied"
    );
}

#[test]
fn rejects_wasm_flavors_and_requires_provenance() {
    let source = package_manifest().replace(":flavors {:jvm", ":flavors {:wasm");
    let error = PackageManifest::parse(&source).unwrap_err();
    assert_eq!(error.code, "package/invalid-manifest");
    assert!(error.detail.contains(":wasm"));

    let source = package_manifest().replace(
        ":provenance {:repository \"https://github.com/example/provider\"\n                         :commit \"0123456789abcdef0123456789abcdef01234567\"}",
        "",
    );
    let error = PackageManifest::parse(&source).unwrap_err();
    assert_eq!(error.code, "package/invalid-manifest");
    assert!(error.detail.contains("provenance"));
}

#[test]
fn route_selection_rejects_cross_route_artifacts() {
    let manifest = PackageManifest::parse(&package_manifest()).unwrap();
    let error = manifest
        .select_hta_require(
            "provider",
            &requirements("wasm32-wasi-preview1", "hta.v1", &[], &[]),
        )
        .unwrap_err();
    assert_eq!(error.code, "package/artifact-type-mismatch");

    let mut jar_only = manifest.clone();
    jar_only.wasm_imports.clear();
    let error = jar_only
        .select_hta_require(
            "provider",
            &requirements("wasm32-wasi-preview1", "hta.v1", &[], &[]),
        )
        .unwrap_err();
    assert_eq!(error.code, "package/missing-require-artifact");

    let hta_source = package_manifest()
        .replace(":artifact/type :wasm", ":artifact/type :hta")
        .replace(":artifact/abi \"core.v1\"", ":artifact/abi \"hta.v1\"");
    let hta = PackageManifest::parse(&hta_source).unwrap();
    let selection = hta
        .select_hta_require(
            "provider",
            &requirements("wasm32-wasi-preview1", "hta.v1", &[], &[]),
        )
        .unwrap();
    let PackageSelection::Variant(variant) = selection else {
        panic!("expected HTA require variant");
    };
    assert_eq!(variant.artifact.artifact_type, PackageArtifactType::Hta);
}

#[test]
fn canonicalization_is_idempotent_and_file_verification_is_exact() {
    let manifest = PackageManifest::parse(&package_manifest()).unwrap();
    let root =
        std::env::temp_dir().join(format!("hara-package-manifest-test-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("artifacts")).unwrap();
    fs::write(root.join("artifacts/provider.jar"), b"jvm!").unwrap();
    fs::write(root.join("artifacts/provider.wasm"), b"wasm").unwrap();
    manifest.verify_files_at(&root).unwrap();
    fs::write(root.join("artifacts/provider.jar"), b"tampered").unwrap();
    assert!(matches!(
        manifest.verify_files_at(&root).unwrap_err().code,
        "package/size-mismatch" | "package/digest-mismatch"
    ));
    fs::remove_dir_all(&root).unwrap();
    let canonical = manifest.canonical_edn().to_owned();
    assert_eq!(
        PackageManifest::parse(&canonical).unwrap().canonical_edn(),
        canonical
    );
}

#[test]
fn schema_catalog_descriptor_is_bound_to_declared_bytes_and_canonical_admission() {
    let manifest = PackageManifest::parse(
        r#"{:harp/format "0.0.0-alpha"
             :package {:identity "hara:example/catalog" :version "1.0.0"}
             :files {"catalog/std-typed-catalog.json"
                     {:sha256 "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                      :size 2}}
             :schema/catalog {:format "std.typed.catalog/2"
                              :path "catalog/std-typed-catalog.json"
                              :sha256 "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"}}"#,
    )
    .unwrap();
    let error = manifest.admit_catalog_bytes(b"{}").unwrap_err();
    assert_eq!(error.code, "package/catalog-invalid");
    assert!(error
        .detail
        .contains("canonical std.typed catalog admission"));
}

#[test]
fn schema_catalog_descriptor_rejects_unsupported_format_before_runtime() {
    let source = r#"{:harp/format "0.0.0-alpha"
                    :package {:identity "hara:example/catalog" :version "1.0.0"}
                    :files {"catalog.json"
                            {:sha256 "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                             :size 2}}
                    :schema/catalog {:format "std.typed.catalog/1"
                                     :path "catalog.json"
                                     :sha256 "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"}}"#;
    let manifest = PackageManifest::parse(source).unwrap();
    let error = manifest.admit_catalog_bytes(b"{}").unwrap_err();
    assert_eq!(error.code, "package/catalog-unsupported");
}
