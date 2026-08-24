use hara_wasm::{core, Runtime};

fn keyword_names(values: Vec<core::Value>) -> Vec<String> {
    let mut names = values
        .into_iter()
        .map(|value| match value {
            core::Value::Keyword(keyword) => keyword.as_str().to_owned(),
            value => panic!("expected keyword set member, got {value:?}"),
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn native_set_inspection_accepts_literal_and_constructed_families() {
    let mut runtime = Runtime::new();
    for source in [
        "#{:alpha :beta}",
        "(hash-set :alpha :beta)",
        "(ordered-set :beta :alpha)",
        "(sorted-set :beta :alpha)",
    ] {
        let value = runtime.eval_native_value(source).unwrap();
        assert_eq!(
            keyword_names(core::set_values(&value).expect("expected persistent set")),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
    }
}

#[test]
fn native_set_inspection_does_not_reclassify_sequential_values() {
    let mut runtime = Runtime::new();
    let value = runtime.eval_native_value("[:alpha :beta]").unwrap();
    assert!(core::set_values(&value).is_none());
}

#[test]
fn canonical_peek_first_protocol_is_available_without_legacy_aliases() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native("(std.protocol.ipeekfirst.IPeekFirst/peek-first [40 42])")
            .unwrap(),
        "40"
    );
    assert_eq!(
        runtime.eval_native("(std.protocol.ipeekfirst/peek-first [40 42])").unwrap(),
        "40"
    );
    assert!(runtime.eval_native("std.foundation/IPeekFirst").is_err());
    assert!(runtime.eval_native("peek-first").is_err());

    #[cfg(feature = "bytecode-vm")]
    {
        assert_eq!(
            runtime
                .eval_bytecode_native("(std.protocol.ipeekfirst.IPeekFirst/peek-first [40 42])")
                .unwrap(),
            "40"
        );
        assert_eq!(
            runtime
                .eval_bytecode_native("(std.protocol.ipeekfirst/peek-first [40 42])")
                .unwrap(),
            "40"
        );
    }
}
