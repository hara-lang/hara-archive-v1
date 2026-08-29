/// Installs the runtime-owned substrate required by canonical Foundation
/// source. Ordinary Foundation functions and protocol method aliases are
/// intentionally absent; canonical protocol values are seeded so Foundation
/// predicates can resolve them while the source modules load.
const FOUNDATION_INTRINSICS: &[&str] = &[
    "+", "-", "*", "/", "=", "<", "<=", ">", ">=", "quot", "rem", "mod", "disj",
];

/// Installs the small language-level callable substrate needed while loading
/// `std.foundation`. These are runtime primitives, not a second public native
/// catalog: the Foundation source can replace or wrap them with ordinary HAL
/// Vars, and every installed Var carries `RuntimePrimitive` provenance.
pub fn install_foundation_intrinsics(namespaces: &NamespaceRegistry<Value>) {
    let foundation = namespaces.find_or_create("std.foundation");
    for declaration in NATIVE_DECLARATIONS {
        let qualified = Symbol::parse(&declaration.qualified_name());
        if let Some(descriptor) = namespaces.resolve(&qualified) {
            foundation.map_var(Symbol::parse(declaration.name), descriptor);
        }
    }
    for (name, _) in foundation_protocol_values() {
        let namespace = builtin_protocol_namespace(&name);
        let qualified = Symbol::parse(&format!("{namespace}/{name}"));
        if let Some(protocol) = namespaces.resolve(&qualified) {
            foundation.map_var(Symbol::parse(&name), protocol);
        }
    }
    for name in FOUNDATION_INTRINSICS {
        let value = crate::core::direct_function_value(name)
            .unwrap_or_else(|| panic!("missing Foundation intrinsic implementation: {name}"));
        foundation.intern_with_origin(name, value, VarOrigin::RuntimePrimitive);
    }
    let iterator = NATIVE_DECLARATIONS
        .iter()
        .find(|declaration| declaration.name == "Iter")
        .expect("annotated Iter declaration");
    for method in iterator.methods {
        foundation.intern_with_origin(
            method,
            native_type_function_value(iterator.name, method)
                .unwrap_or_else(|error| panic!("{error}")),
            VarOrigin::RuntimePrimitive,
        );
    }
    for (name, value) in crate::core::exception_function_values() {
        foundation.intern_with_origin(name, value, VarOrigin::RuntimePrimitive);
    }
}

pub fn minimal_namespace_registry() -> NamespaceRegistry<Value> {
    let namespaces = NamespaceRegistry::new("user");

    for declaration in NATIVE_DECLARATIONS {
        let name = declaration.name;
        let path = declaration.qualified_name();
        let namespace = namespaces.find_or_create(&path);
        let var = crate::kernel::Var::with_metadata(
            &path,
            native_descriptor_value(*declaration),
            crate::kernel::VarMetadata {
                origin: VarOrigin::RuntimePrimitive,
                ..crate::kernel::VarMetadata::default()
            },
        );
        namespace.map_var(Symbol::parse(&name), var);
        namespaces
            .register_global_alias(declaration.name, declaration.qualified_name())
            .unwrap_or_else(|error| panic!("{error}"));
        for method in declaration.methods {
            namespace.intern_with_origin(
                method,
                native_type_function_value(declaration.name, method)
                    .unwrap_or_else(|error| panic!("{error}")),
                VarOrigin::RuntimePrimitive,
            );
        }
    }

    // `pair` is the one representation constructor retained by Foundation.
    // Keep its host implementation under the runtime-only `global` namespace
    // so the source-owned `std.foundation/pair` facade can delegate to it
    // without exposing another public constructor name.
    let global = namespaces.find_or_create("global");
    global.intern_with_origin(
        "pair",
        crate::core::direct_function_value("pair")
            .unwrap_or_else(|| panic!("missing global pair implementation")),
        VarOrigin::RuntimePrimitive,
    );

    for (name, protocol) in foundation_protocol_values() {
        let declaration = crate::lang::protocol::find_protocol(&name)
            .unwrap_or_else(|| panic!("annotated protocol declaration missing: {name}"));
        let namespace_name = declaration.runtime_name();
        let namespace = namespaces.find_or_create(&namespace_name);
        let var = crate::kernel::Var::with_metadata(
            &namespace_name,
            protocol,
            crate::kernel::VarMetadata {
                origin: VarOrigin::RuntimePrimitive,
                ..crate::kernel::VarMetadata::default()
            },
        );
        namespace.map_var(Symbol::parse(&name), var);
        namespaces
            .register_global_alias(&name, &namespace_name)
            .unwrap_or_else(|error| panic!("{error}"));
    }
    for (namespace, name, method) in builtin_protocol_method_values() {
        namespaces.find_or_create(namespace).intern_with_origin(
            name,
            method,
            VarOrigin::RuntimePrimitive,
        );
    }

    namespaces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intrinsic_type_vars_use_only_canonical_symbols() {
        let namespaces = minimal_namespace_registry();

        assert!(namespaces.find("std.foundation").is_none());
        assert!(namespaces.find("std.native").is_none());
        assert!(namespaces.find("std.native.Builtins").is_none());

        for name in ["Base", "String"] {
            let path = format!("std.native.{name}");
            let namespace = namespaces.find(&path).expect("native namespace");
            let var = namespace
                .resolve(&Symbol::parse(name))
                .expect("canonical native type");
            assert_eq!(var.symbol().as_str(), path);
            assert_eq!(var.origin(), VarOrigin::RuntimePrimitive);
            assert_eq!(
                namespaces
                    .resolve(&Symbol::parse(&path))
                    .expect("canonical native type resolution")
                    .symbol()
                    .as_str(),
                path
            );
        }

        for (protocol, method) in [("IAssoc", "assoc"), ("IPeekFirst", "peek-first")] {
            let canonical = crate::lang::protocol::find_protocol(protocol)
                .expect("protocol declaration")
                .runtime_name();
            let namespace = namespaces.find(&canonical).expect("protocol namespace");
            let var = namespace
                .resolve(&Symbol::parse(protocol))
                .expect("canonical protocol");
            assert_eq!(var.symbol().as_str(), canonical);
            assert_eq!(var.origin(), VarOrigin::RuntimePrimitive);
            assert!(namespaces
                .resolve(&Symbol::parse(&format!(
                    "std.protocol.{}/{}",
                    protocol.to_ascii_lowercase(),
                    method
                )))
                .is_none());
            assert!(namespaces
                .resolve(&Symbol::parse(&format!("{canonical}/{method}")))
                .is_some());
        }
    }
}
