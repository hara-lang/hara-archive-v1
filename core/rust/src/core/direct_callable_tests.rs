#[test]
fn direct_callable_catalog_closes_the_runtime_inventory() {
    validate_direct_callable_catalog().unwrap();
    assert_eq!(
        DIRECT_CALLABLE_CATALOG.len(),
        RUNTIME_CALLABLE_INVENTORY.len()
    );
    assert!(
        DIRECT_CALLABLE_CATALOG.len() >= 150,
        "the complete ordinary callable catalog unexpectedly shrank"
    );
    for specification in DIRECT_CALLABLE_CATALOG {
        assert!(
            direct_callable_value(specification.symbol).is_some(),
            "missing direct callable value for {}",
            specification.symbol
        );
    }
}

#[cfg(not(feature = "raw-wasm"))]
#[test]
fn complete_ordinary_callable_catalog_never_reenters_the_evaluator() {
    let runtime = crate::Runtime::empty();
    with_test_runner(&runtime.test_runner, || {
        with_capability_providers(
            runtime.providers.file(),
            runtime.providers.socket(),
            runtime.providers.process(),
            runtime.providers.kernel(),
            || {
                with_package_catalog(&runtime.package_catalog, || {
                    with_promise_provider(runtime.providers.promise(), || {
                        with_macros(runtime.macros.clone(), || {
                            with_namespace_registry(&runtime.namespace_registry, || {
                                with_protocols(&runtime.protocols, || {
                                    for specification in DIRECT_CALLABLE_CATALOG
                                        .iter()
                                        .filter(|specification| specification.origin.ordinary())
                                    {
                                        let callable = direct_callable_value(specification.symbol)
                                            .unwrap_or_else(|| {
                                                panic!(
                                                    "missing direct callable value for {}",
                                                    specification.symbol
                                                )
                                            });
                                        let arguments =
                                            direct_callable_probe_arguments(specification);
                                        let (_, evaluator_invocations) =
                                            with_evaluator_invocation_count(|| {
                                                call_value(callable, arguments)
                                            });
                                        assert_eq!(
                                            evaluator_invocations, 0,
                                            "{} must dispatch directly at the value boundary",
                                            specification.symbol
                                        );
                                    }
                                })
                            })
                        })
                    })
                })
            },
        )
    });
}

#[test]
fn representative_direct_callables_preserve_value_behavior() {
    let count = direct_callable_value("count").unwrap();
    let result = call_value(
        count,
        vec![Value::Vector(
            vec![Value::Number(1), Value::Number(2)].into(),
        )],
    )
    .unwrap();
    assert_eq!(result, Value::Number(2));

    let increment = direct_callable_value("inc").unwrap();
    assert_eq!(
        call_value(increment, vec![Value::Number(41)]).unwrap(),
        Value::Number(42)
    );

    let ifn = foundation_protocol_values()
        .into_iter()
        .find_map(|(name, value)| (name == "IFn").then_some(value))
        .expect("the Foundation IFn protocol must be installed");
    let identity = direct_callable_value("identity").unwrap();
    let satisfies = direct_callable_value("satisfies?").unwrap();
    assert_eq!(
        call_value(satisfies, vec![ifn, identity]).unwrap(),
        Value::Bool(true)
    );

    let boolean_predicate = direct_callable_value("boolean?").unwrap();
    assert_eq!(
        call_value(boolean_predicate.clone(), vec![Value::Bool(true)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        call_value(boolean_predicate, vec![Value::Number(1)]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn namespace_string_file_and_map_entry_operations_are_direct_callables() {
    let registry = crate::kernel::NamespaceRegistry::new("user");
    let imported = registry.find_or_create("direct.imports");
    imported.import("Vector", "vendor.numeric.Vector");
    let revision = registry.module_revision("direct.imports");

    with_namespace_registry(&registry, || {
        assert_eq!(
            call_value(
                direct_callable_value("module-revision").unwrap(),
                vec![Value::Symbol(Symbol::parse("direct.imports"))],
            )
            .unwrap(),
            Value::Number(revision as i64)
        );

        assert_eq!(
            call_value(
                direct_callable_value("str/trim").unwrap(),
                vec![Value::String("  hara  ".into())],
            )
            .unwrap(),
            Value::String("hara".into())
        );

        assert_eq!(
            call_value(
                direct_callable_value("file/parent").unwrap(),
                vec![Value::String("/a/b".into())],
            )
            .unwrap(),
            Value::String("/a".into())
        );

        assert_eq!(
            call_value(
                direct_callable_value("map-entry?").unwrap(),
                vec![Value::Tuple(Box::new(
                    PTuple::from_values(
                        vec![Value::Keyword("a".into()), Value::Number(1)],
                    )
                    .unwrap(),
                ))],
            )
            .unwrap(),
            Value::Bool(true)
        );

        assert_eq!(
            call_value(
                direct_callable_value("ns:imports").unwrap(),
                vec![Value::Namespace(Rc::new(imported))],
            )
            .unwrap(),
            Value::Map(PMap::from_iter([(
                Value::Symbol(Symbol::parse("Vector")),
                Value::String("vendor.numeric.Vector".into()),
            )]))
        );
    });
}

#[cfg(not(feature = "raw-wasm"))]
#[test]
fn specs_owned_direct_callable_bootstrap_fixture_runs_before_foundation_source_loading() {
    let registry = std::env::var_os("HARA_SPECS_REGISTRY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("..")
                .join("hara-specs-registry")
        });
    let path = registry
        .join("01-lang/004-foundation/draft/conformance/fixtures/direct_callable_bootstrap.hal");
    let source = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "authoritative direct-callable bootstrap fixture is required at {}: {error}",
            path.display()
        )
    });

    let mut runtime = crate::Runtime::core();
    let report = runtime
        .eval_text(&(source + "\n(direct-callable-bootstrap-report)"))
        .unwrap();
    assert_eq!(
        report,
        "[true false :std.native.Promise :std.native.Promise]"
    );
}

#[test]
fn every_native_inventory_entry_builds_a_direct_value() {
    for (native_type, methods) in NATIVE_TYPES {
        for method in *methods {
            let (value, evaluator_invocations) =
                with_evaluator_invocation_count(|| native_type_function_value(native_type, method));
            value.unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(
                evaluator_invocations, 0,
                "std.native.{native_type}/{method} construction must not re-enter eval"
            );
        }
    }
}
