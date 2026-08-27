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

/// One field in a named value declaration. The runtime only needs the name
/// for storage, while the source declaration keeps the schema and optional
/// field properties beside it until the type Var is published.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedField {
    pub name: String,
    pub properties: Option<Form>,
    pub schema: Form,
}

impl NamedField {
    pub(crate) fn from_form(form: &Form, kind: &str) -> Result<Self, String> {
        let Form::Vector(parts) = form else {
            return Err(format!("{kind} fields must be symbols or [name schema] vectors"));
        };
        let (name, properties, schema) = match parts.as_slice() {
            [Form::Symbol(name), schema] => (name, None, schema),
            [Form::Symbol(name), Form::Map(properties), schema] => {
                (name, Some(Form::Map(properties.clone())), schema)
            }
            _ => {
                return Err(format!(
                    "{kind} fields must be [name schema] or [name properties schema]"
                ))
            }
        };
        if name.is_empty() || name.contains('/') {
            return Err(format!("{kind} field names must be unqualified symbols"));
        }
        Ok(Self {
            name: name.clone(),
            properties,
            schema: schema.clone(),
        })
    }

    pub(crate) fn legacy(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            properties: None,
            schema: Form::Keyword("any".into()),
        }
    }

    pub(crate) fn from_value(value: &Value, kind: &str) -> Result<Self, String> {
        match value {
            Value::String(name) if !name.is_empty() && !name.contains('/') => {
                Ok(Self::legacy(name))
            }
            Value::Vector(_) | Value::Tuple(_) => {
                let form = value_to_form(value)?;
                Self::from_form(&form, kind)
            }
            _ => Err(format!(
                "{kind} fields must contain field names or field specification vectors"
            )),
        }
    }

    pub(crate) fn schema_form(&self) -> Form {
        let mut parts = vec![Form::Keyword(self.name.clone())];
        if let Some(properties) = &self.properties {
            parts.push(properties.clone());
        }
        parts.push(self.schema.clone());
        Form::Vector(parts)
    }
}

/// Canonical declaration data shared by a named type, its constructors, and
/// the schema exposed through the type Var.  The runtime keeps this beside
/// the type object rather than registering a second schema-owned identity.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedDeclaration {
    pub name: String,
    pub mutable: bool,
    pub fields: Vec<NamedField>,
    pub schema: Form,
    pub positional_constructor: String,
    pub map_constructor: String,
}

impl NamedDeclaration {
    pub(crate) fn new(name: String, mutable: bool, fields: Vec<NamedField>, schema: Form) -> Self {
        let local_name = name.rsplit('/').next().unwrap_or(&name).to_owned();
        Self {
            name,
            mutable,
            fields,
            schema,
            positional_constructor: format!("->{local_name}"),
            map_constructor: format!("map->{local_name}"),
        }
    }
}

pub(crate) fn named_value_schema_form(
    type_name: &str,
    mutable: bool,
    fields: &[NamedField],
) -> Form {
    let mut parts = vec![Form::Keyword("struct".into())];
    if mutable {
        parts.push(Form::Map(vec![(
            Form::Keyword("mutable?".into()),
            Form::Bool(true),
        )]));
    }
    parts.push(Form::List(vec![
        Form::Symbol("var".into()),
        Form::Symbol(type_name.to_owned()),
    ]));
    parts.extend(fields.iter().map(NamedField::schema_form));
    Form::Vector(parts)
}

#[derive(Debug, Clone)]
pub struct StructType {
    pub name: String,
    pub fields: Vec<String>,
    pub declaration: Option<Rc<NamedDeclaration>>,
}

impl StructType {
    pub(crate) fn detached(name: String, fields: Vec<String>) -> Self {
        Self {
            name,
            fields,
            declaration: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MutableType {
    pub name: String,
    pub fields: Vec<String>,
    pub declaration: Option<Rc<NamedDeclaration>>,
}

impl MutableType {
    #[cfg(test)]
    pub(crate) fn detached(name: String, fields: Vec<String>) -> Self {
        Self {
            name,
            fields,
            declaration: None,
        }
    }
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
    pub availability: NativeAvailability,
    pub capability: Option<String>,
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
    name: Option<String>,
    namespaces: Vec<String>,
    state: String,
    pending: Option<Promise>,
}

#[derive(Clone, Default)]
pub struct PackageCatalog {
    entries: Rc<RefCell<HashMap<String, PackageCatalogEntry>>>,
}

impl PackageCatalog {
    pub fn register(
        &self,
        coordinate: String,
        name: Option<String>,
        descriptor: Value,
        namespaces: Vec<String>,
    ) {
        self.entries.borrow_mut().insert(
            coordinate,
            PackageCatalogEntry {
                descriptor,
                name,
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
                    || entry.name.as_deref() == Some(target)
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
pub struct NativeOperationDeclaration {
    pub name: &'static str,
    pub arity: u16,
}

pub type NativeProvider = fn(&str, &str) -> Result<Value, String>;

#[derive(Debug, Clone, Copy)]
pub struct NativeDeclaration {
    pub namespace: &'static str,
    pub name: &'static str,
    pub methods: &'static [&'static str],
    pub whole_wasm_methods: &'static [NativeOperationDeclaration],
    pub provider: NativeProvider,
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

    pub fn whole_wasm_method(self, name: &str) -> Option<NativeOperationDeclaration> {
        self.whole_wasm_methods
            .iter()
            .copied()
            .find(|method| method.name == name)
    }
}

pub const NATIVE_DECLARATIONS: &[NativeDeclaration] = DECLARATIONS_DECLARATIONS;

pub fn native_declarations() -> &'static [NativeDeclaration] {
    NATIVE_DECLARATIONS
}

pub(crate) fn native_descriptor_value(declaration: NativeDeclaration) -> Value {
    Value::NativeType(Rc::new(NativeType {
        name: declaration.qualified_name(),
        methods: declaration
            .methods
            .iter()
            .map(|method| (*method).to_owned())
            .collect(),
        availability: declaration.availability,
        capability: declaration.capability.map(str::to_owned),
        metadata: None,
    }))
}

pub fn native_type_values() -> Vec<(String, Value)> {
    NATIVE_DECLARATIONS
        .iter()
        .map(|declaration| {
            (declaration.name.to_owned(), native_descriptor_value(*declaration))
        })
        .collect()
}

/// Returns the closed native declaration surface in a stable, comparison-friendly form.
///
/// This is intentionally a derived inspection view. The annotations remain the source of
/// truth; the manifest only makes the Rust and Java declaration surfaces comparable in tests
/// and diagnostics.
pub fn native_manifest() -> Vec<String> {
    let mut manifest = NATIVE_DECLARATIONS
        .iter()
        .map(|declaration| {
            let mut methods = declaration
                .methods
                .iter()
                .map(|method| format!("std.native.{}/{}", declaration.name, method))
                .collect::<Vec<_>>();
            methods.sort();
            format!(
                "native|std.native.{}|{}|{}|annotation|{}",
                declaration.name,
                native_availability_name(declaration.availability),
                declaration.capability.unwrap_or_default(),
                methods.join(",")
            )
        })
        .collect::<Vec<_>>();
    manifest.sort();
    manifest
}

/// Returns the closed annotated protocol surface in a stable, comparison-friendly form.
///
/// Protocol method arities use the declaration arity (`-1` for variadic methods), and method
/// entries carry their canonical runtime origin. Inherited methods are not copied into a child;
/// the parent list is part of the manifest instead.
pub fn protocol_manifest() -> Vec<String> {
    let mut manifest = protocol_declarations()
        .iter()
        .map(|declaration| {
            let mut parents = declaration
                .parents
                .iter()
                .map(|parent| (*parent).to_owned())
                .collect::<Vec<_>>();
            parents.sort();
            let mut methods = declaration
                .methods
                .iter()
                .map(|method| {
                    format!(
                        "{}/{}:{}",
                        declaration.runtime_name(),
                        method.name,
                        protocol_arity_name(method.arity)
                    )
                })
                .collect::<Vec<_>>();
            methods.sort();
            format!(
                "protocol|{}|{}|{}|{}|annotation|{}|{}",
                declaration.runtime_name(),
                declaration.name,
                protocol_availability_name(declaration.availability),
                declaration.capability.unwrap_or_default(),
                parents.join(","),
                methods.join(",")
            )
        })
        .collect::<Vec<_>>();
    manifest.sort();
    manifest
}

fn native_availability_name(availability: NativeAvailability) -> &'static str {
    match availability {
        NativeAvailability::Portable => "portable",
        NativeAvailability::CapabilityGated => "capability-gated",
        NativeAvailability::InventoryOnly => "inventory-only",
    }
}

fn protocol_availability_name(
    availability: crate::lang::protocol::ProtocolAvailability,
) -> &'static str {
    match availability {
        crate::lang::protocol::ProtocolAvailability::Portable => "portable",
        crate::lang::protocol::ProtocolAvailability::CapabilityGated => "capability-gated",
        crate::lang::protocol::ProtocolAvailability::InventoryOnly => "inventory-only",
    }
}

fn protocol_arity_name(arity: crate::lang::protocol::ProtocolArity) -> String {
    match arity {
        crate::lang::protocol::ProtocolArity::Fixed(value) => value.to_string(),
        crate::lang::protocol::ProtocolArity::Variadic { .. } => "-1".into(),
    }
}

pub(crate) fn protocol_declarations() -> &'static [crate::lang::protocol::ProtocolDeclaration] {
    crate::lang::protocol::protocol_declarations()
}

pub fn builtin_protocol_namespace(protocol: &str) -> String {
    let simple = protocol.strip_prefix("std.foundation/").unwrap_or(protocol);
    crate::lang::protocol::find_protocol(simple)
        .map(|declaration| declaration.runtime_name())
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
    builtin_protocol_name(protocol)
}

pub(crate) fn canonical_intrinsic_protocol_symbol(symbol: &str) -> Option<String> {
    let (protocol, method) = symbol.rsplit_once('/')?;
    let canonical = canonical_protocol_name(protocol);
    (canonical != protocol).then(|| format!("{canonical}/{method}"))
}

/// Resolves the short spelling of an annotated native type to its registered
/// namespace. Protocol names deliberately do not go through this function:
/// their aliases are installed from the protocol declaration registry and
/// ordinary namespace resolution must handle them like every other alias.
pub(crate) fn canonical_native_symbol(symbol: &str) -> Option<String> {
    // `file/*` is the long-standing lowercase spelling used by the portable
    // Hara library.  Keep it as a compatibility alias for the annotated
    // `std.native.File` surface so source compilation and tree evaluation
    // agree on the same callable identity.
    if let Some(method) = symbol.strip_prefix("file/") {
        return Some(format!("std.native.File/{method}"));
    }
    if let Some(method) = symbol.strip_prefix("os/") {
        return Some(format!("std.native.OS/{method}"));
    }
    if NATIVE_DECLARATIONS
        .iter()
        .any(|declaration| declaration.name == symbol)
    {
        return Some(format!("std.native.{symbol}"));
    }
    let (native_type, method) = symbol.rsplit_once('/')?;
    NATIVE_DECLARATIONS
        .iter()
        .any(|declaration| declaration.name == native_type)
        .then(|| format!("std.native.{native_type}/{method}"))
}

pub(crate) fn canonical_intrinsic_symbol(symbol: &str) -> Option<String> {
    canonical_intrinsic_protocol_symbol(symbol).or_else(|| canonical_native_symbol(symbol))
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
        .find(|declaration| declaration.runtime_name() == namespace)
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
                let protocol_name = declaration.runtime_name();
                let namespace = protocol_name.clone();
                let method_name = method.name.to_owned();
                let display_name = format!("{namespace}/{}", method.name);
                let arity_display_name = display_name.clone();
                let (minimum_arity, maximum_arity) = method.arity.range();
                let value = if protocol_name == "std.protocol.ideref.IDeref" && method.name == "deref" {
                    native_protocol_fiber_function(
                        &display_name,
                        &protocol_name,
                        &method_name,
                        minimum_arity,
                        maximum_arity.is_none(),
                        {
                            let protocol_name = protocol_name.clone();
                            let method_name = method_name.clone();
                            move |arguments| protocol_call(&protocol_name, &method_name, &arguments)
                        },
                        protocol_deref_fiber,
                    )
                } else if protocol_name == "std.protocol.icoroutine.ICoroutine"
                    && method.name == "resume"
                {
                    native_protocol_fiber_function(
                        &display_name,
                        &protocol_name,
                        &method_name,
                        minimum_arity,
                        maximum_arity.is_none(),
                        {
                            let protocol_name = protocol_name.clone();
                            let method_name = method_name.clone();
                            move |arguments| protocol_call(&protocol_name, &method_name, &arguments)
                        },
                        protocol_coroutine_resume_fiber,
                    )
                } else {
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
                    })
                };
                (
                    namespace,
                    method.name.to_owned(),
                    value,
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
            .map(|parent| {
                crate::lang::protocol::find_protocol(parent)
                    .map(|declaration| declaration.runtime_name())
                    .unwrap_or_else(|| (*parent).to_owned())
            })
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
    fn protocol_aliases_resolve_to_annotation_owned_namespaces() {
        let namespaces = crate::core::minimal_namespace_registry();
        let assoc = namespaces
            .resolve(&crate::lang::data::Symbol::parse("IAssoc/assoc"))
            .expect("annotated protocol alias");
        assert_eq!(
            assoc.symbol().as_str(),
            "std.protocol.iassoc.IAssoc/assoc"
        );
        assert_eq!(
            crate::lang::protocol::find_protocol("IAssoc")
                .expect("annotated protocol")
                .runtime_name(),
            "std.protocol.iassoc.IAssoc"
        );
        assert_eq!(
            canonical_native_symbol("Base/vec"),
            Some("std.native.Base/vec".into())
        );
        assert_eq!(canonical_native_symbol("std.native/Base"), None);
        assert_eq!(
            canonical_native_symbol("Coroutine"),
            Some("std.native.Coroutine".into())
        );
        assert_eq!(
            canonical_native_symbol("file/join"),
            Some("std.native.File/join".into())
        );
        assert_eq!(
            canonical_native_symbol("os/cwd"),
            Some("std.native.OS/cwd".into())
        );
    }

    #[test]
    fn native_registry_rejects_unknown_annotated_methods() {
        let error = crate::core::native_type_function_value("String", "missing").unwrap_err();
        assert_eq!(
            error,
            "unknown annotated native method: std.native.String/missing"
        );
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
            vec![crate::lang::protocol::find_protocol("IComponent")
                .expect("annotated protocol")
                .runtime_name()]
        );
        assert_eq!(
            protocol("IWorkRun").parents,
            vec![
                crate::lang::protocol::find_protocol("IWorkRef")
                    .expect("annotated protocol")
                    .runtime_name(),
                crate::lang::protocol::find_protocol("IClosed")
                    .expect("annotated protocol")
                    .runtime_name(),
            ]
        );
    }
}
