use hara_wasm::Runtime;

#[test]
fn global_imports_cover_ordinary_vars_and_compact_protocol_methods() {
    let mut runtime = Runtime::core();
    runtime
        .eval_native(
            "(ns demo.global (:config {:set-global [demo.global/value]})) \
             (def value 42) \
             (ns demo.consumer) value",
        )
        .unwrap();
    assert_eq!(
        runtime
            .eval_native(
            "(ns demo.protocol (:config {:set-global [IColl/start-string IMetadata/metatype]})) \
             (start-string [])",
            )
            .unwrap(),
        "\"[\""
    );
    assert_eq!(
        runtime.eval_native("(metatype {:value 1})").unwrap(),
        ":map"
    );
}

#[test]
fn foundation_child_paths_load_the_root_first() {
    let mut runtime = Runtime::core();
    runtime.register_resource(
        "std/foundation.hal",
        "(ns std.foundation) (defn foundation-marker [] 41)",
    );
    runtime.register_resource(
        "std/foundation/child.hal",
        "(ns std.foundation.child) (defn child-marker [] (foundation-marker))",
    );
    assert_eq!(
        runtime
            .require_resource("std/foundation/child.hal")
            .unwrap(),
        "#'std.foundation.child/child-marker"
    );
    assert_eq!(
        runtime
            .eval_native("(std.foundation.child/child-marker)")
            .unwrap(),
        "41"
    );
}
