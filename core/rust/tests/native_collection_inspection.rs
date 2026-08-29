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
        "(std.native.Algo/ordered-set :beta :alpha)",
        "(std.native.Algo/sorted-set :beta :alpha)",
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
fn canonical_peek_first_protocol_alias_is_available_without_legacy_method_aliases() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native("(std.protocol.ipeekfirst.IPeekFirst/peek-first [40 42])")
            .unwrap(),
        "40"
    );
    assert!(runtime
        .eval_native("std.protocol.ipeekfirst/peek-first")
        .is_err());
    assert!(runtime.eval_native("std.foundation/IPeekFirst").is_ok());

    #[cfg(feature = "bytecode-vm")]
    assert_eq!(
        runtime
            .eval_bytecode_native("(std.protocol.ipeekfirst.IPeekFirst/peek-first [40 42])")
            .unwrap(),
        "40"
    );
}

#[test]
fn foundation_first_and_last_use_indexed_collections_before_iteration() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "[(first [40 41 42]) (first [40 41 42]) (last [40 41 42]) (last [40 41 42])]"
            )
            .unwrap(),
        "[40 40 42 42]"
    );
}

#[test]
fn portable_collection_categories_classify_by_protocol_parents() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "[(satisfies? IMapType {:a 1})\
                 (satisfies? IMapType [1])\
                 (satisfies? ISetType #{1})\
                 (satisfies? ISetType [1])\
                 (satisfies? ILinearType [1])\
                 (satisfies? ILinearType #{1})\
                 (map? {:a 1}) (map? [1])\
                 (set? #{1}) (set? [1])\
                 (sequential? [1]) (sequential? #{1})]"
            )
            .unwrap(),
        "[true false true false true false true false true false true false]"
    );
}

#[test]
fn portable_collection_categories_classify_all_portable_families() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "[(map? {:a 1})\
                 (map? (std.native.Algo/ordered-map :a 1))\
                 (map? [1])\
                 (set? #{1})\
                 (set? (std.native.Algo/ordered-set 1))\
                 (set? [1])\
                 (sequential? '(1 2))\
                 (sequential? [1 2])\
                 (sequential? (vector 1 2))\
                 (sequential? (std.native.Algo/queue 1 2))\
                 (sequential? (std.native.Algo/deque 1 2))\
                 (sequential? (cons 1 [2]))\
                 (sequential? (seq [1 2]))\
                 (sequential? (std.native.Algo/ordered-set 1))\
                 (coll? (seq [1 2]))\
                 (coll? (iter [1 2]))\
                 (seq? (seq [1 2]))\
                 (seq? [1 2])\
                 (iter? (iter [1 2]))\
                 (iter? [1 2])]"
            )
            .unwrap(),
        "[true true false true true false true true true true true true true false false false true false true false]"
    );
}

#[test]
fn sequential_and_linear_categories_remain_distinct() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "[(satisfies? ISequential (seq [1 2]))\
                 (satisfies? ILinearType (seq [1 2]))\
                 (satisfies? ISequential (cons 1 [2]))\
                 (satisfies? ILinearType (cons 1 [2]))\
                 (satisfies? ILinearType [1 2])]"
            )
            .unwrap(),
        "[true false true false true]"
    );
}

#[test]
fn not_and_compare_are_foundation_owned() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "[(std.foundation/not nil)\
                 (std.foundation/not true)\
                 (std.foundation/compare 1 2)\
                 (std.foundation/compare 2 2)\
                 (std.foundation/compare 2 1)]"
            )
            .unwrap(),
        "[true false -1 0 1]"
    );
}

#[test]
fn foundation_protocol_predicates_use_satisfies() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "[(iterable? [1])\
                 (iterator? (iter [1]))\
                 (counted? [1])\
                 (reducible? [1])\
                 (indexed? [1])\
                 (associative? {:a 1})\
                 (findable? {:a 1})\
                 (lookupable? {:a 1})\
                 (derefable? (atom 1))\
                 (resettable? (atom 1))\
                 (casable? (atom 1))\
                 (watchable? (atom 1))\
                 (applicable? (pointer {:context :test}))\
                 (mutable? (to-mutable (vec [1])))\
                 (persistent? [1])]"
            )
            .unwrap(),
        "[true true true true true true true true true true true true true true true]"
    );
}
