/// Installs the runtime-owned substrate required by canonical Foundation
/// source. Ordinary Foundation functions and compatibility aliases are
/// intentionally absent; they are defined by the source modules themselves.
pub fn minimal_namespace_registry() -> NamespaceRegistry<Value> {
    let namespaces = NamespaceRegistry::new("user");
    let foundation = namespaces.find_or_create("std.foundation");
    let native = namespaces.find_or_create("std.native");

    for (name, descriptor) in native_type_values() {
        let var = native.intern_with_origin(
            &name,
            descriptor,
            VarOrigin::RuntimePrimitive,
        );
        foundation.map_var(Symbol::parse(&format!("std.native.{name}")), var);
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
        let namespace_name = builtin_protocol_namespace(&name);
        let namespace = namespaces.find_or_create(&namespace_name);
        let var = namespace.intern_with_origin(&name, protocol, VarOrigin::RuntimePrimitive);
        foundation.map_var(Symbol::parse(&namespace_name), var);
    }
    for (namespace, name, method) in builtin_protocol_method_values() {
        namespaces
            .find_or_create(namespace)
            .intern_with_origin(name, method, VarOrigin::RuntimePrimitive);
    }
    for (name, value) in exception_function_values() {
        foundation.intern_with_origin(name, value, VarOrigin::RuntimePrimitive);
    }

    namespaces
}
