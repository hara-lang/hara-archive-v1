use super::{
    import_wit, inspect_direct, project_wit, WasmInterface, WasmValueType, WitImportOptions,
    WitProjectionOptions, WitRoute,
};

const START_SENTINEL: &[u8] = b"\0asm\x01\0\0\0\x08\x01\0";

const SCALAR_INTERFACE: &str = r#"
  (wasm/interface
   {:schema "hara.wasm-interface/0-alpha"
    :namespace math.scalar
    :module "modules/math.wasm"
    :exports
    {add {:wasm/export "add_i64"
          :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                      {:name right :hara/type :i64 :wasm/type :i64}]
          :returns {:hara/type :i64 :wasm/type :i64}}}})"#;

const MEMORY_INTERFACE: &str = r#"
  (wasm/interface
   {:schema "hara.wasm-interface/0-alpha"
    :namespace codec.echo
    :module "echo.wasm"
    :memory {:export "memory" :allocate "alloc" :release "free"}
    :exports
    {echo {:wasm/export "echo_bytes"
           :arguments [{:name input
                        :hara/type :bytes
                        :wasm/type :i32
                        :lower [:pointer :length]
                        :ownership :borrowed}]
           :returns {:hara/type :bytes
                     :wasm/type :i64
                     :lift :packed-i64
                     :ownership :caller}}}})"#;

#[test]
fn parses_scalar_interface_without_evaluation() {
    let interface = WasmInterface::parse(SCALAR_INTERFACE, "fixture").unwrap();
    assert_eq!(interface.namespace, "math.scalar");
    assert_eq!(interface.module, "modules/math.wasm");
    assert_eq!(interface.exports[0].name, "add");
    assert_eq!(interface.exports[0].wasm_export, "add_i64");
    assert_eq!(
        interface.exports[0].arguments[0].wasm_type,
        WasmValueType::I64
    );
    assert_eq!(interface.direct_exports()[0].0, "add_i64");
    assert_eq!(interface.digest().len(), 71);
    assert!(interface.digest().starts_with("sha256:"));
    assert_eq!(
        WasmInterface::parse(&interface.canonical_source(), "canonical").unwrap(),
        interface
    );
}

#[test]
fn parses_explicit_memory_semantics_without_executing_them() {
    let interface = WasmInterface::parse(MEMORY_INTERFACE, "fixture").unwrap();
    let memory = interface.memory.as_ref().unwrap();
    assert_eq!(memory.export, "memory");
    assert_eq!(memory.allocate.as_deref(), Some("alloc"));
    assert_eq!(memory.release.as_deref(), Some("free"));
    assert_eq!(
        WasmInterface::parse(&interface.canonical_source(), "canonical").unwrap(),
        interface
    );
}

#[test]
fn static_inspection_records_a_start_function_without_running_it() {
    let inspection = inspect_direct(START_SENTINEL).unwrap();
    assert_eq!(inspection.start, Some(0));
}

#[test]
fn canonicalizes_map_and_set_order() {
    let left = r#"
      {:schema "hara.wasm-interface/0-alpha"
       :namespace math.scalar
       :module "modules/math.wasm"
       :capabilities [:random :clock]
       :exports
       {subtract {:wasm/export "sub"
                  :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                              {:name right :hara/type :i64 :wasm/type :i64}]
                  :returns {:hara/type :i64 :wasm/type :i64}}
        add {:wasm/export "add_i64"
             :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                         {:name right :hara/type :i64 :wasm/type :i64}]
             :returns {:hara/type :i64 :wasm/type :i64}
             :capabilities [:clock :random]}}}"#;
    let right = r#"
      {:exports
       {add {:capabilities [:random :clock]
             :returns {:wasm/type :i64 :hara/type :i64}
             :arguments [{:wasm/type :i64 :hara/type :i64 :name left}
                         {:hara/type :i64 :name right :wasm/type :i64}]
             :wasm/export "add_i64"}
        subtract {:returns {:wasm/type :i64 :hara/type :i64}
                  :wasm/export "sub"
                  :arguments [{:wasm/type :i64 :name left :hara/type :i64}
                              {:name right :wasm/type :i64 :hara/type :i64}]}}
       :module "modules/math.wasm"
       :capabilities [:clock :random]
       :namespace math.scalar
       :schema "hara.wasm-interface/0-alpha"}"#;
    let left = WasmInterface::parse(left, "left").unwrap();
    let right = WasmInterface::parse(right, "right").unwrap();
    assert_eq!(left, right);
    assert_eq!(left.canonical_source(), right.canonical_source());
    assert_eq!(left.digest(), right.digest());
}

#[test]
fn rejects_executable_unknown_duplicate_and_unsafe_sources() {
    for source in [
        "(do (println \"not data\"))".to_owned(),
        SCALAR_INTERFACE.replace(":module \"modules/math.wasm\"", ":module \"../math.wasm\""),
        SCALAR_INTERFACE.replace(
            ":schema \"hara.wasm-interface/0-alpha\"",
            ":schema \"hara.wasm-interface/9\"",
        ),
        SCALAR_INTERFACE.replace(":namespace math.scalar", ":namespace Math.scalar"),
        SCALAR_INTERFACE.replace(":exports", ":unknown true :exports"),
        SCALAR_INTERFACE.replace(":name left", ":name left :name duplicate"),
    ] {
        let error = WasmInterface::parse(&source, "fixture").unwrap_err();
        assert!(error.starts_with("wasm-interface/"));
    }
}

#[test]
fn rejects_ambiguous_and_future_semantics() {
    let mismatch = SCALAR_INTERFACE.replace(
        ":name left :hara/type :i64 :wasm/type :i64",
        ":name left :hara/type :i32 :wasm/type :i64",
    );
    assert!(WasmInterface::parse(&mismatch, "mismatch")
        .unwrap_err()
        .contains("maps :i32 to :i64"));

    let missing_ownership = SCALAR_INTERFACE.replace(
        ":name left :hara/type :i64 :wasm/type :i64",
        ":name left :hara/type :bytes :wasm/type :i32 :lower [:pointer :length]",
    );
    assert!(WasmInterface::parse(&missing_ownership, "bytes")
        .unwrap_err()
        .contains("requires :ownership"));

    let missing_memory = MEMORY_INTERFACE.replace(
        ":memory {:export \"memory\" :allocate \"alloc\" :release \"free\"}",
        "",
    );
    assert!(WasmInterface::parse(&missing_memory, "bytes")
        .unwrap_err()
        .contains("require an explicit :memory contract"));

    let asynchronous = SCALAR_INTERFACE.replace(
        ":returns {:hara/type :i64 :wasm/type :i64}",
        ":returns {:hara/type :i64 :wasm/type :i64} :async true",
    );
    assert!(WasmInterface::parse(&asynchronous, "async")
        .unwrap()
        .exports
        .first()
        .is_some_and(|export| export.asynchronous));

    let handles = SCALAR_INTERFACE.replace(
        ":exports",
        ":handles {stream {:tag stream :release \"stream_drop\"}} :exports",
    );
    let handles = WasmInterface::parse(&handles, "handles").unwrap();
    assert_eq!(handles.handles["stream"].tag, "stream");
    assert_eq!(
        handles.handles["stream"].release.as_deref(),
        Some("stream_drop")
    );
}

const WIT_SCALAR: &str = r#"
package demo:calculator;

interface calculator {
  add: func(left: s64, right: s64) -> s64;
}

world calculator-world {
  export calculator;
}
"#;

const WIT_RICH: &str = r#"
package demo:rich;

interface rich {
  record point {
    x: s32,
    y: s32,
  }
  variant choice {
    point(point),
    none,
  }
  resource stream;
  transform: func(value: option<point>, data: list<string>) -> result<choice, string>;
}

world rich-world {
  import host;
  export rich;
}
"#;

#[test]
fn imports_scalar_wit_deterministically_and_keeps_the_direct_route() {
    let options = WitImportOptions {
        module: Some("fixtures/calculator.wasm".into()),
        ..WitImportOptions::default()
    };
    let left = import_wit(WIT_SCALAR, "calculator.wit", &options).unwrap();
    let right = import_wit(WIT_SCALAR, "calculator.wit", &options).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.route, WitRoute::DirectImport);
    assert!(left.diagnostics.is_empty());
    assert!(left.interface_source.contains(":wasm/type :i64"));
    let interface = WasmInterface::parse(&left.interface_source, "wit skeleton").unwrap();
    let projection = project_wit(
        &interface,
        &WitProjectionOptions {
            strict: true,
            ..WitProjectionOptions::default()
        },
    )
    .unwrap();
    assert!(projection
        .source
        .contains("add: func(left: s64, right: s64) -> s64;"));
    let round_trip = import_wit(&projection.source, "projection.wit", &options).unwrap();
    assert_eq!(
        interface,
        WasmInterface::parse(&round_trip.interface_source, "round trip").unwrap()
    );
}

#[test]
fn rich_wit_reports_lossy_features_and_strict_mode_rejects_them() {
    let permissive = import_wit(WIT_RICH, "rich.wit", &WitImportOptions::default()).unwrap();
    assert_eq!(permissive.route, WitRoute::HtaRequire);
    assert!(permissive
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "option"));
    assert!(permissive
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "resource"));
    assert!(permissive.normalized_ir.contains(":provenance"));

    let strict = import_wit(
        WIT_RICH,
        "rich.wit",
        &WitImportOptions {
            strict: true,
            ..WitImportOptions::default()
        },
    )
    .unwrap_err();
    assert!(strict.starts_with("wasm-wit/strict"));
    assert!(strict.contains("option"));
    assert!(strict.contains("world-import"));
}
