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

#[test]
fn imports_interface_exports_and_direct_world_functions() {
    let interface_export = r#"
package demo:calculator@1.2.3;
interface calculator {
  add: func(left: s32, right: s32) -> s32;
}
world calculator-world {
  export calculator: interface;
}
"#;
    let imported = import_wit(
        interface_export,
        "interface-export.wit",
        &WitImportOptions {
            strict: true,
            ..WitImportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(imported.route, WitRoute::DirectImport);
    assert_eq!(
        WasmInterface::parse(&imported.interface_source, "interface-export.hal")
            .unwrap()
            .exports[0]
            .name,
        "add"
    );

    let world_function = r#"
package demo:calculator;
world calculator-world {
  export add: func(left: s32, right: s32) -> s32;
}
"#;
    let imported = import_wit(
        world_function,
        "world-function.wit",
        &WitImportOptions {
            strict: true,
            ..WitImportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(imported.route, WitRoute::DirectImport);
    assert_eq!(
        WasmInterface::parse(&imported.interface_source, "world-function.hal")
            .unwrap()
            .exports[0]
            .name,
        "add"
    );
}
