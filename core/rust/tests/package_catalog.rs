use hara_wasm::package_catalog::{
    definitions_from_packages_edn, find_package_definition, package_dependency_order,
    package_namespaces, validate_package_definitions,
};
use std::path::Path;

#[test]
fn hara_package_profile_matches_the_semantic_package_coordinates() {
    let definitions = definitions_from_packages_edn(include_str!("../../config/packages.edn"))
        .expect("the Hara package profile should be valid EDN");
    let postgres = find_package_definition(&definitions, "lang.model.v1.postgres")
        .expect("the Postgres model package should be defined");
    let available = [
        "lang.model.v1.spec-postgres",
        "lang.model.v1.spec-postgres.deftype.common",
        "postgres.core",
        "postgres.core.graph",
        "postgres.gen",
        "postgres.gen.rpc",
        "postgres.typed",
        "db.postgres",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(
        package_namespaces(postgres, &available),
        vec![
            "lang.model.v1.spec-postgres",
            "lang.model.v1.spec-postgres.deftype.common",
            "postgres.core",
            "postgres.core.graph",
            "postgres.gen",
            "postgres.gen.rpc",
            "postgres.typed",
        ]
    );
    assert_eq!(
        package_dependency_order(&definitions, &["lang.model.v1.postgres".to_owned()])
            .expect("package dependencies should be acyclic"),
        vec![
            "lang.base".to_owned(),
            "lang.common".to_owned(),
            "lang.core".to_owned(),
            "lang.model.v1.spec-xtalk".to_owned(),
            "lang.typed".to_owned(),
            "lang.model.v1.postgres".to_owned(),
        ]
    );
}

#[test]
fn hara_package_profile_owns_non_overlapping_real_source_namespaces() {
    let definitions = definitions_from_packages_edn(include_str!("../../config/packages.edn"))
        .expect("the Hara package profile should be valid EDN");
    let project = hara_wasm::project::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../project.edn")
            .as_path(),
    )
    .expect("the core project should be readable");
    let available = hara_wasm::project::source_resources(&project)
        .expect("the core source tree should be discoverable")
        .into_iter()
        .map(|(namespace, _)| namespace)
        .collect::<Vec<_>>();
    validate_package_definitions(&definitions, &available)
        .expect("semantic package ownership must not overlap");
    for name in [
        "code.test",
        "code.manage",
        "lang.base",
        "lang.seedgen",
        "lang.model.v1.postgres",
    ] {
        let package = find_package_definition(&definitions, name).unwrap();
        assert!(
            !package_namespaces(package, &available).is_empty(),
            "{name} must select at least one source namespace"
        );
    }
}
