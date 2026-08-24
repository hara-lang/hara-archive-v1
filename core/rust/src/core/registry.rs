pub fn completion_symbols() -> &'static [&'static str] {
    fiber::completion_symbols()
}

/// Closed accounting inventory for evaluator/compiler forms. These are not a
/// native type and do not create Vars in a `std.native.Builtins` namespace.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const LANGUAGE_BUILTINS: &[(&str, &[&str])] = &[
    (
        "evaluation",
        &[
            "quote",
            "syntax-quote",
            "do",
            "if",
            "let",
            "letfn",
            "binding",
            "loop",
            "recur",
            "throw",
            "try",
            "fn",
        ],
    ),
    (
        "definitions",
        &[
            "def",
            "declare",
            "var",
            "set!",
            "defmacro",
            "defstruct",
            "defmutable",
            "defprotocol",
            "extend-type",
            "defmulti",
            "defmethod",
        ],
    ),
    ("namespaces", &["ns", "ns+", "require", "alias"]),
    ("interop", &["new", "field", "."]),
];

pub(crate) fn invoke_function_sync(
    function: Rc<Function>,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    fiber::invoke_function_sync(function, arguments)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionValue {
    pub provider: String,
    pub type_name: String,
    pub handle: u64,
}

#[derive(Debug, Clone)]
pub struct StructType {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MutableType {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StructValue {
    pub ty: Rc<StructType>,
    pub values: POrderedMap<Value, Value>,
    pub metadata: Option<Rc<Metadata>>,
}

#[derive(Debug, Clone)]
pub struct MutableValue {
    pub ty: Rc<MutableType>,
    pub values: Rc<RefCell<Vec<Value>>>,
    pub metadata: Option<Rc<Metadata>>,
}

#[derive(Debug, Clone)]
pub struct GuestProtocol {
    pub name: String,
    pub methods: HashMap<String, usize>,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NativeType {
    pub name: String,
    pub methods: Vec<String>,
    pub metadata: Option<Rc<Metadata>>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSchema {
    pub form: Form,
    pub ast: crate::kernel::SchemaType,
    pub origin: Option<KernelVar<Value>>,
}

#[derive(Clone)]
struct PackageCatalogEntry {
    descriptor: Value,
    namespaces: Vec<String>,
    state: String,
    pending: Option<Promise>,
}

#[derive(Clone, Default)]
pub struct PackageCatalog {
    entries: Rc<RefCell<HashMap<String, PackageCatalogEntry>>>,
}

impl PackageCatalog {
    pub fn register(&self, coordinate: String, descriptor: Value, namespaces: Vec<String>) {
        self.entries.borrow_mut().insert(
            coordinate,
            PackageCatalogEntry {
                descriptor,
                namespaces,
                state: "available".into(),
                pending: None,
            },
        );
    }

    fn catalog_value(&self) -> Value {
        let mut entries = self
            .entries
            .borrow()
            .iter()
            .map(|(coordinate, entry)| {
                (
                    Value::String(coordinate.clone()),
                    package_descriptor_state(&entry.descriptor, &entry.state),
                )
            })
            .collect::<Vec<_>>();
        entries.sort_by(|(left, _), (right, _)| left.display().cmp(&right.display()));
        Value::OrderedMap(Box::new(POrderedMap::from_iter(entries)))
    }

    fn find(&self, target: &str) -> Option<(String, Value)> {
        self.entries
            .borrow()
            .iter()
            .find_map(|(coordinate, entry)| {
                (coordinate == target
                    || entry.namespaces.iter().any(|namespace| namespace == target))
                .then(|| {
                    (
                        coordinate.clone(),
                        package_descriptor_state(&entry.descriptor, &entry.state),
                    )
                })
            })
    }

    pub fn contains_namespace(&self, namespace: &str) -> bool {
        self.entries
            .borrow()
            .values()
            .any(|entry| entry.namespaces.iter().any(|name| name == namespace))
    }

    fn coordinate_for_namespace(&self, namespace: &str) -> Option<String> {
        self.entries
            .borrow()
            .iter()
            .find_map(|(coordinate, entry)| {
                entry
                    .namespaces
                    .iter()
                    .any(|name| name == namespace)
                    .then(|| coordinate.clone())
            })
    }

    fn state(&self, coordinate: &str) -> Option<String> {
        self.entries
            .borrow()
            .get(coordinate)
            .map(|entry| entry.state.clone())
    }

    fn set_state(&self, coordinate: &str, state: &str) {
        if let Some(entry) = self.entries.borrow_mut().get_mut(coordinate) {
            entry.state = state.into();
        }
    }

    fn pending(&self, coordinate: &str) -> Option<Promise> {
        self.entries
            .borrow()
            .get(coordinate)
            .and_then(|entry| entry.pending.clone())
    }

    fn set_pending(&self, coordinate: &str, pending: Option<Promise>) {
        if let Some(entry) = self.entries.borrow_mut().get_mut(coordinate) {
            entry.pending = pending;
        }
    }
}

fn package_descriptor_state(descriptor: &Value, state: &str) -> Value {
    let Value::OrderedMap(values) = descriptor else {
        return descriptor.clone();
    };
    Value::OrderedMap(Box::new(POrderedMap::from_iter(
        values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .chain(std::iter::once((
                Value::Keyword("package/state".into()),
                Value::Keyword(state.into()),
            ))),
    )))
}

fn package_descriptor_coordinate(descriptor: &Value) -> Option<String> {
    let Value::OrderedMap(values) = descriptor else {
        return None;
    };
    match values.get(&Value::Keyword("package/coordinate".into())) {
        Some(Value::String(coordinate)) => Some(coordinate.clone()),
        Some(Value::Symbol(coordinate)) => Some(coordinate.as_str().to_owned()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvailability {
    Portable,
    CapabilityGated,
    InventoryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeDeclaration {
    pub namespace: &'static str,
    pub name: &'static str,
    pub methods: &'static [&'static str],
    pub availability: NativeAvailability,
    pub capability: Option<&'static str>,
}

impl NativeDeclaration {
    pub fn qualified_name(self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }

    pub fn method(self, name: &str) -> bool {
        self.methods.iter().any(|method| *method == name)
    }
}

pub const NATIVE_DECLARATIONS: &[NativeDeclaration] = DECLARATIONS_DECLARATIONS;
pub const NATIVE_TYPES: &[(&str, &[&str])] = DECLARATIONS_TYPES;

pub fn native_declarations() -> &'static [NativeDeclaration] {
    NATIVE_DECLARATIONS
}

pub fn native_type_values() -> Vec<(String, Value)> {
    NATIVE_DECLARATIONS
        .iter()
        .map(|declaration| {
            (
                declaration.name.to_owned(),
                Value::NativeType(Rc::new(NativeType {
                    name: declaration.qualified_name(),
                    methods: declaration
                        .methods
                        .iter()
                        .map(|method| (*method).to_owned())
                        .collect(),
                    metadata: None,
                })),
            )
        })
        .collect()
}

pub(crate) fn protocol_declarations() -> &'static [crate::lang::protocol::ProtocolDeclaration] {
    crate::lang::protocol::protocol_declarations()
}

pub fn builtin_protocol_namespace(protocol: &str) -> String {
    let simple = protocol.strip_prefix("std.foundation/").unwrap_or(protocol);
    crate::lang::protocol::find_protocol(simple)
        .map(|declaration| {
            if declaration.namespace.ends_with(&format!(".{}", declaration.name)) {
                declaration.namespace.to_owned()
            } else {
                format!("{}.{}", declaration.namespace, declaration.name)
            }
        })
        .unwrap_or_else(|| {
            if simple.starts_with("std.protocol.") {
                simple.to_owned()
            } else {
                format!("std.protocol.{}.{}", simple.to_ascii_lowercase(), simple)
            }
        })
}

pub(crate) fn builtin_protocol_name(protocol: &str) -> String {
    let simple = protocol.strip_prefix("std.foundation/").unwrap_or(protocol);
    crate::lang::protocol::find_protocol(simple)
        .map(|declaration| declaration.runtime_name())
        .unwrap_or_else(|| protocol.to_owned())
}

pub(crate) fn canonical_protocol_name(protocol: &str) -> String {
    let simple = protocol.strip_prefix("std.foundation/").unwrap_or(protocol);
    crate::lang::protocol::find_protocol(simple)
        .map(|declaration| declaration.runtime_name())
        .unwrap_or_else(|| protocol.to_owned())
}

pub(crate) fn canonical_intrinsic_protocol_symbol(symbol: &str) -> Option<String> {
    if let Some((protocol, method)) = symbol.rsplit_once('/') {
        let canonical = canonical_protocol_name(protocol);
        if canonical != protocol {
            return Some(format!("{canonical}/{method}"));
        }
        return None;
    }
    let canonical = canonical_protocol_name(symbol);
    (canonical != symbol).then_some(canonical)
}

pub(crate) fn canonical_intrinsic_symbol(symbol: &str) -> Option<String> {
    canonical_intrinsic_protocol_symbol(symbol).or_else(|| {
        let (native_type, method) = symbol.rsplit_once('/')?;
        NATIVE_DECLARATIONS
            .iter()
            .any(|declaration| declaration.name == native_type)
            .then(|| format!("std.native.{native_type}/{method}"))
    })
}

/// Returns the canonical identity of a callable owned by the native or
/// protocol registries. Ordinary Foundation functions deliberately do not
/// appear here: they must resolve through their namespace Vars after
/// `std.foundation` has been loaded.
pub(crate) fn canonical_intrinsic_callable_symbol(symbol: &str) -> Option<String> {
    let canonical = canonical_intrinsic_symbol(symbol).unwrap_or_else(|| symbol.to_owned());
    if let Some(native) = canonical.strip_prefix("std.native.") {
        let (native_type, method) = native.split_once('/')?;
        if NATIVE_DECLARATIONS.iter().any(|declaration| {
            declaration.name == native_type && declaration.method(method)
        }) {
            return Some(canonical);
        }
    }
    let (namespace, method) = canonical.split_once('/')?;
    protocol_declarations()
        .iter()
        .find(|declaration| builtin_protocol_namespace(declaration.name) == namespace)
        .filter(|declaration| declaration.methods.iter().any(|candidate| candidate.name == method))
        .map(|_| canonical)
}

/// Resolves a canonical native/protocol callable for bytecode instructions.
/// The registry is the only source of these values; no unqualified fallback
/// catalog is consulted.
pub(crate) fn bytecode_callable_value(name: &str) -> Result<Value, String> {
    let canonical = canonical_intrinsic_callable_symbol(name)
        .ok_or_else(|| format!("unknown canonical builtin: {name}"))?;
    let registry = namespace_registry()?;
    registry
        .resolve(&crate::lang::data::Symbol::parse(&canonical))
        .map(|var| var.deref_value())
        .ok_or_else(|| format!("unbound canonical builtin: {canonical}"))
}

pub fn foundation_protocol_values() -> Vec<(String, Value)> {
    protocol_declarations()
        .iter()
        .filter(|declaration| declaration.availability.is_guest_visible())
        .map(|declaration| {
            (
                declaration.name.to_owned(),
                Value::Protocol(Rc::new(guest_protocol(*declaration))),
            )
        })
        .collect()
}

pub fn builtin_protocol_method_values() -> Vec<(String, String, Value)> {
    protocol_declarations()
        .iter()
        .filter(|declaration| declaration.availability.is_guest_visible())
        .flat_map(|declaration| {
            declaration.methods.iter().map(move |method| {
                let namespace = builtin_protocol_namespace(declaration.name);
                let protocol_name = declaration.runtime_name();
                let method_name = method.name.to_owned();
                let display_name = format!("{namespace}/{}", method.name);
                let arity_display_name = display_name.clone();
                let (minimum_arity, maximum_arity) = method.arity.range();
                (
                    namespace,
                    method.name.to_owned(),
                    native_variadic_function(&display_name, move |arguments| {
                        if arguments.len() < minimum_arity
                            || maximum_arity.is_some_and(|maximum| arguments.len() > maximum)
                        {
                            let expected = match maximum_arity {
                                Some(maximum) if maximum == minimum_arity => {
                                    minimum_arity.to_string()
                                }
                                Some(maximum) => format!("{minimum_arity} to {maximum}"),
                                None => format!("at least {minimum_arity}"),
                            };
                            return Err(format!(
                                "protocol/arity: {arity_display_name} expects {expected} arguments, received {}",
                                arguments.len()
                            ));
                        }
                        protocol_call(&protocol_name, &method_name, &arguments)
                    }),
                )
            })
        })
        .collect()
}

fn guest_protocol(declaration: crate::lang::protocol::ProtocolDeclaration) -> GuestProtocol {
    GuestProtocol {
        name: declaration.runtime_name(),
        methods: declaration
            .methods
            .iter()
            .map(|method| (method.name.to_owned(), method.arity.guest_arity()))
            .collect(),
        parents: declaration
            .parents
            .iter()
            .map(|parent| builtin_protocol_name(parent))
            .collect(),
    }
}

#[cfg(test)]
mod native_work_protocol_tests {
    use super::*;

    fn methods(name: &str) -> Vec<(&'static str, usize)> {
        protocol_declarations()
            .iter()
            .find(|declaration| declaration.name == name)
            .map(|declaration| {
                declaration
                    .methods
                    .iter()
                    .map(|method| (method.name, method.arity.guest_arity()))
                    .collect()
            })
            .expect("protocol must exist")
    }

    fn protocol(name: &str) -> Rc<GuestProtocol> {
        foundation_protocol_values()
            .into_iter()
            .find(|(candidate, _)| candidate == name)
            .and_then(|(_, value)| match value {
                Value::Protocol(protocol) => Some(protocol),
                _ => None,
            })
            .expect("protocol value must exist")
    }

    #[test]
    fn canonical_protocol_names_preserve_std_protocol_identity() {
        assert_eq!(canonical_protocol_name("IFn"), "std.protocol.ifn.IFn");
        assert_eq!(
            canonical_protocol_name("std.foundation/IFn"),
            "std.protocol.ifn.IFn"
        );
        assert_eq!(
            canonical_protocol_name("std.protocol.ifn.IFn"),
            "std.protocol.ifn.IFn"
        );
        assert_eq!(
            canonical_protocol_name("std.protocol.application/Portable"),
            "std.protocol.application/Portable"
        );
        assert_eq!(
            canonical_intrinsic_protocol_symbol("IFn"),
            Some("std.protocol.ifn.IFn".into())
        );
        assert_eq!(
            canonical_intrinsic_protocol_symbol("IFn/invoke"),
            Some("std.protocol.ifn.IFn/invoke".into())
        );
        assert_eq!(
            canonical_intrinsic_protocol_symbol("IAssoc/assoc"),
            Some("std.protocol.iassoc.IAssoc/assoc".into())
        );
        assert_eq!(
            canonical_intrinsic_symbol("Base/vec"),
            Some("std.native.Base/vec".into())
        );
        assert_eq!(canonical_intrinsic_symbol("std.native/Base"), None);
    }

    #[test]
    fn native_work_protocol_methods_are_stable() {
        assert_eq!(methods("IWork"), vec![("work-spec", 1)]);
        assert_eq!(methods("IWorkExecutor"), vec![("work-execute", 2)]);
        assert_eq!(
            methods("IWorkStore"),
            vec![("work-query", 2), ("work-transact", 2)]
        );
        assert_eq!(methods("IWorkRef"), vec![("work-id", 1)]);
        assert_eq!(
            methods("IWorkHost"),
            vec![("work-submit", 4), ("work-resolve", 2)]
        );
        assert_eq!(
            methods("IWorkRun"),
            vec![
                ("work-status", 1),
                ("work-result", 1),
                ("work-events", 2),
                ("work-cancel", 2),
            ]
        );
    }

    #[test]
    fn native_work_protocol_parents_match_the_lifecycle_contract() {
        assert!(protocol("IWorkExecutor").parents.is_empty());
        assert!(protocol("IWorkStore").parents.is_empty());
        assert_eq!(
            protocol("IWorkHost").parents,
            vec![builtin_protocol_name("IComponent")]
        );
        assert_eq!(
            protocol("IWorkRun").parents,
            vec![
                builtin_protocol_name("IWorkRef"),
                builtin_protocol_name("IClosed"),
            ]
        );
    }
}
