use hara_wasm::Runtime;

#[test]
fn rust_runtime_rejects_host_flavors() {
    let mut runtime = Runtime::core();
    for flavor in ["jvm", "dotnet"] {
        let error = runtime
            .eval_native(&format!(
                "(ns host.{flavor} (:flavor :{flavor} [java.lang String]))"
            ))
            .unwrap_err();
        assert_eq!(
            error,
            format!(
                "native/unsupported-flavor: :{flavor} (host flavors are only available on JVM/.NET runtimes)"
            )
        );
    }
}

#[test]
fn wasm_flavor_retains_the_import_guidance() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime
            .eval_native("(ns explicit.wasm (:flavor :wasm))")
            .unwrap_err(),
        "native/unsupported-flavor: :wasm (Wasm modules use :import)"
    );
}
