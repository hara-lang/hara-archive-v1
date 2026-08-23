use hara_wasm::wasm_binding::{
    import_wit, project_wit, WasmInterface, WitImportOptions, WitProjectionOptions, WitRoute,
};

const SCALAR: &str = r#"
package demo:calculator;
interface calculator {
  add: func(left: s64, right: s64) -> s64;
}
world calculator-world {
  export calculator;
}
"#;

#[test]
fn scalar_import_and_projection_are_deterministic() {
    let options = WitImportOptions {
        module: Some("calculator.wasm".into()),
        ..WitImportOptions::default()
    };
    let first = import_wit(SCALAR, "scalar.wit", &options).unwrap();
    let second = import_wit(SCALAR, "scalar.wit", &options).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.route, WitRoute::DirectImport);
    let interface = WasmInterface::parse(&first.interface_source, "scalar.hal").unwrap();
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
}

#[test]
fn strict_import_lists_lossy_features() {
    let source = r#"
package demo:rich;
interface rich {
  resource stream;
  invoke: func(value: option<string>) -> result<string, string>;
}
world rich-world {
  import host;
  export rich;
}
"#;
    let error = import_wit(
        source,
        "rich.wit",
        &WitImportOptions {
            strict: true,
            ..WitImportOptions::default()
        },
    )
    .unwrap_err();
    assert!(error.starts_with("wasm-wit/strict"));
    assert!(error.contains("option"));
    assert!(error.contains("world-import"));
}
