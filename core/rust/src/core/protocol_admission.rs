fn mutable_assoc_satisfies(value: &Value) -> bool {
    let Value::MutableCollection(collection) = value else {
        return false;
    };
    let borrowed = collection.borrow();
    let Some(collection) = borrowed.as_ref() else {
        return false;
    };
    matches!(
        collection,
        MutableCollection::Map(_)
            | MutableCollection::OrderedMap(_)
            | MutableCollection::SortedMap(_)
            | MutableCollection::Trie(_)
            | MutableCollection::Vector(_)
            | MutableCollection::List(_)
    )
}

fn mutable_dissoc_satisfies(value: &Value) -> bool {
    let Value::MutableCollection(collection) = value else {
        return false;
    };
    let borrowed = collection.borrow();
    let Some(collection) = borrowed.as_ref() else {
        return false;
    };
    matches!(
        collection,
        MutableCollection::Map(_)
            | MutableCollection::OrderedMap(_)
            | MutableCollection::SortedMap(_)
            | MutableCollection::Trie(_)
            | MutableCollection::Set(_)
            | MutableCollection::OrderedSet(_)
            | MutableCollection::SortedSet(_)
    )
}

fn native_assoc_satisfies(value: &Value) -> bool {
    Value::supports_native_map(value)
        || matches!(
            value,
            Value::Nil
                | Value::Tuple(_)
                | Value::Vector(_)
                | Value::Deque(_)
                | Value::Object(_)
                | Value::Struct(_)
        )
        || mutable_assoc_satisfies(value)
}

fn native_dissoc_satisfies(value: &Value) -> bool {
    Value::supports_native_map(value)
        || matches!(
            value,
            Value::Nil
                | Value::Set(_)
                | Value::OrderedSet(_)
                | Value::SortedSet(_)
                | Value::Struct(_)
        )
        || mutable_dissoc_satisfies(value)
}

fn replace_native_protocol_implementation<S, F>(
    protocols: &mut ProtocolRegistry,
    protocol: &str,
    method: &str,
    supports: S,
    function: F,
) where
    S: Fn(&Value) -> bool + 'static,
    F: Fn(&[Value]) -> Result<Value, String> + 'static,
{
    protocols.methods.borrow_mut().insert(
        (canonical_protocol_name(protocol), method.to_owned()),
        vec![ProtocolImplementation {
            supports: Rc::new(supports),
            invoke: Rc::new(function),
        }],
    );
}

/// Replaces the broad bootstrap predicates for the two associative protocols
/// with the exact receiver sets already implemented by collection_assoc and
/// collection_dissoc. Extension and guest implementations remain separate.
pub(crate) fn install_native_collection_protocol_admission(
    protocols: &mut ProtocolRegistry,
) {
    replace_native_protocol_implementation(
        protocols,
        "std.protocol.iassoc.IAssoc",
        "assoc",
        native_assoc_satisfies,
        protocol_assoc,
    );
    replace_native_protocol_implementation(
        protocols,
        "std.protocol.idissoc.IDissoc",
        "dissoc",
        native_dissoc_satisfies,
        protocol_dissoc,
    );
}

#[cfg(test)]
mod protocol_admission_tests {
    use super::*;

    fn registry() -> ProtocolRegistry {
        let mut protocols = ProtocolRegistry::core();
        install_native_collection_protocol_admission(&mut protocols);
        protocols
    }

    fn invoke_assoc(protocols: &ProtocolRegistry, receiver: Value) -> Result<Value, String> {
        protocols.invoke(
            "std.protocol.iassoc.IAssoc",
            "assoc",
            &[
                receiver,
                Value::Number(0),
                Value::String("replacement".into()),
            ],
        )
    }

    #[test]
    fn assoc_admission_matches_implemented_immutable_receivers() {
        let protocols = registry();
        let values = [
            Value::Nil,
            Value::Tuple(Box::new(PTuple::Tup1([Value::Number(1)]))),
            Value::Vector([Value::Number(1)].into_iter().collect()),
            Value::Deque(Box::new([Value::Number(1)].into_iter().collect())),
            Value::Map(PMap::new()),
            Value::Object(Rc::new(RefCell::new(Vec::new()))),
        ];
        for value in values {
            assert!(
                invoke_assoc(&protocols, value).is_ok(),
                "admitted assoc receiver must invoke the implementation"
            );
        }
    }

    #[test]
    fn assoc_admission_rejects_unimplemented_receivers() {
        let protocols = registry();
        for value in [
            Value::String("not-associative".into()),
            Value::Set(PSet::new()),
            Value::List(PList::new()),
        ] {
            let error = invoke_assoc(&protocols, value).unwrap_err();
            assert!(error.contains("protocol/unsupported-receiver"), "{error}");
        }
    }

    #[test]
    fn dissoc_admission_matches_maps_sets_and_nil() {
        let protocols = registry();
        for value in [Value::Map(PMap::new()), Value::Set(PSet::new()), Value::Nil] {
            let result = protocols.invoke(
                "std.protocol.idissoc.IDissoc",
                "dissoc",
                &[value, Value::Keyword("missing".into())],
            );
            assert!(result.is_ok(), "admitted dissoc receiver must invoke");
        }
    }

    #[test]
    fn dissoc_does_not_inherit_assoc_only_receivers() {
        let protocols = registry();
        for value in [
            Value::Tuple(Box::new(PTuple::Tup1([Value::Number(1)]))),
            Value::Vector([Value::Number(1)].into_iter().collect()),
            Value::Deque(Box::new([Value::Number(1)].into_iter().collect())),
            Value::Object(Rc::new(RefCell::new(Vec::new()))),
        ] {
            let error = protocols
                .invoke(
                    "std.protocol.idissoc.IDissoc",
                    "dissoc",
                    &[value, Value::Number(0)],
                )
                .unwrap_err();
            assert!(error.contains("protocol/unsupported-receiver"), "{error}");
        }
    }
}
