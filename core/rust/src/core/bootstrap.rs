/// Installs the runtime-owned substrate required by canonical Foundation
/// source. Ordinary Foundation functions and compatibility aliases are
/// intentionally absent; they are defined by the source modules themselves.
pub fn minimal_namespace_registry() -> NamespaceRegistry<Value> {
    let namespaces = NamespaceRegistry::new("user");

    for (name, descriptor) in native_type_values() {
        let path = format!("std.native.{name}");
        let namespace = namespaces.find_or_create(&path);
        let var = crate::kernel::Var::with_metadata(
            &path,
            descriptor,
            crate::kernel::VarMetadata {
                origin: VarOrigin::RuntimePrimitive,
                ..crate::kernel::VarMetadata::default()
            },
        );
        namespace.map_var(Symbol::parse(&name), var);
    }
    for declaration in NATIVE_DECLARATIONS {
        let namespace = namespaces.find_or_create(declaration.qualified_name());
        for method in declaration.methods {
            namespace.intern_with_origin(
                method,
                native_type_function_value(declaration.name, method)
                    .unwrap_or_else(|error| panic!("{error}")),
                VarOrigin::RuntimePrimitive,
            );
        }
    }

    for (name, protocol) in foundation_protocol_values() {
        let namespace_name = builtin_protocol_namespace(&name);
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
    use crate::lang::protocol::INamespaced;

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
            let canonical = builtin_protocol_namespace(protocol);
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
