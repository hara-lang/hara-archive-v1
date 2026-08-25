use hara_wasm::kernel::{parse_forms, Form};
use hara_wasm::lang::protocol::{
    find_protocol, protocol_declarations, ProtocolArity, ProtocolAvailability,
};
use hara_wasm::spec_registry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[test]
fn annotated_protocols_close_the_specs_surface() {
    let expected = read_specs();
    let actual = protocol_declarations();

    assert_eq!(actual.len(), expected.len());
    assert_eq!(
        actual
            .iter()
            .filter(|protocol| protocol.availability != ProtocolAvailability::InventoryOnly)
            .count(),
        expected.len()
    );
    assert_eq!(
        actual
            .iter()
            .map(|protocol| protocol.methods.len())
            .sum::<usize>(),
        expected
            .values()
            .map(|protocol| protocol.methods.len())
            .sum::<usize>()
    );

    let actual_names = actual
        .iter()
        .map(|protocol| protocol.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_names, expected.keys().copied().collect());

    for (name, expected) in expected {
        let declaration = find_protocol(name).expect("spec protocol declaration");
        assert_eq!(
            declaration.namespace,
            format!("std.protocol.{}", name.to_ascii_lowercase())
        );
        assert_eq!(
            declaration.parents.iter().copied().collect::<BTreeSet<_>>(),
            expected.parents
        );
        assert_eq!(
            declaration
                .methods
                .iter()
                .map(|method| (method.name, method.arity.guest_arity()))
                .collect::<BTreeMap<_, _>>(),
            expected.methods
        );
        assert_eq!(declaration.availability, expected.availability);
        assert_eq!(declaration.capability, expected.capability);
    }
}

#[test]
fn variadic_annotations_preserve_runtime_ranges() {
    let lookup = find_protocol("ILookup").unwrap().method("lookup").unwrap();
    assert_eq!(
        lookup.arity,
        ProtocolArity::Variadic {
            minimum: 2,
            maximum: Some(3)
        }
    );
    let invoke_in = find_protocol("IInvokeIn")
        .unwrap()
        .method("invoke-in")
        .unwrap();
    assert_eq!(
        invoke_in.arity,
        ProtocolArity::Variadic {
            minimum: 2,
            maximum: None
        }
    );
}

#[derive(Debug)]
struct ExpectedProtocol {
    methods: BTreeMap<&'static str, usize>,
    parents: BTreeSet<&'static str>,
    availability: ProtocolAvailability,
    capability: Option<&'static str>,
}

fn read_specs() -> BTreeMap<&'static str, ExpectedProtocol> {
    let source = std::fs::read_to_string(specs_path()).expect("protocol specs");
    let root = parse_forms(&source).expect("parse protocol specs");
    let Form::Map(root) = root.first().expect("protocol specs root") else {
        panic!("protocol specs root must be a map");
    };
    let mut protocols = BTreeMap::new();
    read_section(
        root,
        "protocols",
        ProtocolAvailability::Portable,
        None,
        &mut protocols,
    );
    read_section(
        root,
        "capability-protocols",
        ProtocolAvailability::CapabilityGated,
        Some("native-runtime-protocols"),
        &mut protocols,
    );
    protocols
}

fn read_section(
    root: &[(Form, Form)],
    section: &str,
    availability: ProtocolAvailability,
    capability: Option<&'static str>,
    protocols: &mut BTreeMap<&'static str, ExpectedProtocol>,
) {
    let Form::Vector(entries) = lookup(root, section) else {
        panic!("{section} must be a vector");
    };
    for entry in entries {
        let Form::Map(entry) = entry else {
            panic!("protocol entry must be a map");
        };
        let name = match lookup(entry, "name") {
            Form::Symbol(name) => name,
            other => panic!("protocol name must be a symbol: {other:?}"),
        };
        let name = Box::leak(name.clone().into_boxed_str());
        let Form::Map(methods) = lookup(entry, "methods") else {
            panic!("protocol methods must be a map");
        };
        let methods = methods
            .iter()
            .map(|(method, arity)| {
                let Form::Symbol(method) = method else {
                    panic!("method name must be a symbol");
                };
                let Form::Number(arity) = arity else {
                    panic!("method arity must be a number");
                };
                (
                    Box::leak(method.clone().into_boxed_str()) as &'static str,
                    if *arity == -1 {
                        usize::MAX
                    } else {
                        *arity as usize
                    },
                )
            })
            .collect();
        let parents = match entry
            .iter()
            .find(|(key, _)| key == &Form::Keyword("extends".into()))
        {
            Some((_, Form::Vector(parents))) => parents
                .iter()
                .map(|parent| match parent {
                    Form::Symbol(parent) => {
                        Box::leak(parent.clone().into_boxed_str()) as &'static str
                    }
                    other => panic!("protocol parent must be a symbol: {other:?}"),
                })
                .collect(),
            Some((_, other)) => panic!("protocol parents must be a vector: {other:?}"),
            None => BTreeSet::new(),
        };
        assert!(
            protocols
                .insert(
                    name,
                    ExpectedProtocol {
                        methods,
                        parents,
                        availability,
                        capability,
                    },
                )
                .is_none(),
            "duplicate protocol {name}"
        );
    }
}

fn lookup<'a>(map: &'a [(Form, Form)], key: &str) -> &'a Form {
    map.iter()
        .find_map(|(candidate, value)| (candidate == &Form::Keyword(key.into())).then_some(value))
        .unwrap_or_else(|| panic!("missing :{key}"))
}

fn specs_path() -> PathBuf {
    spec_registry::require("01-lang/001-language/draft/conformance/protocols.edn")
}
