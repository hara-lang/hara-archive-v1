fn builtin_protocol_owner_namespace(interface_namespace: &str) -> &str {
    interface_namespace
        .rsplit_once('.')
        .map(|(owner, _)| owner)
        .expect("builtin protocol interface namespace")
}

fn map_builtin_protocol_products(
    namespaces: &NamespaceRegistry<Value>,
    interface_namespace: &str,
    local_name: &str,
    var: crate::kernel::Var<Value>,
) {
    namespaces
        .find_or_create(interface_namespace)
        .map_var(Symbol::parse(local_name), var.clone());
    namespaces
        .find_or_create(builtin_protocol_owner_namespace(interface_namespace))
        .map_var(Symbol::parse(local_name), var);
}

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
    for (native_type, methods) in NATIVE_TYPES {
        let namespace = namespaces.find_or_create(format!("std.native.{native_type}"));
        for method in *methods {
            namespace.intern_with_origin(
                *method,
                native_type_function_value(native_type, method)
                    .unwrap_or_else(|error| panic!("{error}")),
                VarOrigin::RuntimePrimitive,
            );
        }
    }

    for (name, protocol) in foundation_protocol_values() {
        let interface_namespace = builtin_protocol_namespace(&name);
        let var = crate::kernel::Var::with_metadata(
            &interface_namespace,
            protocol,
            crate::kernel::VarMetadata {
                origin: VarOrigin::RuntimePrimitive,
                ..crate::kernel::VarMetadata::default()
            },
        );
        map_builtin_protocol_products(&namespaces, &interface_namespace, &name, var);
    }
    for (interface_namespace, name, method) in builtin_protocol_method_values() {
        let var = namespaces
            .find_or_create(&interface_namespace)
            .intern_with_origin(&name, method, VarOrigin::RuntimePrimitive);
        map_builtin_protocol_products(&namespaces, &interface_namespace, &name, var);
    }

    namespaces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::protocol::INamespaced;

    #[test]
    fn intrinsic_type_vars_publish_named_declaration_products() {
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

        let interface_type = namespaces
            .resolve(&Symbol::parse("std.protocol.iassoc.IAssoc"))
            .expect("protocol interface type");
        let protocol_var = namespaces
            .resolve(&Symbol::parse("std.protocol.iassoc/IAssoc"))
            .expect("protocol namespace var");
        assert!(interface_type.same_identity(&protocol_var));
        assert_eq!(
            interface_type.symbol().as_str(),
            "std.protocol.iassoc.IAssoc"
        );
        assert_eq!(interface_type.origin(), VarOrigin::RuntimePrimitive);
        assert_eq!(protocol_var.origin(), VarOrigin::RuntimePrimitive);

        let interface_method = namespaces
            .resolve(&Symbol::parse("std.protocol.iassoc.IAssoc/assoc"))
            .expect("interface-qualified protocol method");
        let protocol_method = namespaces
            .resolve(&Symbol::parse("std.protocol.iassoc/assoc"))
            .expect("namespace-qualified protocol method");
        assert!(interface_method.same_identity(&protocol_method));
        assert_eq!(
            interface_method.symbol().as_str(),
            "std.protocol.iassoc.IAssoc/assoc"
        );
        assert_eq!(interface_method.origin(), VarOrigin::RuntimePrimitive);
        assert_eq!(protocol_method.origin(), VarOrigin::RuntimePrimitive);

        assert!(namespaces.resolve(&Symbol::parse("IAssoc")).is_none());
        assert!(namespaces.resolve(&Symbol::parse("assoc")).is_none());
    }
}
