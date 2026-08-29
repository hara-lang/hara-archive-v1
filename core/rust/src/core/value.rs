#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionSite {
    pub namespace: Option<String>,
    pub resource: Option<String>,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ExceptionProvenance {
    pub created_at: Option<ExceptionSite>,
    pub throws: Vec<ExceptionSite>,
}

#[derive(Debug, Clone)]
pub struct ExceptionInfo {
    pub message: String,
    pub data: Box<Value>,
    pub cause: Option<Box<Value>>,
    pub provenance: Rc<RefCell<ExceptionProvenance>>,
}

fn default_exception_class(code: &Keyword) -> Option<Keyword> {
    if code.get_namespace() != Some("hara") {
        return None;
    }
    let class = match code.get_name() {
        "security" | "timeout" | "not-found" | "conflict" | "limit" | "syntax" | "io"
        | "database" | "dependency" | "serialization" | "argument" | "state" | "host" => {
            code.get_name()
        }
        "generic" => "internal",
        _ => return None,
    };
    Keyword::parse(&format!("ex.class/{class}")).ok()
}

fn normalize_exception_code(code: &Keyword) -> Result<Keyword, String> {
    if code.get_namespace().is_some() {
        return Ok(code.clone());
    }
    let canonical = Keyword::parse(&format!("hara/{}", code.get_name()))?;
    if default_exception_class(&canonical).is_some() {
        Ok(canonical)
    } else {
        Err("ex expects a registered standard keyword or namespaced keyword code".into())
    }
}

pub(crate) fn record_exception_throw(value: &Value, site: Option<ExceptionSite>) {
    let (Value::ExceptionInfo(exception), Some(site)) = (value, site) else {
        return;
    };
    let mut provenance = exception.provenance.borrow_mut();
    provenance.throws.push(site);
}

pub(crate) fn record_exception_creation(value: &Value, site: Option<ExceptionSite>) {
    let (Value::ExceptionInfo(exception), Some(site)) = (value, site) else {
        return;
    };
    let mut provenance = exception.provenance.borrow_mut();
    if provenance.created_at.is_none() {
        provenance.created_at = Some(site);
    }
}

pub(crate) fn exception_site_value(site: &ExceptionSite) -> Value {
    Value::Map(
        [
            (
                "namespace",
                site.namespace
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Nil),
            ),
            (
                "resource",
                site.resource
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Nil),
            ),
            ("line", Value::Number(site.line as i64)),
            ("column", Value::Number(site.column as i64)),
        ]
        .into_iter()
        .map(|(key, value)| (Value::Keyword(key.into()), value))
        .collect(),
    )
}

pub(crate) fn exception_provenance_value(exception: &ExceptionInfo) -> Value {
    let provenance = exception.provenance.borrow();
    Value::Map(
        [
            (
                Value::Keyword("ex/created-at".into()),
                provenance
                    .created_at
                    .as_ref()
                    .map(exception_site_value)
                    .unwrap_or(Value::Nil),
            ),
            (
                Value::Keyword("ex/throws".into()),
                Value::Vector(provenance.throws.iter().map(exception_site_value).collect()),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

#[derive(Debug, Clone)]
pub enum Value {
    Number(i64),
    Float(f64),
    BigInteger(BigInt),
    Character(char),
    Regex(String),
    Tagged(Box<PTaggedLiteral<Value>>),
    Bool(bool),
    String(String),
    Keyword(Keyword),
    Bytes(Vec<u8>),
    ByteBuffer(Rc<RefCell<Vec<u8>>>),
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<Vec<(String, Value)>>>),
    Promise(Promise),
    Atom(Box<RuntimeAtom>),
    Recur(Vec<Value>),
    Map(PMap<Value, Value>),
    OrderedMap(Box<POrderedMap<Value, Value>>),
    SortedMap(Box<PSortedMap<Value, Value>>),
    Trie(Box<PTrie<Value>>),
    Set(PSet<Value>),
    OrderedSet(Box<POrderedSet<Value>>),
    SortedSet(Box<PSortedSet<Value>>),
    List(PList<Value>),
    Cons(Box<PCons<Value>>),
    Deque(Box<PDeque<Value>>),
    Queue(Box<PQueue<Value>>),
    PriorityMap(Box<PPriorityMap<Value, Value>>),
    Symbol(Symbol),
    Pointer(PPointer),
    Function(Rc<Function>),
    Tuple(Box<PTuple<Value>>),
    Vector(PVector<Value>),
    MapEntry(Box<PMapEntry>),
    MutableCollection(Rc<RefCell<Option<MutableCollection>>>),
    Seq(Box<PSeq<Result<Value, String>>>),
    Iterator(Rc<RefCell<IteratorState>>),
    Var(KernelVar<Value>),
    Namespace(Rc<crate::kernel::Namespace<Value>>),
    Extension(ExtensionValue),
    StructType(Rc<StructType>),
    Struct(Rc<StructValue>),
    MutableType(Rc<MutableType>),
    Mutable(Rc<MutableValue>),
    Protocol(Rc<GuestProtocol>),
    NativeType(Rc<NativeType>),
    Schema(Rc<RuntimeSchema>),
    Coroutine(Rc<Coroutine>),
    Stream(Rc<RuntimeStream>),
    Result(Rc<ResultValue>),
    ExceptionInfo(Rc<ExceptionInfo>),
    Nil,
}

const UUID_TAG: &str = "uuid";

fn uuid_value_from_uuid(value: uuid::Uuid) -> Value {
    Value::Tagged(Box::new(PTaggedLiteral::new(
        Symbol::parse(UUID_TAG),
        Value::String(value.hyphenated().to_string()),
    )))
}

fn uuid_from_bytes(bytes: &[u8]) -> uuid::Uuid {
    let digest = md5::compute(bytes);
    let mut value = digest.0;
    value[6] = (value[6] & 0x0f) | 0x30;
    value[8] = (value[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(value)
}

fn uuid_from_parts(most: i64, least: i64) -> uuid::Uuid {
    let value = ((most as u64 as u128) << 64) | least as u64 as u128;
    uuid::Uuid::from_u128(value)
}

fn uuid_from_value(value: &Value) -> Result<uuid::Uuid, String> {
    match value {
        Value::String(value) => uuid::Uuid::parse_str(value)
            .map_err(|_| "Base/uuid expects a valid UUID string".into()),
        Value::Bytes(value) => Ok(uuid_from_bytes(value)),
        Value::ByteBuffer(value) => Ok(uuid_from_bytes(&value.borrow())),
        Value::Keyword(value) => Ok(uuid_from_parts(
            crate::lang::hash::java_string_hash(value.as_str()) as i64,
            crate::lang::hash::java_string_hash(value.get_name()) as i64,
        )),
        _ => Err("Base/uuid expects a string, bytes, or keyword".into()),
    }
}

fn random_uuid() -> uuid::Uuid {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .unwrap_or_else(|_| panic!("could not retrieve random bytes for uuid"));
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

pub(crate) fn uuid_value(values: &[Value]) -> Result<Value, String> {
    let value = match values {
        [] => random_uuid(),
        [value] => uuid_from_value(value)?,
        [Value::Number(most), Value::Number(least)] => uuid_from_parts(*most, *least),
        _ if values.len() == 2 => {
            return Err("Base/uuid expects two integer arguments".into())
        }
        _ => return Err("Base/uuid expects zero, one, or two arguments".into()),
    };
    Ok(uuid_value_from_uuid(value))
}

pub(crate) fn uuid_text_from_tagged(value: &PTaggedLiteral<Value>) -> Option<&str> {
    if value.tag().as_str() != UUID_TAG {
        return None;
    }
    let Value::String(text) = value.form() else {
        return None;
    };
    uuid::Uuid::parse_str(text)
        .ok()
        .filter(|uuid| uuid.hyphenated().to_string() == *text)
        .map(|_| text.as_str())
}

pub(crate) fn is_uuid_tagged(value: &PTaggedLiteral<Value>) -> bool {
    uuid_text_from_tagged(value).is_some()
}

#[derive(Debug, Clone)]
pub enum MutableCollection {
    Map(MutableMap<Value, Value>),
    OrderedMap(MutableOrderedMap<Value, Value>),
    SortedMap(MutableSortedMap<Value, Value>),
    Trie(MutableTrie<Value>),
    Set(MutableSet<Value>),
    OrderedSet(MutableOrderedSet<Value>),
    SortedSet(MutableSortedSet<Value>),
    List(MutableList<Value>),
    Queue(MutableQueue<Value>),
    Vector(MutableVector<Value>),
}

fn named_field_key(field: &str) -> Value {
    Value::Keyword(Keyword::from(field))
}

fn named_field_name(value: &Value) -> Option<&str> {
    match value {
        Value::String(name) => Some(name.as_str()),
        Value::Keyword(name) if name.get_namespace().is_none() => Some(name.get_name()),
        Value::Symbol(name) if name.get_namespace().is_none() => Some(name.get_name()),
        _ => None,
    }
}

impl StructValue {
    pub(crate) fn from_values(
        ty: Rc<StructType>,
        values: Vec<Value>,
        metadata: Option<Rc<Metadata>>,
    ) -> Result<Self, String> {
        if values.len() != ty.fields.len() {
            return Err(format!("{} expects {} arguments", ty.name, ty.fields.len()));
        }
        let values = ty
            .fields
            .iter()
            .zip(values)
            .fold(POrderedMap::new(), |values, (field, value)| {
                values.assoc_value(named_field_key(field), value)
            });
        Ok(Self {
            ty,
            values,
            metadata,
        })
    }

    pub(crate) fn get(&self, field: &str) -> Option<&Value> {
        self.values.get(&named_field_key(field))
    }

    pub(crate) fn ordered_values(&self) -> Vec<&Value> {
        self.ty
            .fields
            .iter()
            .filter_map(|field| self.get(field))
            .collect()
    }

    pub(crate) fn ordered_entries(&self) -> Vec<(Value, Value)> {
        self.ty
            .fields
            .iter()
            .filter_map(|field| {
                self.get(field)
                    .cloned()
                    .map(|value| (named_field_key(field), value))
            })
            .collect()
    }
}

impl MutableValue {
    pub(crate) fn from_values(
        ty: Rc<MutableType>,
        values: Vec<Value>,
        metadata: Option<Rc<Metadata>>,
    ) -> Result<Self, String> {
        if values.len() != ty.fields.len() {
            return Err(format!("{} expects {} arguments", ty.name, ty.fields.len()));
        }
        Ok(Self {
            ty,
            values: Rc::new(RefCell::new(values)),
            metadata,
        })
    }

    fn field_index(&self, field: &str) -> Option<usize> {
        self.ty
            .fields
            .iter()
            .position(|candidate| candidate == field)
    }

    pub(crate) fn get(&self, field: &str) -> Option<Value> {
        let index = self.field_index(field)?;
        self.values.borrow().get(index).cloned()
    }

    pub(crate) fn set(&self, field: &str, replacement: Value) -> Result<Value, String> {
        let index = self
            .field_index(field)
            .ok_or_else(|| format!("unknown mutable field: {field}"))?;
        self.values.borrow_mut()[index] = replacement.clone();
        Ok(replacement)
    }

    pub(crate) fn ordered_values(&self) -> Vec<Value> {
        self.values.borrow().clone()
    }

    pub(crate) fn ordered_entries(&self) -> Vec<(Value, Value)> {
        self.ty
            .fields
            .iter()
            .cloned()
            .zip(self.ordered_values())
            .map(|(field, value)| (named_field_key(&field), value))
            .collect()
    }

    fn same_identity(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.values, &other.values)
    }

    fn identity_address(&self) -> usize {
        Rc::as_ptr(&self.values) as usize
    }
}

#[derive(Clone)]
pub struct Function {
    params: Vec<String>,
    variadic: Option<String>,
    patterns: Vec<Form>,
    variadic_pattern: Option<Form>,
    body: Vec<Form>,
    captured: Rc<RefCell<HashMap<String, Value>>>,
    pub name: Option<String>,
    /// Namespace in which the function body was defined. Lazy aliases and
    /// qualified Vars are resolved against this namespace when invoked.
    namespace: Option<String>,
    native: Option<Rc<dyn Fn(Vec<Value>) -> Result<Value, String>>>,
    fiber_native: Option<Rc<dyn Fn(Vec<Value>, Cont) -> Step>>,
    /// Arity clauses for multi-arity `defn`/`fn` dispatchers; empty otherwise.
    clauses: Vec<Rc<Function>>,
    /// Runtime-neutral metadata attached through IObjType.
    metadata: Option<Rc<Metadata>>,
    /// Whether this function is a macro expander.
    is_macro: bool,
}

impl Function {
    /// Returns the symbol that identifies this callable's definition.
    /// Named source functions use their defining namespace as the origin;
    /// native callables may already carry a qualified display name.
    pub(crate) fn origin_symbol(&self) -> Option<Symbol> {
        let name = self.name.as_deref()?;
        if name.contains('/') {
            Some(Symbol::parse(name))
        } else {
            Some(Symbol::create(self.namespace.as_deref(), name))
        }
    }
}

#[derive(Clone)]
pub(crate) struct MultiMethod {
    dispatch: Rc<Function>,
    methods: Vec<(Value, Rc<Function>)>,
    default: Option<Rc<Function>>,
}

impl std::fmt::Debug for Function {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Function")
            .field("params", &self.params)
            .field("variadic", &self.variadic)
            .field("name", &self.name)
            .field("native", &self.native.is_some())
            .finish()
    }
}

/// State of a portable coroutine value.
pub enum CoroutineState {
    /// The body has not started yet; stores the body function.
    New(Value),
    /// Parked at a yield/await; stores the continuation that resumes the body.
    Suspended(Box<dyn FnOnce(Value) -> Step>),
    /// Currently executing on a fiber.
    Running,
    /// Completed, closed, or killed by an error.
    Dead,
}

impl std::fmt::Debug for CoroutineState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New(_) => formatter.debug_tuple("New").finish(),
            Self::Suspended(_) => formatter.debug_tuple("Suspended").finish(),
            Self::Running => formatter.write_str("Running"),
            Self::Dead => formatter.write_str("Dead"),
        }
    }
}

/// A re-entrant coroutine implemented with the fiber/CPS evaluator.
pub struct Coroutine {
    pub state: RefCell<CoroutineState>,
}

impl std::fmt::Debug for Coroutine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Coroutine")
            .field("state", &self.state.borrow())
            .finish()
    }
}

impl Coroutine {
    pub fn new(body: Value) -> Self {
        Self {
            state: RefCell::new(CoroutineState::New(body)),
        }
    }
}

pub struct RuntimeStream {
    source: RuntimeStreamSource,
    pending: Rc<Cell<bool>>,
    closed: Rc<Cell<bool>>,
}

enum RuntimeStreamSource {
    Coroutine {
        coroutine: Rc<Coroutine>,
        initial_arguments: RefCell<Option<Vec<Value>>>,
    },
    Guest {
        next: Rc<Function>,
        close: Option<Rc<Function>>,
    },
    Host {
        next: Rc<dyn Fn() -> Result<Promise, String>>,
        close: Rc<dyn Fn() -> Result<(), String>>,
    },
}

impl std::fmt::Debug for RuntimeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeStream")
            .field("closed", &self.closed.get())
            .finish()
    }
}

impl RuntimeStream {
    fn new(body: Value, initial_arguments: Vec<Value>) -> Self {
        Self {
            source: RuntimeStreamSource::Coroutine {
                coroutine: Rc::new(Coroutine::new(body)),
                initial_arguments: RefCell::new(Some(initial_arguments)),
            },
            pending: Rc::new(Cell::new(false)),
            closed: Rc::new(Cell::new(false)),
        }
    }
    fn host(
        next: Rc<dyn Fn() -> Result<Promise, String>>,
        close: Rc<dyn Fn() -> Result<(), String>>,
    ) -> Self {
        Self {
            source: RuntimeStreamSource::Host { next, close },
            pending: Rc::new(Cell::new(false)),
            closed: Rc::new(Cell::new(false)),
        }
    }
    fn guest(next: Rc<Function>, close: Option<Rc<Function>>) -> Self {
        Self {
            source: RuntimeStreamSource::Guest { next, close },
            pending: Rc::new(Cell::new(false)),
            closed: Rc::new(Cell::new(false)),
        }
    }
}

#[derive(Clone)]
pub struct RuntimeAtom {
    value: PAtom<Value>,
    watches: Rc<RefCell<Vec<(Value, Rc<Function>)>>>,
    watchable: bool,
}

impl RuntimeAtom {
    pub(crate) fn new(value: Value, watchable: bool) -> Self {
        Self {
            value: PAtom::new(value),
            watches: Rc::new(RefCell::new(Vec::new())),
            watchable,
        }
    }
    fn same_identity(&self, other: &Self) -> bool {
        self.value.same_identity(&other.value)
    }
    fn identity_address(&self) -> usize {
        self.value.identity_address()
    }
    pub(crate) fn deref_value(&self) -> Value {
        self.value.deref_value()
    }
    fn reset(&self, new_value: Value) -> Result<Value, String> {
        let old_value = self.value.deref_value();
        let result = self.value.reset(new_value.clone())?;
        self.notify(old_value, new_value)?;
        Ok(result)
    }
    fn compare_and_set(&self, old: &Value, new_value: Value) -> Result<bool, String> {
        let prior = self.value.deref_value();
        let changed = self.value.compare_and_set(old, new_value.clone())?;
        if changed {
            self.notify(prior, new_value)?;
        }
        Ok(changed)
    }
    fn add_watch(&self, key: Value, function: Rc<Function>) -> Result<(), String> {
        if !self.watchable {
            return Err("watch-add expects a standard atom".into());
        }
        let mut watches = self.watches.borrow_mut();
        watches.retain(|(candidate, _)| candidate != &key);
        watches.push((key, function));
        Ok(())
    }
    fn remove_watch(&self, key: &Value) -> Result<(), String> {
        if !self.watchable {
            return Err("watch-remove expects a standard atom".into());
        }
        self.watches
            .borrow_mut()
            .retain(|(candidate, _)| candidate != key);
        Ok(())
    }
    fn watch_entries(&self) -> Result<Vec<Value>, String> {
        if !self.watchable {
            return Err("watch-list expects a standard atom".into());
        }
        self.watches
            .borrow()
            .iter()
            .map(|(key, function)| {
                vector_literal(vec![key.clone(), Value::Function(function.clone())])
            })
            .collect()
    }
    fn notify(&self, old_value: Value, new_value: Value) -> Result<(), String> {
        let watches = self.watches.borrow().clone();
        for (key, function) in watches {
            call_function(
                &function,
                vec![
                    key,
                    Value::Atom(Box::new(self.clone())),
                    old_value.clone(),
                    new_value.clone(),
                ],
            )?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for RuntimeAtom {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeAtom")
            .finish_non_exhaustive()
    }
}

fn function_definition_namespace() -> Option<String> {
    namespace_registry()
        .ok()
        .map(|registry| registry.current().name().as_str().to_owned())
}

/// Builds a fixed-arity native callable for embedding-owned namespaces.
pub fn native_function(
    name: &str,
    arity: usize,
    callback: impl Fn(Vec<Value>) -> Result<Value, String> + 'static,
) -> Value {
    let function = Rc::new(Function {
        params: (0..arity).map(|index| format!("arg{index}")).collect(),
        variadic: None,
        patterns: Vec::new(),
        variadic_pattern: None,
        body: Vec::new(),
        captured: Rc::new(RefCell::new(HashMap::new())),
        name: Some(name.into()),
        namespace: function_definition_namespace(),
        native: Some(Rc::new(callback)),
        fiber_native: None,
        clauses: Vec::new(),
        metadata: None,
        is_macro: false,
    });
    debug_assert!(function.origin_symbol().is_some());
    Value::Function(function)
}

/// A native function wrapper with an exact fixed parameter list and an
/// optional rest marker: `params.len()` reflects the fixed arity so the
/// multi-arity `select_clause` boundary can dispatch on it, unlike
/// [`native_variadic_function`] whose parameter list is empty. Used by
/// the bytecode VM for variadic closures (issue #223).
pub(crate) fn native_fixed_variadic_function(
    name: &str,
    fixed_arity: usize,
    callback: impl Fn(Vec<Value>) -> Result<Value, String> + 'static,
) -> Value {
    let function = Rc::new(Function {
        params: (0..fixed_arity)
            .map(|index| format!("arg{index}"))
            .collect(),
        variadic: Some("rest".into()),
        patterns: Vec::new(),
        variadic_pattern: None,
        body: Vec::new(),
        captured: Rc::new(RefCell::new(HashMap::new())),
        name: Some(name.into()),
        namespace: function_definition_namespace(),
        native: Some(Rc::new(callback)),
        fiber_native: None,
        clauses: Vec::new(),
        metadata: None,
        is_macro: false,
    });
    debug_assert!(function.origin_symbol().is_some());
    Value::Function(function)
}

pub(crate) fn native_variadic_function(
    name: &str,
    callback: impl Fn(Vec<Value>) -> Result<Value, String> + 'static,
) -> Value {
    let function = Rc::new(Function {
        params: Vec::new(),
        variadic: Some("arguments".into()),
        patterns: Vec::new(),
        variadic_pattern: None,
        body: Vec::new(),
        captured: Rc::new(RefCell::new(HashMap::new())),
        name: Some(name.into()),
        namespace: function_definition_namespace(),
        native: Some(Rc::new(callback)),
        fiber_native: None,
        clauses: Vec::new(),
        metadata: None,
        is_macro: false,
    });
    debug_assert!(function.origin_symbol().is_some());
    Value::Function(function)
}

pub(crate) fn native_fiber_function(
    name: &str,
    fixed_arity: usize,
    variadic: bool,
    callback: impl Fn(Vec<Value>) -> Result<Value, String> + 'static,
    fiber_callback: impl Fn(Vec<Value>, Cont) -> Step + 'static,
) -> Value {
    native_fiber_function_with_arity_error(
        name,
        fixed_arity,
        variadic,
        callback,
        fiber_callback,
        |expectation, _received| format!("function expects {expectation} arguments"),
    )
}

pub(crate) fn native_protocol_fiber_function(
    name: &str,
    protocol: &str,
    method: &str,
    fixed_arity: usize,
    variadic: bool,
    callback: impl Fn(Vec<Value>) -> Result<Value, String> + 'static,
    fiber_callback: impl Fn(Vec<Value>, Cont) -> Step + 'static,
) -> Value {
    let display_name = format!("{protocol}/{method}");
    native_fiber_function_with_arity_error(
        name,
        fixed_arity,
        variadic,
        callback,
        fiber_callback,
        move |expectation, received| {
            format!(
                "protocol/arity: {display_name} expects {expectation} arguments, received {received}"
            )
        },
    )
}

fn native_fiber_function_with_arity_error(
    name: &str,
    fixed_arity: usize,
    variadic: bool,
    callback: impl Fn(Vec<Value>) -> Result<Value, String> + 'static,
    fiber_callback: impl Fn(Vec<Value>, Cont) -> Step + 'static,
    arity_error: impl Fn(String, usize) -> String + 'static,
) -> Value {
    let fiber_callback = move |arguments: Vec<Value>, continuation: Cont| {
        let valid = if variadic {
            arguments.len() >= fixed_arity
        } else {
            arguments.len() == fixed_arity
        };
        if !valid {
            let expectation = if variadic {
                format!("at least {fixed_arity}")
            } else {
                fixed_arity.to_string()
            };
            return continuation(Err(arity_error(expectation, arguments.len())));
        }
        fiber_callback(arguments, continuation)
    };
    let function = Rc::new(Function {
        params: (0..fixed_arity)
            .map(|index| format!("arg{index}"))
            .collect(),
        variadic: variadic.then(|| "rest".into()),
        patterns: Vec::new(),
        variadic_pattern: None,
        body: Vec::new(),
        captured: Rc::new(RefCell::new(HashMap::new())),
        name: Some(name.into()),
        namespace: function_definition_namespace(),
        native: Some(Rc::new(callback)),
        fiber_native: Some(Rc::new(fiber_callback)),
        clauses: Vec::new(),
        metadata: None,
        is_macro: false,
    });
    debug_assert!(function.origin_symbol().is_some());
    Value::Function(function)
}

pub(crate) fn exception_function_values() -> Vec<(&'static str, Value)> {
    vec![
        (
            "ex",
            native_variadic_function("ex", |arguments| {
                if arguments.len() < 2 || arguments.len() % 2 != 0 {
                    return Err("ex expects a code, attributes map, and key/value pairs".into());
                }
                let Value::Keyword(input_code) = &arguments[0] else {
                    return Err(
                        "ex expects a registered standard keyword or namespaced keyword code"
                            .into(),
                    );
                };
                let code = normalize_exception_code(input_code)?;
                let mut attributes = arguments[1].clone();
                for pair in arguments[2..].chunks_exact(2) {
                    attributes = map_assoc_value(&attributes, pair[0].clone(), pair[1].clone())?;
                }
                let Some(entries) = map_entries(&attributes) else {
                    return Err("ex expects an attributes map".into());
                };
                let lookup = |name: &str| {
                    entries.iter().find_map(|(key, value)| {
                        matches!(key, Value::Keyword(key_name) if key_name.as_str() == name)
                            .then_some(value)
                    })
                };
                let message = match lookup("ex/message") {
                    Some(Value::String(message)) => message.clone(),
                    Some(_) => return Err(":ex/message must be a string".into()),
                    None => format!(":{code}"),
                };
                if lookup("ex/code").is_some() {
                    return Err("ex attributes must not contain :ex/code; pass the code as the first argument".into());
                }
                if let Some(class) = lookup("ex/class") {
                    match class {
                        Value::Keyword(class) if class.get_namespace().is_some() => {
                            if let Some(expected) = default_exception_class(&code) {
                                if class != &expected {
                                    return Err(":ex/class conflicts with the registered class for :ex/code".into());
                                }
                            }
                        }
                        _ => return Err(":ex/class must be a namespaced keyword".into()),
                    }
                }
                let cause = match lookup("ex/cause") {
                    Some(cause @ Value::ExceptionInfo(_)) => Some(cause.clone()),
                    Some(_) => return Err(":ex/cause must be an Exception".into()),
                    None => None,
                };
                if let Some(context) = lookup("ex/context") {
                    if map_entries(context).is_none() {
                        return Err(":ex/context must be a map".into());
                    }
                }
                let mut data = map_assoc_value(
                    &attributes,
                    Value::Keyword("ex/code".into()),
                    Value::Keyword(code.clone()),
                )?;
                if lookup("ex/class").is_none() {
                    if let Some(class) = default_exception_class(&code) {
                        data = map_assoc_value(
                            &data,
                            Value::Keyword("ex/class".into()),
                            Value::Keyword(class),
                        )?;
                    }
                }
                if let Some(cause) = &cause {
                    data =
                        map_assoc_value(&data, Value::Keyword("ex/cause".into()), cause.clone())?;
                }
                let value = Value::ExceptionInfo(Rc::new(ExceptionInfo {
                    message,
                    cause: cause.map(Box::new),
                    data: Box::new(data),
                    provenance: Rc::new(RefCell::new(ExceptionProvenance {
                        created_at: None,
                        throws: Vec::new(),
                    })),
                }));
                record_exception_creation(&value, current_exception_site());
                Ok(value)
            }),
        ),
        (
            "ex-info",
            native_variadic_function("ex-info", |arguments| {
                if !(2..=3).contains(&arguments.len()) {
                    return Err("ex-info expects a message, data map, and optional cause".into());
                }
                let Value::String(message) = &arguments[0] else {
                    return Err("ex-info expects a string message".into());
                };
                if map_entries(&arguments[1]).is_none() {
                    return Err("ex-info expects a data map".into());
                }
                let cause = match arguments.get(2) {
                    Some(cause @ Value::ExceptionInfo(_)) => Some(Box::new(cause.clone())),
                    Some(_) => return Err("ex-info expects an Exception cause".into()),
                    None => None,
                };
                let value = Value::ExceptionInfo(Rc::new(ExceptionInfo {
                    message: message.clone(),
                    data: Box::new(arguments[1].clone()),
                    cause,
                    provenance: Rc::new(RefCell::new(ExceptionProvenance {
                        created_at: None,
                        throws: Vec::new(),
                    })),
                }));
                record_exception_creation(&value, current_exception_site());
                Ok(value)
            }),
        ),
        (
            "ex-data",
            native_function("ex-data", 1, |arguments| match &arguments[0] {
                Value::ExceptionInfo(value) => Ok((*value.data).clone()),
                _ => Ok(Value::Nil),
            }),
        ),
        (
            "ex-message",
            native_function("ex-message", 1, |arguments| match &arguments[0] {
                Value::ExceptionInfo(value) => Ok(Value::String(value.message.clone())),
                Value::String(value) => Ok(Value::String(value.clone())),
                value => Ok(Value::String(value.display())),
            }),
        ),
        (
            "ex-cause",
            native_function("ex-cause", 1, |arguments| match &arguments[0] {
                Value::ExceptionInfo(value) => {
                    Ok(value.cause.as_deref().cloned().unwrap_or(Value::Nil))
                }
                _ => Err("ex-cause expects an Exception".into()),
            }),
        ),
        (
            "ex-provenance",
            native_function("ex-provenance", 1, |arguments| match &arguments[0] {
                Value::ExceptionInfo(value) => Ok(exception_provenance_value(value)),
                _ => Err("ex-provenance expects an Exception".into()),
            }),
        ),
        (
            "ex-class",
            native_function("ex-class", 1, |arguments| match &arguments[0] {
                Value::ExceptionInfo(value) => {
                    let Some(entries) = map_entries(&value.data) else {
                        return Err("Exception data must be a map".into());
                    };
                    match entries.iter().find_map(|(key, value)| {
                        matches!(key, Value::Keyword(name) if name.as_str() == "ex/class")
                            .then_some(value)
                    }) {
                        None => Ok(Value::Nil),
                        Some(Value::Keyword(class)) if class.get_namespace().is_some() => {
                            Ok(Value::Keyword(class.clone()))
                        }
                        Some(_) => Err(":ex/class must be a namespaced keyword".into()),
                    }
                }
                _ => Err("ex-class expects an Exception".into()),
            }),
        ),
        (
            "ex-native-type",
            native_function("ex-native-type", 1, |arguments| match &arguments[0] {
                Value::ExceptionInfo(_) => Ok(Value::Nil),
                _ => Err("ex-native-type expects an Exception".into()),
            }),
        ),
    ]
}

pub(crate) fn direct_function_value(name: &str) -> Option<Value> {
    match name {
        "pair" => Some(native_function("pair", 2, |arguments| {
            Ok(Value::MapEntry(Box::new(PMapEntry::new(
                arguments[0].clone(),
                arguments[1].clone(),
            ))))
        })),
        "disj" => Some(native_variadic_function("disj", |arguments| {
            let (collection, values) = arguments
                .split_first()
                .ok_or_else(|| "disj expects a collection".to_string())?;
            let mut output = collection.clone();
            for value in values {
                if matches!(output, Value::Nil) {
                    break;
                }
                output = crate::core::protocol_intrinsic_call(
                    "std.protocol.idissoc.IDissoc/dissoc",
                    &[output, value.clone()],
                )?;
            }
            Ok(output)
        })),
        "quot" => Some(native_function("quot", 2, |arguments| {
            numeric::numeric_quotient(&arguments[0], &arguments[1])
        })),
        "rem" => Some(native_function("rem", 2, |arguments| {
            apply_binary_intrinsic(IntrinsicOp::Remainder, &arguments[0], &arguments[1])
        })),
        "mod" => Some(native_variadic_function("mod", |arguments| {
            if arguments.len() != 2 {
                return Err("mod expects arguments".into());
            }
            numeric::numeric_binary(ArithmeticOp::Modulo, &arguments[0], &arguments[1])
        })),
        _ => IntrinsicOp::from_symbol(name).map(|primitive| {
            native_variadic_function(name, move |arguments| {
                apply_intrinsic(primitive, &arguments)
            })
        }),
    }
}

/// Creates a callable exported by a `std.native.*` namespace.
///
/// Native type methods must terminate in their Rust implementation. They must
/// not resolve their public HAL facade name and re-enter `eval`, because doing
/// so makes alias precedence part of native invocation and permits facade →
/// native → facade recursion.
pub fn native_type_function_value(native_type: &str, method: &str) -> Result<Value, String> {
    let declaration = NATIVE_DECLARATIONS
        .iter()
        .find(|declaration| declaration.name == native_type)
        .ok_or_else(|| {
            format!(
                "missing annotated native declaration: std.native.{native_type}/{method}"
            )
        })?;
    if !declaration.method(method) {
        return Err(format!(
            "unknown annotated native method: std.native.{native_type}/{method}"
        ));
    }
    (declaration.provider)(native_type, method)
}

fn native_display_name(native_type: &str, method: &str) -> String {
    format!("std.native.{native_type}/{method}")
}

fn native_base_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_base_values(&method, &arguments)
    }))
}

fn native_schema_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_schema_values(&method, &arguments)
    }))
}

fn native_string_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let operation = format!("str/{method}");
    Ok(native_variadic_function(&display_name, move |arguments| {
        string_operation(&operation, arguments)
    }))
}

fn native_bytes_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_bytes_operation(&method, arguments)
    }))
}

fn native_iter_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_iter_operation(&method, arguments)
    }))
}

fn native_maths_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        math_values(&method, arguments)
    }))
}

fn native_num_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        if arguments.len() != 1 {
            return Err(format!("{method} expects one value"));
        }
        number_conversion_value(&method, arguments.into_iter().next().unwrap())
    }))
}

fn native_bits_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        bit_values(&method, &arguments)
    }))
}

fn native_kernel_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        require_native_capability("Kernel", &method, "kernel")?;
        kernel_provider(&method)?(method.clone(), arguments)
    }))
}

fn native_sandbox_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let operation = format!("sandbox-{method}");
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        require_native_capability("Sandbox", &method, "sandbox")?;
        kernel_provider(&operation)?(operation.clone(), arguments)
    }))
}

fn native_crypto_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_crypto::operation(&method, arguments)
    }))
}

fn native_document_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        document_operation(&method, arguments)
    }))
}

fn native_package_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        require_native_capability("Package", &method, "kernel")?;
        native_package_values(&method, arguments, &mut HashMap::new())
    }))
}

fn native_os_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let native_type = native_type.to_owned();
    let method = method.to_owned();
    let operation = native_display_name(&native_type, &method);
    Ok(native_variadic_function(&display_name, move |arguments| {
        if native_type == "Process" {
            require_native_capability("Process", &method, "native-runtime")?;
        }
        os_values(&operation, arguments)
    }))
}

fn native_file_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    let operation = native_display_name(native_type, &method);
    Ok(native_variadic_function(&display_name, move |arguments| {
        require_native_capability("File", &method, "file")?;
        file_values(&operation, arguments)
    }))
}

fn native_socket_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    let operation = native_display_name(native_type, &method);
    Ok(native_variadic_function(&display_name, move |arguments| {
        require_native_capability("Socket", &method, "network")?;
        socket_values(&operation, arguments)
    }))
}

fn native_promise_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_promise_values(&method, arguments)
    }))
}

fn native_coroutine_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    match method {
        "create" => Ok(native_fiber_function(
            &display_name,
            1,
            false,
            native_coroutine_create,
            native_coroutine_create_fiber,
        )),
        "yield" => Ok(native_fiber_function(
            &display_name,
            1,
            false,
            native_coroutine_yield,
            native_coroutine_yield_fiber,
        )),
        "await" => Ok(native_fiber_function(
            &display_name,
            1,
            false,
            native_coroutine_await,
            native_coroutine_await_fiber,
        )),
        _ => Err(format!("unknown std.native.Coroutine operation: {method}")),
    }
}

fn native_stream_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_stream_values(&method, arguments)
    }))
}

fn native_mutable_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let operation = native_display_name(native_type, method);
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_mutable_values(&operation, arguments)
    }))
}

fn native_runtime_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_runtime_values(&method, arguments, &mut HashMap::new())
    }))
}

fn native_printer_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_printer_values(&method, arguments)
    }))
}

fn native_edn_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_edn_values(&method, arguments)
    }))
}

fn native_json_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        match (method.as_str(), arguments.as_slice()) {
            ("read", [Value::String(source)]) => crate::json::read(source),
            ("write", [value]) => crate::json::write(value).map(Value::String),
            ("pretty", [value, options]) if map_entries(options).is_some() => {
                crate::json::write_pretty(value).map(Value::String)
            }
            ("pretty", [_, _]) => Err("json/pretty expects an options map".into()),
            ("read", _) => Err("json/read expects a string".into()),
            ("write", _) => Err("json/write expects one value".into()),
            ("pretty", _) => Err("json/pretty expects a value and options map".into()),
            _ => Err(format!("unknown std.native.Json operation: {method}")),
        }
    }))
}

fn native_host_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        if !native_capability_granted("host-call") {
            return Ok(native_capability_denied_promise(
                "Host",
                &method,
                "host-call",
            ));
        }
        native_host_values(&method, arguments)
    }))
}

fn native_test_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_test_values(&method, arguments)
    }))
}

fn native_regexp_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_regex_values(&method, arguments)
    }))
}

fn native_result_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_result_values(&method, arguments)
    }))
}

fn native_exception_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let method = method.to_owned();
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_exception_values(&method, arguments)
    }))
}

fn native_algo_provider(native_type: &str, method: &str) -> Result<Value, String> {
    let display_name = native_display_name(native_type, method);
    let operation = native_display_name(native_type, method);
    Ok(native_variadic_function(&display_name, move |arguments| {
        native_algo_values(&operation, arguments)
    }))
}

fn native_work_provider(_native_type: &str, method: &str) -> Result<Value, String> {
    crate::work::guest::values()
        .into_iter()
        .find(|(name, _)| *name == method)
        .map(|(_, value)| value)
        .ok_or_else(|| format!("unknown std.native.Work operation: {method}"))
}

fn native_coroutine_create(arguments: Vec<Value>) -> Result<Value, String> {
    match arguments.as_slice() {
        [Value::Function(function)] => Ok(Value::Coroutine(Rc::new(Coroutine::new(
            Value::Function(function.clone()),
        )))),
        _ => Err("Coroutine/create expects one function".into()),
    }
}

fn native_coroutine_create_fiber(arguments: Vec<Value>, k: Cont) -> Step {
    match arguments.as_slice() {
        [Value::Function(function)] => k(Ok(Value::Coroutine(Rc::new(Coroutine::new(
            Value::Function(function.clone()),
        ))))),
        _ => k(Err("Coroutine/create expects one function".into())),
    }
}

fn native_coroutine_yield(_arguments: Vec<Value>) -> Result<Value, String> {
    Err("Coroutine/yield requires the fiber evaluator".into())
}

fn native_coroutine_yield_fiber(arguments: Vec<Value>, k: Cont) -> Step {
    match arguments.as_slice() {
        [value] => Step::Yield(value.clone(), Box::new(move |resumed| k(Ok(resumed)))),
        _ => k(Err("Coroutine/yield expects one value".into())),
    }
}

fn native_coroutine_await(_arguments: Vec<Value>) -> Result<Value, String> {
    Err("Coroutine/await requires the fiber evaluator".into())
}

fn native_coroutine_await_fiber(arguments: Vec<Value>, k: Cont) -> Step {
    match arguments.as_slice() {
        [Value::Var(reference)] => k(Ok(reference.deref_value())),
        [Value::Promise(promise)] => match promise.state() {
            PromiseState::Fulfilled(value) => k(Ok(value)),
            PromiseState::Rejected(error) => k(Err(crate::core::promise_rejection_error(error))),
            PromiseState::Pending => Step::Wait(
                promise.clone(),
                Box::new(move |state| match state {
                    PromiseState::Fulfilled(value) => k(Ok(value)),
                    PromiseState::Rejected(error) => {
                        k(Err(crate::core::promise_rejection_error(error)))
                    }
                    PromiseState::Pending => k(Err("Coroutine/await resumed pending".into())),
                }),
            ),
        },
        _ => k(Err("Coroutine/await expects a derefable (e.g. a promise)".into())),
    }
}

fn native_edn_values(method: &str, arguments: Vec<Value>) -> Result<Value, String> {
    match (method, arguments.as_slice()) {
        ("read", [Value::String(source)]) => read_edn(source),
        ("read-forms", [Value::String(path)]) => {
            if !(path.ends_with(".hal") || path.ends_with(".hrl")) {
                return Err("read-forms expects a .hal or .hrl path".into());
            }
            let promise = file_provider("read-forms")?
                .read(path)
                .map_err(|error| file_error("read-forms", error))?;
            let bytes = match promise.wait_state() {
                PromiseState::Fulfilled(Value::Bytes(bytes)) => bytes,
                PromiseState::Fulfilled(Value::ByteBuffer(bytes)) => bytes.borrow().clone(),
                PromiseState::Fulfilled(value) => {
                    return Err(format!(
                        "read-forms expected file bytes, got {}",
                        value.display()
                    ));
                }
                PromiseState::Rejected(error) => return Err(error.message()),
                PromiseState::Pending => return Err("read-forms file read is still pending".into()),
            };
            let source = String::from_utf8(bytes)
                .map_err(|_| format!("read-forms source is not UTF-8: {path}"))?;
            let forms = crate::kernel::parse_forms(&source)
                .map_err(|error| format!("read-forms failed: {error}"))?;
            Ok(Value::Vector(PVector::from_iter(
                forms
                    .iter()
                    .map(form_to_value)
                    .collect::<Result<Vec<_>, _>>()?,
            )))
        }
        ("write", [value]) => Ok(Value::String(value.display())),
        ("pretty", [value, options]) if map_entries(options).is_some() => {
            Ok(Value::String(value.display()))
        }
        ("pretty", [_, _]) => Err("edn/pretty expects an options map".into()),
        ("read", _) => Err("edn/read expects one string".into()),
        ("read-forms", _) => Err("read-forms expects a path string".into()),
        ("write", _) => Err("std.native.Edn/write expects one value".into()),
        ("pretty", _) => Err("std.native.Edn/pretty expects a value and options map".into()),
        _ => Err(format!("unknown std.native.Edn operation: {method}")),
    }
}

fn native_printer_values(method: &str, arguments: Vec<Value>) -> Result<Value, String> {
    match method {
        "capture" => {
            let [callable] = arguments.as_slice() else {
                return Err("Printer/capture expects one callable".into());
            };
            PRINTER_CAPTURES.with(|captures| captures.borrow_mut().push(String::new()));
            let result = call_value(callable.clone(), Vec::new());
            let output = PRINTER_CAPTURES.with(|captures| {
                captures
                    .borrow_mut()
                    .pop()
                    .expect("Printer/capture stack must contain the active capture")
            });
            result.map(|_| Value::String(output))
        }
        "p" | "println" => {
            let text = arguments
                .iter()
                .map(|value| match (method, value) {
                    ("p", Value::Nil) => String::new(),
                    ("p", Value::String(text)) => text.clone(),
                    ("p", Value::Character(character)) => character.to_string(),
                    (_, Value::String(text)) => text.clone(),
                    _ => value.display(),
                })
                .collect::<Vec<_>>()
                .join(if method == "println" { " " } else { "" });
            let output = if method == "println" {
                format!("{text}\n")
            } else {
                text
            };
            printer_write(&output)?;
            Ok(Value::Nil)
        }
        _ => Err(format!("unknown std.native.Printer operation: {method}")),
    }
}

fn native_promise_values(method: &str, arguments: Vec<Value>) -> Result<Value, String> {
    match (method, arguments.as_slice()) {
        ("from", [value]) => Ok(Value::Promise(promise_from(value.clone()))),
        ("all", [values]) => Ok(Value::Promise(promise_all(iterator_values(
            values.clone(),
        )?))),
        ("run", [Value::Function(function)]) => {
            let function = function.clone();
            let context = crate::core::NativeCallbackContext::capture();
            let task = Rc::new(move || context.with(|| call_function(&function, Vec::new())));
            Ok(Value::Promise(promise_provider().run(task)))
        }
        ("new", [Value::Function(function)]) => {
            let promise = Promise::new();
            let resolving = promise.clone();
            let resolve = native_function("promise-resolve", 1, move |mut values| {
                let value = values.remove(0);
                settle_promise_result(&resolving, Ok(value.clone()));
                Ok(value)
            });
            let rejecting = promise.clone();
            let reject = native_function("promise-reject", 1, move |mut values| {
                let value = values.remove(0);
                rejecting.reject_value(value.clone());
                Ok(value)
            });
            if let Err(error) = call_function(function, vec![resolve, reject]) {
                promise.reject(error);
            }
            Ok(Value::Promise(promise))
        }
        ("delay", [millis, Value::Function(function)]) => {
            let millis = value_u64_integer(millis, "promise/delay")
                .map_err(|_| "promise/delay expects non-negative milliseconds".to_string())?;
            let function = function.clone();
            let context = crate::core::NativeCallbackContext::capture();
            let task = Rc::new(move || context.with(|| call_function(&function, Vec::new())));
            Ok(Value::Promise(
                promise_provider().delay(std::time::Duration::from_millis(millis), task),
            ))
        }
        ("run", _) => Err("promise/run expects one function".into()),
        ("new", [_]) => Err("promise/new expects a function".into()),
        ("new", _) => Err("promise/new expects one function".into()),
        ("from", _) => Err("promise/from expects one value".into()),
        ("all", _) => Err("promise/all expects one collection".into()),
        ("delay", _) => Err("promise/delay expects milliseconds and a function".into()),
        _ => Err(format!("unknown std.native.Promise operation: {method}")),
    }
}

fn native_iter_operation(method: &str, arguments: Vec<Value>) -> Result<Value, String> {
    let unary = |label: &str| {
        arguments
            .first()
            .cloned()
            .filter(|_| arguments.len() == 1)
            .ok_or_else(|| format!("Iter/{label} expects one argument"))
    };
    let binary = |label: &str| {
        if arguments.len() == 2 {
            Ok((arguments[0].clone(), arguments[1].clone()))
        } else {
            Err(format!("Iter/{label} expects two arguments"))
        }
    };
    match method {
        "seq" => iterator_seq(unary(method)?),
        "iter" => make_iterator(unary(method)?),
        "iter-finite?" => Ok(Value::Bool(iterator_is_finite(&unary(method)?))),
        "iter-materialize" => Ok(Value::Vector(iterator_to_vec(unary(method)?)?.into())),
        "iter-next?" => iterator_has_next(&unary(method)?),
        "iter-next" => iterator_next(&unary(method)?),
        "iter-close" => iterator_close(&unary(method)?),
        "iter-concat" => iterator_concat(arguments),
        "iter-interleave" => iterator_interleave(arguments),
        "iter-zip" => iterator_zip(arguments),
        "iter-map" => {
            let (function, source) = binary(method)?;
            iterator_map(function, source)
        }
        "iter-filter" => {
            let (function, source) = binary(method)?;
            iterator_filter(function, source)
        }
        "iter-take-while" => {
            let (function, source) = binary(method)?;
            iterator_take_while(function, source)
        }
        "iter-drop-while" => {
            let (function, source) = binary(method)?;
            iterator_drop_while(function, source)
        }
        "iter-mapcat" => {
            let (function, source) = binary(method)?;
            iterator_mapcat(function, source)
        }
        "iter-keep" => {
            let (function, source) = binary(method)?;
            iterator_keep(function, source)
        }
        "iter-interpose" => {
            let (separator, source) = binary(method)?;
            iterator_interpose(separator, source)
        }
        "iter-every?" | "iter-any?" => {
            let (predicate, source) = binary(method)?;
            let iterator = make_iterator(source)?;
            let expect_every = method == "iter-every?";
            let result = (|| {
                while let Some(value) = iterator_try_next(&iterator)? {
                    let matched = call_value(predicate.clone(), vec![value])?.truthy();
                    if matched != expect_every {
                        return Ok(Value::Bool(!expect_every));
                    }
                }
                Ok(Value::Bool(expect_every))
            })();
            let close = iterator_close(&iterator);
            close?;
            result
        }
        "iter-take" | "iter-drop" => {
            let (amount, source) = binary(method)?;
            let amount = value_index(&amount)?;
            if method == "iter-take" {
                iterator_take(source, amount)
            } else {
                iterator_drop(source, amount)
            }
        }
        "iter-cycle" => iterator_cycle(unary(method)?),
        "iter-partition-pair" => iterator_partition(unary(method)?, 2, false),
        "iter-partition" | "iter-partition-all" => {
            let (amount, source) = binary(method)?;
            iterator_partition(source, value_index(&amount)?, method.ends_with("-all"))
        }
        "iter-range" => {
            let bounds = arguments
                .iter()
                .map(|value| {
                    numeric::to_i64_exact(value).map_err(|_| {
                        "iter-range bounds must fit signed 64-bit integers".to_string()
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (start, end) = match bounds.as_slice() {
                [end] => (0, *end),
                [start, end] => (*start, *end),
                _ => return Err("iter-range expects an end or start and end".into()),
            };
            Ok(iterator_from_values(
                (start..end).map(Value::Number).collect(),
            ))
        }
        "iter-constantly" => Ok(iterator_constant(unary(method)?)),
        "iter-repeatedly" => Ok(iterator_repeated(unary(method)?)),
        "iter-iterate" => {
            let (function, seed) = binary(method)?;
            Ok(iterator_iterate(function, seed))
        }
        _ => Err(format!("unknown std.native.Iter operation: {method}")),
    }
}

fn native_bytes_operation(method: &str, arguments: Vec<Value>) -> Result<Value, String> {
    match (method, arguments.as_slice()) {
        ("new", values) => native_bytes_new(values),
        ("count", [value]) => byte_count(value),
        ("get", [value, index]) => byte_get(value, index, None),
        ("get", [value, index, default]) => byte_get(value, index, Some(default.clone())),
        ("set", [value, index, item]) => byte_set(value, index, item),
        ("copy", [value]) => byte_copy(value),
        ("slice", [value, start]) => {
            let end = byte_count(value)?;
            byte_slice(value, start, &end)
        }
        ("slice", [value, start, end]) => byte_slice(value, start, end),
        ("u8" | "s8", [Value::Number(number)]) if (-128..=255).contains(number) => {
            let raw = (*number as i8) as u8;
            Ok(Value::Number(if method == "u8" {
                raw as i64
            } else {
                raw as i8 as i64
            }))
        }
        ("u8" | "s8", [_]) => Err(format!(
            "bytes/{method} expects a value in the range -128..255"
        )),
        _ => Err(format!(
            "std.native.Bytes/{method} received unsupported arguments"
        )),
    }
}

fn native_bytes_new(values: &[Value]) -> Result<Value, String> {
    let values = values
        .iter()
        .map(|value| byte_input(value, "bytes"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::ByteBuffer(Rc::new(RefCell::new(values))))
}

/// Structural evaluator arms that are ordinary callable values.  Rust keeps
/// the implementations in `eval`, but exposes the names through real
/// `std.foundation` Vars just as the JVM runtime does.  Syntax and namespace
/// mutation forms deliberately remain structural and are never interned here.
pub(crate) fn syntax_symbol(name: &str) -> bool {
    const SYNTAX_FORMS: &[&str] = &[
        ".",
        "binding",
        "comment",
        "declare",
        "def",
        "defmacro",
        "defmethod",
        "defmulti",
        "defmutable",
        "defn",
        "defn-",
        "defprotocol",
        "defstruct",
        "do",
        "extend-type",
        "field",
        "fn",
        "if",
        "let",
        "letfn",
        "loop",
        "ns",
        "ns+",
        "quote",
        "read-forms",
        "recur",
        "require",
        "set!",
        "syntax-quote",
        "throw",
        "try",
        "var",
    ];
    SYNTAX_FORMS.contains(&name)
}

pub fn with_macros<R>(
    macros: Rc<RefCell<HashMap<(String, String), Rc<Function>>>>,
    operation: impl FnOnce() -> R,
) -> R {
    ACTIVE_MACROS.with(|active| {
        let previous = active.replace(Some(macros));
        let result = operation();
        active.replace(previous);
        result
    })
}

fn register_macro(namespace: &str, name: &str, function: Rc<Function>) -> Result<(), String> {
    ACTIVE_MACROS.with(|active| {
        active
            .try_borrow_mut()
            .map_err(|_| "macro registry is busy".into())
            .and_then(|opt| {
                if let Some(macros) = opt.as_ref() {
                    macros
                        .try_borrow_mut()
                        .map_err(|_| "macro registry is busy".into())
                        .map(|mut macros| {
                            macros.insert((namespace.into(), name.into()), function);
                        })
                } else {
                    Err("macro registry is unavailable".into())
                }
            })
    })
}

fn resolve_macro_in(namespace: &str, name: &str) -> Option<Rc<Function>> {
    ACTIVE_MACROS.with(|active| {
        active.borrow().as_ref().and_then(|macros| {
            macros
                .borrow()
                .get(&(namespace.into(), name.into()))
                .cloned()
        })
    })
}

pub(crate) fn resolve_macro(name: &str) -> Option<Rc<Function>> {
    if let Some((namespace, local)) = name.split_once('/') {
        let resolved = namespace_registry().ok().and_then(|registry| {
            let current = registry.current();
            if namespace == "-" {
                return Some(current.name().as_str().to_owned());
            }
            current
                .aliases()
                .into_iter()
                .find(|(alias, _)| alias.as_str() == namespace)
                .map(|(_, target)| target.name().as_str().to_owned())
        });
        return resolve_macro_in(resolved.as_deref().unwrap_or(namespace), local);
    }
    let current = namespace_registry()
        .map(|registry| registry.current().name().as_str().to_owned())
        .ok()?;
    resolve_macro_in(&current, name).or_else(|| resolve_macro_in("std.foundation", name))
}

fn gensym(prefix: &str) -> String {
    let index = GENSYM_COUNTER.with(|counter| {
        let value = counter.get();
        counter.set(value + 1);
        value
    });
    format!("{prefix}{index}")
}

pub(crate) fn form_to_value(form: &Form) -> Result<Value, String> {
    literal_value(form)
}

fn metadata_value_to_form(value: &MetadataValue) -> Form {
    match value {
        MetadataValue::Nil => Form::Nil,
        MetadataValue::Boolean(value) => Form::Bool(*value),
        MetadataValue::Number(value) => Form::Number(*value),
        MetadataValue::Float(value) => Form::Float(*value),
        MetadataValue::BigInteger(value) => Form::BigInteger(value.clone()),
        MetadataValue::Character(value) => Form::Character(*value),
        MetadataValue::Regex(value) => Form::Regex(value.clone()),
        MetadataValue::Tagged(tag, value) => {
            Form::Tagged(tag.clone(), Box::new(metadata_value_to_form(value)))
        }
        MetadataValue::String(value) => Form::String(value.clone()),
        MetadataValue::Keyword(value) => Form::Keyword(value.as_str().into()),
        MetadataValue::Symbol(value) => Form::Symbol(value.as_str().into()),
        MetadataValue::Vector(values) => {
            Form::Vector(values.iter().map(metadata_value_to_form).collect())
        }
        MetadataValue::List(values) => {
            Form::List(values.iter().map(metadata_value_to_form).collect())
        }
        MetadataValue::Set(values) => {
            Form::Set(values.iter().map(metadata_value_to_form).collect())
        }
        MetadataValue::Map(values) => Form::Map(
            values
                .iter()
                .map(|(key, value)| (metadata_value_to_form(key), metadata_value_to_form(value)))
                .collect(),
        ),
    }
}

pub(crate) fn value_to_form(value: &Value) -> Result<Form, String> {
    let form = match value {
        Value::Nil => Ok(Form::Nil),
        Value::Bool(value) => Ok(Form::Bool(*value)),
        Value::Number(value) => Ok(Form::Number(*value)),
        Value::Float(value) => Ok(Form::Float(*value)),
        Value::BigInteger(value) => Ok(Form::BigInteger(value.clone())),
        Value::Character(value) => Ok(Form::Character(*value)),
        Value::Regex(value) => Ok(Form::Regex(value.clone())),
        Value::String(value) => Ok(Form::String(value.clone())),
        Value::Keyword(value) => Ok(Form::Keyword(value.as_str().into())),
        Value::Symbol(value) => Ok(Form::Symbol(value.as_str().into())),
        Value::Tagged(value) => Ok(Form::Tagged(
            value.tag().get_name().into(),
            Box::new(value_to_form(value.form())?),
        )),
        Value::Pointer(value) => Ok(Form::Tagged(
            "ptr".into(),
            Box::new(value_to_form(&Value::Map(value.descriptor()))?),
        )),
        Value::List(values) => Ok(Form::List(
            values
                .iter()
                .map(|v| value_to_form(v))
                .collect::<Result<_, _>>()?,
        )),
        Value::Queue(values) => Ok(Form::List(
            values
                .iter()
                .map(|v| value_to_form(v))
                .collect::<Result<_, _>>()?,
        )),
        Value::Deque(values) => Ok(Form::List(
            values
                .iter()
                .map(|v| value_to_form(v))
                .collect::<Result<_, _>>()?,
        )),
        Value::Cons(values) => Ok(Form::List(
            values
                .iter()
                .map(|v| value_to_form(&v))
                .collect::<Result<_, _>>()?,
        )),
        Value::Vector(values) => Ok(Form::Vector(
            values
                .iter()
                .map(|v| value_to_form(v))
                .collect::<Result<_, _>>()?,
        )),
        Value::Tuple(values) => Ok(Form::Vector(
            values
                .iter()
                .map(|v| value_to_form(v))
                .collect::<Result<_, _>>()?,
        )),
        Value::MapEntry(entry) => Ok(Form::Vector(
            entry
                .iter()
                .map(value_to_form)
                .collect::<Result<_, _>>()?,
        )),
        Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_) => Ok(Form::Set(
            set_items(value)
                .unwrap()
                .iter()
                .copied()
                .map(value_to_form)
                .collect::<Result<_, _>>()?,
        )),
        Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_) => Ok(Form::Map(
            map_entries(value)
                .unwrap()
                .into_iter()
                .map(|(key, value)| -> Result<(Form, Form), String> {
                    Ok((value_to_form(&key)?, value_to_form(&value)?))
                })
                .collect::<Result<_, _>>()?,
        )),
        value => Err(format!("cannot use {} as code", portable_type_name(value))),
    }?;
    Ok(match value_metadata(value) {
        Some(metadata) => Form::Metadata(
            Box::new(metadata_value_to_form(&MetadataValue::Map(
                metadata.entries().to_vec(),
            ))),
            Box::new(form),
        ),
        None => form,
    })
}

/// Evaluates code retained as an immutable bytecode constant. This is used
/// for namespace-level declarations whose effect mutates runtime protocol or
/// multimethod registries and therefore cannot be reduced to ordinary stack
/// instructions.
pub(crate) fn eval_bytecode_declaration(
    expected_operator: &str,
    value: &Value,
) -> Result<Value, String> {
    let form = value_to_form(value)?;
    if !matches!(
        &form,
        Form::List(items)
            if matches!(items.first(), Some(Form::Symbol(operator)) if operator == expected_operator)
    ) {
        return Err(format!(
            "{expected_operator} instruction contains the wrong declaration"
        ));
    }
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    if direct_native_execution() {
        return eval_direct_native_declaration(expected_operator, &form);
    }
    let mut env = HashMap::new();
    if let Ok(registry) = namespace_registry() {
        env.extend(
            registry
                .current()
                .mappings()
                .into_iter()
                .map(|(name, var)| (name.as_str().to_owned(), Value::Var(var))),
        );
        refresh_namespace_environment(&registry, &mut env);
    }
    let result = eval(&form, &mut env).map_err(|error| {
        let namespace = namespace_registry()
            .map(|registry| registry.current().name().as_str().to_owned())
            .unwrap_or_else(|_| "<unavailable>".into());
        format!("{expected_operator} in {namespace}: {error}")
    })?;
    if let Ok(registry) = namespace_registry() {
        save_namespace_environment(&registry, &mut env);
    }
    Ok(result)
}

pub(crate) fn bytecode_dynamic_bind(name: &str, value: Value) -> Result<(), String> {
    let registry = namespace_registry()?;
    let var = registry
        .resolve(&crate::lang::data::Symbol::parse(name))
        .ok_or_else(|| format!("binding expects a Var: {name}"))?;
    if !var.is_dynamic() {
        return Err(format!("binding expects a dynamic Var: {name}"));
    }
    var.bind(value);
    Ok(())
}

pub(crate) fn bytecode_dynamic_unbind(name: &str) -> Result<(), String> {
    let registry = namespace_registry()?;
    let var = registry
        .resolve(&crate::lang::data::Symbol::parse(name))
        .ok_or_else(|| format!("binding expects a Var: {name}"))?;
    var.unbind().map(|_| ())
}

fn macro_environment() -> Result<Value, String> {
    let namespace = namespace_registry()?.current().name().as_str().to_owned();
    let entries = vec![
        (
            Value::Keyword(Keyword::from("ns")),
            Value::Symbol(Symbol::from(namespace)),
        ),
        (
            Value::Keyword(Keyword::from("locals")),
            Value::OrderedMap(Box::new(POrderedMap::new())),
        ),
        (
            Value::Keyword(Keyword::from("aliases")),
            Value::OrderedMap(Box::new(POrderedMap::new())),
        ),
    ];
    Ok(Value::OrderedMap(Box::new(POrderedMap::from_iter(entries))))
}

fn macroexpand_call(
    name: &str,
    invocation: &[Form],
    _env: &mut HashMap<String, Value>,
) -> Result<Option<Form>, String> {
    let function = match resolve_macro(name) {
        Some(function) => function,
        None => return Ok(None),
    };
    let mut arguments = Vec::with_capacity(invocation.len() + 1);
    arguments.push(form_to_value(&Form::List(invocation.to_vec()))?);
    arguments.push(macro_environment()?);
    for form in &invocation[1..] {
        arguments.push(form_to_value(form)?);
    }
    let expansion = call_function(&function, arguments)?;
    let expansion = value_to_form(&expansion)?;
    #[cfg(feature = "evaluation-journal")]
    evaluation_journal_macro(name, &Form::List(invocation.to_vec()), &expansion);
    Ok(Some(expansion))
}

pub(crate) fn form_without_metadata(mut form: &Form) -> &Form {
    while let Form::Metadata(_, value) = form {
        form = value.as_ref();
    }
    form
}

fn macro_clause_with_implicit_params(clause: &Form) -> Result<Form, String> {
    match form_without_metadata(clause) {
        Form::List(parts) if !parts.is_empty() => {
            let params = match form_without_metadata(&parts[0]) {
                Form::Vector(params) => params,
                _ => return Err("macro arity must start with a parameter vector".into()),
            };
            let mut implicit = vec![Form::Symbol("&form".into()), Form::Symbol("&env".into())];
            implicit.extend_from_slice(params);
            let mut new_parts = vec![Form::Vector(implicit)];
            new_parts.extend_from_slice(&parts[1..]);
            Ok(Form::List(new_parts))
        }
        _ => Err("macro arity must be a list".into()),
    }
}

fn macroexpand_once(form: &Form, env: &mut HashMap<String, Value>) -> Result<Form, String> {
    match form {
        Form::List(values) if !values.is_empty() => {
            if let Form::Symbol(name) = &values[0] {
                if let Some(expanded) = macroexpand_call(name, values, env)? {
                    return Ok(expanded);
                }
            }
            Ok(form.clone())
        }
        _ => Ok(form.clone()),
    }
}

pub(crate) fn vm_macroexpand(form: &Form) -> Result<Form, String> {
    let mut current = form.clone();
    let mut env = HashMap::new();
    for _ in 0..1000 {
        let expanded = macroexpand_once(&current, &mut env)?;
        if expanded == current {
            return Ok(current);
        }
        current = expanded;
    }
    Err("macro expansion exceeded 1000 steps".into())
}

thread_local! {
    static TRACE_ENABLED: Cell<bool> = const { Cell::new(false) };
    static TRACE_STACK: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    #[cfg(feature = "evaluation-journal")]
    static EVALUATION_JOURNAL: RefCell<Option<crate::journal::JournalCollector>> = const { RefCell::new(None) };
    #[cfg(feature = "evaluation-journal")]
    static EVALUATION_JOURNAL_STACK: RefCell<Vec<crate::journal::OperationId>> = const { RefCell::new(Vec::new()) };
    static ACTIVE_MACROS: RefCell<Option<Rc<RefCell<HashMap<(String, String), Rc<Function>>>>>> =
        const { RefCell::new(None) };
    static GENSYM_COUNTER: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn trace_stack_snapshot() -> Vec<String> {
    TRACE_STACK.with(|stack| stack.borrow().clone())
}

pub(crate) fn with_trace_stack<R>(trace: &[String], operation: impl FnOnce() -> R) -> R {
    let previous = TRACE_STACK.with(|stack| {
        std::mem::replace(&mut *stack.borrow_mut(), trace.to_vec())
    });
    let result = operation();
    TRACE_STACK.with(|stack| {
        *stack.borrow_mut() = previous;
    });
    result
}

pub(crate) fn trace_frame_label(
    name: String,
    namespace: Option<String>,
    site: Option<ExceptionSite>,
) -> String {
    let label = namespace
        .map(|namespace| format!("{namespace}/{name}"))
        .unwrap_or(name);
    match site {
        Some(site) if site.line > 0 => format!("{label} @ {}:{}", site.line, site.column),
        _ => label,
    }
}

#[cfg(feature = "evaluation-journal")]
fn journal_preview(value: &Value) -> crate::journal::ValuePreview {
    EVALUATION_JOURNAL.with(|active| {
        active
            .borrow()
            .as_ref()
            .expect("evaluation journal must be active")
            .preview_value(portable_type_name(value), value.display())
    })
}

#[cfg(feature = "evaluation-journal")]
fn evaluation_journal_enter(
    function: &Function,
    arguments: &[Value],
) -> Option<crate::journal::OperationId> {
    if EVALUATION_JOURNAL.with(|active| active.borrow().is_none()) {
        return None;
    }
    let values = arguments.iter().map(journal_preview).collect();
    let parent_operation = EVALUATION_JOURNAL_STACK.with(|stack| stack.borrow().last().copied());
    let depth = EVALUATION_JOURNAL_STACK.with(|stack| stack.borrow().len());
    EVALUATION_JOURNAL.with(|active| {
        let mut active = active.borrow_mut();
        let collector = active.as_mut()?;
        let operation = collector.next_operation_id();
        let mut event =
            crate::journal::JournalEvent::new(crate::journal::JournalEventKind::OperationEnter);
        event.operation = Some(operation);
        event.parent_operation = parent_operation;
        event.depth = depth;
        event.function = Some(
            function
                .name
                .clone()
                .unwrap_or_else(|| "<anonymous>".into()),
        );
        event.values = values;
        collector.record(event);
        EVALUATION_JOURNAL_STACK.with(|stack| stack.borrow_mut().push(operation));
        Some(operation)
    })
}

#[cfg(feature = "evaluation-journal")]
fn evaluation_journal_exit(
    operation: Option<crate::journal::OperationId>,
    function: &Function,
    result: Option<&Value>,
) {
    let Some(operation) = operation else { return };
    let value = result.map(journal_preview);
    EVALUATION_JOURNAL.with(|active| {
        if let Some(collector) = active.borrow_mut().as_mut() {
            let mut event = crate::journal::JournalEvent::new(
                crate::journal::JournalEventKind::OperationReturn,
            );
            event.operation = Some(operation);
            event.function = Some(
                function
                    .name
                    .clone()
                    .unwrap_or_else(|| "<anonymous>".into()),
            );
            event.values = value.into_iter().collect();
            collector.record(event);
        }
    });
    EVALUATION_JOURNAL_STACK.with(|stack| {
        let popped = stack.borrow_mut().pop();
        debug_assert_eq!(popped, Some(operation));
    });
}

#[cfg(feature = "evaluation-journal")]
fn evaluation_journal_macro(name: &str, source: &Form, expansion: &Form) {
    let parent_operation = EVALUATION_JOURNAL_STACK.with(|stack| stack.borrow().last().copied());
    let depth = EVALUATION_JOURNAL_STACK.with(|stack| stack.borrow().len());
    EVALUATION_JOURNAL.with(|active| {
        if let Some(collector) = active.borrow_mut().as_mut() {
            let mut event =
                crate::journal::JournalEvent::new(crate::journal::JournalEventKind::MacroExpand);
            event.parent_operation = parent_operation;
            event.depth = depth;
            event.function = Some(name.into());
            event.values = vec![
                collector.preview_value("form", source.to_string()),
                collector.preview_value("form", expansion.to_string()),
            ];
            collector.record(event);
        }
    });
}

struct StackTraceGuard {
    previous: bool,
}

impl StackTraceGuard {
    fn enable() -> Self {
        let previous = TRACE_ENABLED.with(|enabled| {
            let previous = enabled.get();
            enabled.set(true);
            previous
        });
        TRACE_STACK.with(|stack| stack.borrow_mut().clear());
        Self { previous }
    }
}

/// Runs an execution boundary with Hara stack collection enabled.
///
/// Stack collection belongs to callable invocation, not to a second tree
/// evaluator. Fiber, bytecode, and other execution targets can share this
/// boundary while retaining the same opt-in error contract.
pub(crate) fn with_stack_trace<R>(operation: impl FnOnce() -> R) -> R {
    let _guard = StackTraceGuard::enable();
    operation()
}

impl Drop for StackTraceGuard {
    fn drop(&mut self) {
        TRACE_STACK.with(|stack| stack.borrow_mut().clear());
        TRACE_ENABLED.with(|enabled| enabled.set(self.previous));
    }
}

fn tracing_enabled() -> bool {
    TRACE_ENABLED.with(Cell::get)
}

pub(crate) fn append_trace(error: String) -> String {
    if !tracing_enabled() {
        return error;
    }
    let frames = TRACE_STACK.with(|stack| stack.borrow().iter().rev().cloned().collect::<Vec<_>>());
    if frames.is_empty() {
        return error;
    }
    if error.contains("\n[hara stack]") {
        return error;
    }
    format!(
        "{error}\n[hara stack]\n{}",
        frames
            .iter()
            .map(|frame| format!("  at {frame}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[derive(Debug, Clone)]
enum IteratorGenerator {
    Seq(PSeq<Result<Value, String>>),
    Constant(Value),
    Repeated(Value),
    Iterate(Value, Value),
    Take(Value, usize),
    Drop(Value, usize),
    Cycle(Value, Vec<Value>, usize, bool),
    TakeWhile(Value, Value),
    DropWhile(Value, Value, bool),
    Map(Value, Value, bool),
    Filter(Value, Value),
    Mapcat(Value, Value, Option<Value>),
    Keep(Value, Value),
    Prepend(Option<Value>, Value),
    Concat(Vec<Value>, usize),
    Zip(Vec<Value>),
    Interleave(Vec<Value>, usize),
    Interpose(Value, Value, bool, Option<Value>),
    Partition(Value, usize, bool),
}

#[derive(Debug, Clone)]
pub struct IteratorState {
    values: Vec<Value>,
    index: usize,
    closed: bool,
    cycle: bool,
    lookahead: Option<Value>,
    generator: Option<IteratorGenerator>,
}

fn close_iterator_source(value: &Value) {
    if let Value::Iterator(iterator) = value {
        if let Ok(mut state) = iterator.try_borrow_mut() {
            state.close();
        }
    }
}

impl IteratorState {
    fn new(values: Vec<Value>) -> Self {
        Self {
            values,
            index: 0,
            closed: false,
            cycle: false,
            lookahead: None,
            generator: None,
        }
    }
    fn generated(generator: IteratorGenerator) -> Self {
        Self {
            values: Vec::new(),
            index: 0,
            closed: false,
            cycle: false,
            lookahead: None,
            generator: Some(generator),
        }
    }
    pub(crate) fn is_finite(&self) -> bool {
        if self.closed || self.generator.is_none() {
            return true;
        }
        match self.generator.as_ref().unwrap() {
            IteratorGenerator::Seq(_) => false,
            IteratorGenerator::Constant(_)
            | IteratorGenerator::Repeated(_)
            | IteratorGenerator::Iterate(_, _)
            | IteratorGenerator::Cycle(_, _, _, _) => false,
            IteratorGenerator::Take(_, _) => true,
            IteratorGenerator::Drop(source, _)
            | IteratorGenerator::TakeWhile(_, source)
            | IteratorGenerator::DropWhile(_, source, _)
            | IteratorGenerator::Map(_, source, _)
            | IteratorGenerator::Filter(_, source)
            | IteratorGenerator::Keep(_, source)
            | IteratorGenerator::Prepend(_, source)
            | IteratorGenerator::Interpose(source, _, _, _)
            | IteratorGenerator::Partition(source, _, _) => value_iterator_is_finite(source),
            IteratorGenerator::Mapcat(_, _, _) => false,
            IteratorGenerator::Concat(sources, _) | IteratorGenerator::Interleave(sources, _) => {
                sources.iter().all(value_iterator_is_finite)
            }
            IteratorGenerator::Zip(sources) => sources.iter().any(value_iterator_is_finite),
        }
    }
    fn has_next(&mut self) -> Result<bool, String> {
        if self.lookahead.is_some() {
            return Ok(true);
        }
        match self.pull_next()? {
            Some(value) => {
                self.lookahead = Some(value);
                Ok(true)
            }
            None => Ok(false),
        }
    }
    fn try_next(&mut self) -> Result<Option<Value>, String> {
        if let Some(value) = self.lookahead.take() {
            return Ok(Some(value));
        }
        self.pull_next()
    }
    fn pull_next(&mut self) -> Result<Option<Value>, String> {
        if self.closed {
            return Ok(None);
        }
        if let Some(generator) = &mut self.generator {
            return match generator {
                IteratorGenerator::Seq(sequence) => match sequence.peek_first() {
                    None => {
                        self.closed = true;
                        Ok(None)
                    }
                    Some(result) => {
                        *sequence = sequence.pop_first();
                        result.map(Some)
                    }
                },
                IteratorGenerator::Constant(value) => Ok(Some(value.clone())),
                IteratorGenerator::Repeated(function) => {
                    call_value(function.clone(), Vec::new()).map(Some)
                }
                IteratorGenerator::Iterate(function, current) => {
                    let output = current.clone();
                    *current = call_value(function.clone(), vec![current.clone()])?;
                    Ok(Some(output))
                }
                IteratorGenerator::Take(source, remaining) => {
                    if *remaining == 0 {
                        close_iterator_source(source);
                        self.closed = true;
                        Ok(None)
                    } else {
                        *remaining -= 1;
                        let value = iterator_try_next(source)?;
                        if value.is_none() {
                            close_iterator_source(source);
                            self.closed = true;
                        }
                        Ok(value)
                    }
                }
                IteratorGenerator::Drop(source, remaining) => {
                    while *remaining > 0 {
                        if iterator_try_next(source)?.is_none() {
                            close_iterator_source(source);
                            self.closed = true;
                            return Ok(None);
                        }
                        *remaining -= 1;
                    }
                    let value = iterator_try_next(source)?;
                    if value.is_none() {
                        close_iterator_source(source);
                        self.closed = true;
                    }
                    Ok(value)
                }
                IteratorGenerator::Cycle(source, cache, index, exhausted) => {
                    if *index < cache.len() {
                        let value = cache[*index].clone();
                        *index += 1;
                        Ok(Some(value))
                    } else if *exhausted {
                        if cache.is_empty() {
                            self.closed = true;
                            Ok(None)
                        } else {
                            *index = 1;
                            Ok(Some(cache[0].clone()))
                        }
                    } else {
                        match iterator_try_next(source)? {
                            Some(value) => {
                                cache.push(value.clone());
                                *index += 1;
                                Ok(Some(value))
                            }
                            None => {
                                close_iterator_source(source);
                                *exhausted = true;
                                if cache.is_empty() {
                                    self.closed = true;
                                    Ok(None)
                                } else {
                                    *index = 1;
                                    Ok(Some(cache[0].clone()))
                                }
                            }
                        }
                    }
                }
                IteratorGenerator::TakeWhile(function, source) => {
                    let Some(value) = iterator_try_next(source)? else {
                        close_iterator_source(source);
                        self.closed = true;
                        return Ok(None);
                    };
                    if call_value(function.clone(), vec![value.clone()])?.truthy() {
                        Ok(Some(value))
                    } else {
                        close_iterator_source(source);
                        self.closed = true;
                        Ok(None)
                    }
                }
                IteratorGenerator::DropWhile(function, source, started) => loop {
                    let Some(value) = iterator_try_next(source)? else {
                        close_iterator_source(source);
                        self.closed = true;
                        break Ok(None);
                    };
                    if *started || !call_value(function.clone(), vec![value.clone()])?.truthy() {
                        *started = true;
                        break Ok(Some(value));
                    }
                },
                IteratorGenerator::Map(function, source, spread) => {
                    let Some(value) = iterator_try_next(source)? else {
                        close_iterator_source(source);
                        self.closed = true;
                        return Ok(None);
                    };
                    match value {
                        value if !*spread => call_value(function.clone(), vec![value]),
                        Value::Tuple(values) => {
                            call_value(function.clone(), values.iter().cloned().collect())
                        }
                        Value::Vector(values) => {
                            call_value(function.clone(), values.iter().cloned().collect())
                        }
                        value => call_value(function.clone(), vec![value]),
                    }
                    .map(Some)
                }
                IteratorGenerator::Filter(function, source) => loop {
                    let Some(value) = iterator_try_next(source)? else {
                        close_iterator_source(source);
                        self.closed = true;
                        break Ok(None);
                    };
                    if call_value(function.clone(), vec![value.clone()])?.truthy() {
                        break Ok(Some(value));
                    }
                },
                IteratorGenerator::Mapcat(function, source, pending) => loop {
                    if let Some(iterator) = pending {
                        match iterator_try_next(iterator)? {
                            Some(value) => break Ok(Some(value)),
                            None => {
                                close_iterator_source(iterator);
                                *pending = None;
                            }
                        }
                    }
                    let Some(value) = iterator_try_next(source)? else {
                        close_iterator_source(source);
                        self.closed = true;
                        break Ok(None);
                    };
                    *pending = Some(make_iterator(call_value(function.clone(), vec![value])?)?);
                },
                IteratorGenerator::Keep(function, source) => loop {
                    let Some(value) = iterator_try_next(source)? else {
                        close_iterator_source(source);
                        self.closed = true;
                        break Ok(None);
                    };
                    let mapped = call_value(function.clone(), vec![value])?;
                    if !matches!(mapped, Value::Nil) {
                        break Ok(Some(mapped));
                    }
                },
                IteratorGenerator::Prepend(head, source) => {
                    if let Some(value) = head.take() {
                        Ok(Some(value))
                    } else {
                        let value = iterator_try_next(source)?;
                        if value.is_none() {
                            close_iterator_source(source);
                            self.closed = true;
                        }
                        Ok(value)
                    }
                }
                IteratorGenerator::Concat(sources, index) => {
                    while *index < sources.len() {
                        match iterator_try_next(&sources[*index])? {
                            Some(value) => return Ok(Some(value)),
                            None => {
                                close_iterator_source(&sources[*index]);
                                *index += 1;
                            }
                        }
                    }
                    self.closed = true;
                    Ok(None)
                }
                IteratorGenerator::Zip(sources) => {
                    for source in sources.iter() {
                        if !matches!(iterator_has_next(source)?, Value::Bool(true)) {
                            for source in sources.iter() {
                                close_iterator_source(source);
                            }
                            self.closed = true;
                            return Ok(None);
                        }
                    }
                    let mut values = Vec::new();
                    for source in sources.iter() {
                        let Some(value) = iterator_try_next(source)? else {
                            for source in sources.iter() {
                                close_iterator_source(source);
                            }
                            self.closed = true;
                            return Ok(None);
                        };
                        values.push(value);
                    }
                    Ok(Some(Value::Vector(values.into())))
                }
                IteratorGenerator::Interleave(sources, index) => {
                    if sources.is_empty() {
                        self.closed = true;
                        return Ok(None);
                    }
                    if *index == 0 {
                        for source in sources.iter() {
                            if !matches!(iterator_has_next(source)?, Value::Bool(true)) {
                                for source in sources.iter() {
                                    close_iterator_source(source);
                                }
                                self.closed = true;
                                return Ok(None);
                            }
                        }
                    }
                    let source = &sources[*index];
                    let Some(value) = iterator_try_next(source)? else {
                        for source in sources.iter() {
                            close_iterator_source(source);
                        }
                        self.closed = true;
                        return Ok(None);
                    };
                    *index = (*index + 1) % sources.len();
                    Ok(Some(value))
                }
                IteratorGenerator::Interpose(source, separator, first, pending) => {
                    if let Some(value) = pending.take() {
                        return Ok(Some(value));
                    }
                    match iterator_try_next(source)? {
                        None => {
                            close_iterator_source(source);
                            self.closed = true;
                            Ok(None)
                        }
                        Some(value) if *first => {
                            *first = false;
                            Ok(Some(value))
                        }
                        Some(value) => {
                            *pending = Some(value);
                            Ok(Some(separator.clone()))
                        }
                    }
                }
                IteratorGenerator::Partition(source, amount, all) => {
                    let mut values = Vec::new();
                    for _ in 0..*amount {
                        match iterator_try_next(source)? {
                            Some(value) => values.push(value),
                            None => {
                                close_iterator_source(source);
                                self.closed = true;
                                if values.is_empty() || !*all {
                                    return Ok(None);
                                }
                                break;
                            }
                        }
                    }
                    if values.is_empty() {
                        self.closed = true;
                        Ok(None)
                    } else {
                        Ok(Some(Value::Vector(values.into())))
                    }
                }
            };
        }
        if self.values.is_empty() {
            self.closed = true;
            return Ok(None);
        }
        if self.cycle && self.index >= self.values.len() {
            self.index = 0;
        }
        if self.index >= self.values.len() {
            self.closed = true;
            return Ok(None);
        }
        let value = self.values[self.index].clone();
        self.index += 1;
        Ok(Some(value))
    }
    fn close(&mut self) {
        if self.closed {
            self.lookahead = None;
            return;
        }
        self.closed = true;
        self.lookahead = None;
        if let Some(generator) = &self.generator {
            match generator {
                IteratorGenerator::Constant(_)
                | IteratorGenerator::Repeated(_)
                | IteratorGenerator::Iterate(_, _)
                | IteratorGenerator::Seq(_) => {}
                IteratorGenerator::Take(source, _)
                | IteratorGenerator::Drop(source, _)
                | IteratorGenerator::Cycle(source, _, _, _)
                | IteratorGenerator::TakeWhile(_, source)
                | IteratorGenerator::DropWhile(_, source, _)
                | IteratorGenerator::Map(_, source, _)
                | IteratorGenerator::Filter(_, source)
                | IteratorGenerator::Keep(_, source)
                | IteratorGenerator::Prepend(_, source)
                | IteratorGenerator::Interpose(source, _, _, _)
                | IteratorGenerator::Partition(source, _, _) => close_iterator_source(source),
                IteratorGenerator::Mapcat(_, source, pending) => {
                    close_iterator_source(source);
                    if let Some(pending) = pending {
                        close_iterator_source(pending);
                    }
                }
                IteratorGenerator::Concat(sources, _)
                | IteratorGenerator::Zip(sources)
                | IteratorGenerator::Interleave(sources, _) => {
                    for source in sources {
                        close_iterator_source(source);
                    }
                }
            }
        }
    }
}

fn value_iterator_is_finite(value: &Value) -> bool {
    match value {
        Value::Iterator(iterator) => iterator.borrow().is_finite(),
        Value::Seq(_) => false,
        _ => true,
    }
}

#[inline(never)]
fn sequential_equality(left: &Value, right: &Value) -> Option<bool> {
    fn items(value: &Value) -> Option<Vec<Value>> {
        match value {
            Value::Seq(values) => values.iter().collect::<Result<Vec<_>, _>>().ok(),
            Value::List(values) => Some(values.iter().cloned().collect()),
            Value::Cons(values) => Some(values.iter().collect()),
            Value::Queue(values) => Some(values.iter().cloned().collect()),
            Value::Deque(values) => Some(values.iter().cloned().collect()),
            Value::Tuple(values) => Some(values.iter().cloned().collect()),
            Value::Vector(values) => Some(values.iter().cloned().collect()),
            _ => None,
        }
    }
    Some(items(left)? == items(right)?)
}

/// Returns cloned entries for every map-like runtime representation.
/// Embedding hosts should use this instead of depending on a concrete
/// persistent-map implementation.
pub fn map_entries(value: &Value) -> Option<Vec<(Value, Value)>> {
    match value {
        Value::Map(values) => Some(values.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        Value::OrderedMap(values) => {
            Some(values.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        Value::SortedMap(values) => {
            Some(values.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        Value::PriorityMap(values) => Some(values.iter().collect()),
        Value::Trie(values) => Some(
            values
                .entries()
                .into_iter()
                .map(|(k, v)| (Value::String(k), v.clone()))
                .collect(),
        ),
        _ => None,
    }
}

fn pointer_from_descriptor(descriptor: Value) -> Result<Value, String> {
    let entries =
        map_entries(&descriptor).ok_or_else(|| "pointer expects one descriptor map".to_string())?;
    let context_key = Value::Keyword(Keyword::from("context"));
    let mut context = None;
    let mut fields = Vec::new();
    for (key, value) in entries {
        if key == context_key {
            if context.is_some() {
                return Err("pointer descriptor contains duplicate :context".into());
            }
            context = match value {
                Value::Keyword(context) => Some(context),
                _ => return Err("pointer :context must be a keyword".into()),
            };
        } else {
            if !matches!(key, Value::Keyword(_)) {
                return Err("pointer descriptor fields must use keyword keys".into());
            }
            fields.push((key, value));
        }
    }
    let context = context.ok_or_else(|| "pointer descriptor requires :context".to_string())?;
    Ok(Value::Pointer(PPointer::new(
        context,
        fields.into_iter().collect(),
    )))
}

/// Returns whether a value may leave the evaluator session as immutable HAL data.
///
/// Session transfer is deliberately narrower than displayability. Functions,
/// Vars, mutable containers, iterators, asynchronous values, and native handles
/// all have printable representations, but those representations must not turn
/// a live session-owned value into an apparently successful transfer.
pub(crate) fn session_transferable(value: &Value) -> bool {
    match value {
        Value::Number(_)
        | Value::Float(_)
        | Value::BigInteger(_)
        | Value::Character(_)
        | Value::Regex(_)
        | Value::Tagged(_)
        | Value::Bool(_)
        | Value::String(_)
        | Value::Keyword(_)
        | Value::Bytes(_)
        | Value::Symbol(_)
        | Value::Nil => true,
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => map_entries(value).is_some_and(|entries| {
            entries
                .iter()
                .all(|(key, value)| session_transferable(key) && session_transferable(value))
        }),
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => set_items(value)
            .is_some_and(|values| values.iter().all(|value| session_transferable(value))),
        Value::List(values) => values.iter().all(session_transferable),
        Value::Cons(values) => values.iter().all(|value| session_transferable(&value)),
        Value::Queue(values) => values.iter().all(session_transferable),
        Value::Deque(values) => values.iter().all(session_transferable),
        Value::Tuple(values) => values.iter().all(session_transferable),
        Value::Vector(values) => values.iter().all(session_transferable),
        Value::MapEntry(entry) => {
            session_transferable(entry.key()) && session_transferable(entry.value())
        }
        Value::Struct(value) => value.ordered_values().into_iter().all(session_transferable),
        Value::Pointer(value) => value
            .fields()
            .iter()
            .all(|(key, value)| session_transferable(key) && session_transferable(value)),
        Value::ExceptionInfo(value) => {
            session_transferable(&value.data)
                && value.cause.as_deref().map_or(true, session_transferable)
        }
        Value::ByteBuffer(_)
        | Value::Array(_)
        | Value::Object(_)
        | Value::Promise(_)
        | Value::Atom(_)
        | Value::Recur(_)
        | Value::Function(_)
        | Value::Seq(_)
        | Value::Iterator(_)
        | Value::Var(_)
        | Value::Namespace(_)
        | Value::Extension(_)
        | Value::StructType(_)
        | Value::MutableType(_)
        | Value::Mutable(_)
        | Value::Protocol(_)
        | Value::NativeType(_)
        | Value::Schema(_)
        | Value::Coroutine(_)
        | Value::Stream(_)
        | Value::Result(_)
        | Value::MutableCollection(_) => false,
    }
}

fn map_value<'a>(value: &'a Value, key: &Value) -> Option<&'a Value> {
    match value {
        Value::Map(values) => values.get(key),
        Value::OrderedMap(values) => values.get(key),
        Value::SortedMap(values) => values.get(key),
        Value::PriorityMap(values) => values.get(key),
        Value::Trie(values) => match key {
            Value::String(key) => values.get(key),
            _ => None,
        },
        _ => None,
    }
}

fn map_equality(left: &Value, right: &Value) -> Option<bool> {
    let left_entries = map_entries(left)?;
    let right_entries = map_entries(right)?;
    Some(
        left_entries.len() == right_entries.len()
            && left_entries
                .iter()
                .all(|(key, value)| map_value(right, key) == Some(value)),
    )
}

fn set_items(value: &Value) -> Option<Vec<&Value>> {
    match value {
        Value::Set(values) => Some(values.iter().collect()),
        Value::OrderedSet(values) => Some(values.iter().collect()),
        Value::SortedSet(values) => Some(values.iter().collect()),
        _ => None,
    }
}

fn set_equality(left: &Value, right: &Value) -> Option<bool> {
    let left_items = set_items(left)?;
    let right_items = set_items(right)?;
    Some(
        left_items.len() == right_items.len()
            && left_items.iter().all(|item| right_items.contains(item)),
    )
}

fn map_assoc_value(collection: &Value, key: Value, value: Value) -> Result<Value, String> {
    Ok(match collection {
        Value::Map(values) => Value::Map(values.assoc_value(key, value)),
        Value::OrderedMap(values) => Value::OrderedMap(Box::new(values.assoc_value(key, value))),
        Value::SortedMap(values) => Value::SortedMap(Box::new(values.assoc_value(key, value))),
        Value::PriorityMap(values) => Value::PriorityMap(Box::new(values.assoc_value(key, value))),
        Value::Trie(values) => match key {
            Value::String(key) => Value::Trie(Box::new(values.assoc_value(key, value))),
            _ => return Err("trie expects string keys".into()),
        },
        _ => return Err("assoc expects a map".into()),
    })
}

fn map_dissoc_value(collection: &Value, key: &Value) -> Result<Value, String> {
    Ok(match collection {
        Value::Map(values) => Value::Map(values.dissoc_value(key)),
        Value::OrderedMap(values) => Value::OrderedMap(Box::new(values.dissoc_value(key))),
        Value::SortedMap(values) => Value::SortedMap(Box::new(values.dissoc_value(key))),
        Value::PriorityMap(values) => Value::PriorityMap(Box::new(values.dissoc_value(key))),
        Value::Trie(values) => match key {
            Value::String(key) => Value::Trie(Box::new(values.dissoc_value(key))),
            _ => return Err("trie expects string keys".into()),
        },
        _ => return Err("dissoc expects a map".into()),
    })
}

fn set_find(collection: &Value, key: &Value) -> Option<Value> {
    set_items(collection)?
        .into_iter()
        .find(|value| *value == key)
        .cloned()
}

fn set_conj_value(collection: &Value, value: Value) -> Result<Value, String> {
    Ok(match collection {
        Value::Set(values) => Value::Set(values.conj_value(value)),
        Value::OrderedSet(values) => Value::OrderedSet(Box::new(values.conj_value(value))),
        Value::SortedSet(values) => Value::SortedSet(Box::new(values.conj_value(value))),
        _ => return Err("conj expects a set".into()),
    })
}

fn set_dissoc_value(collection: &Value, value: &Value) -> Result<Value, String> {
    Ok(match collection {
        Value::Set(values) => Value::Set(values.dissoc_value(value)),
        Value::OrderedSet(values) => Value::OrderedSet(Box::new(values.dissoc_value(value))),
        Value::SortedSet(values) => Value::SortedSet(Box::new(values.dissoc_value(value))),
        _ => return Err("dissoc expects a set".into()),
    })
}

fn collection_to_mutable(value: &Value) -> Result<Value, String> {
    let mutable = match value {
        Value::Map(values) => MutableCollection::Map(values.to_mutable()),
        Value::OrderedMap(values) => MutableCollection::OrderedMap(values.to_mutable()),
        Value::SortedMap(values) => MutableCollection::SortedMap(values.to_mutable()),
        Value::Trie(values) => MutableCollection::Trie(values.to_mutable()),
        Value::Set(values) => MutableCollection::Set(values.to_mutable()),
        Value::OrderedSet(values) => MutableCollection::OrderedSet(values.to_mutable()),
        Value::SortedSet(values) => MutableCollection::SortedSet(values.to_mutable()),
        Value::List(values) => MutableCollection::List(values.to_mutable()),
        Value::Queue(values) => MutableCollection::Queue(values.to_mutable()),
        Value::Vector(values) => MutableCollection::Vector(values.to_mutable()),
        Value::MutableCollection(_) => return Err("value is already mutable".into()),
        _ => return Err("to-mutable expects a persistent collection".into()),
    };
    Ok(Value::MutableCollection(Rc::new(RefCell::new(Some(
        mutable,
    )))))
}

fn collection_to_persistent(value: &Value) -> Result<Value, String> {
    let Value::MutableCollection(collection) = value else {
        return Err("to-persistent expects a mutable collection".into());
    };
    let mut mutable = collection
        .borrow_mut()
        .take()
        .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
    Ok(match &mut mutable {
        MutableCollection::Map(values) => Value::Map(values.to_persistent()),
        MutableCollection::OrderedMap(values) => {
            Value::OrderedMap(Box::new(values.to_persistent()))
        }
        MutableCollection::SortedMap(values) => Value::SortedMap(Box::new(values.to_persistent())),
        MutableCollection::Trie(values) => Value::Trie(Box::new(values.to_persistent())),
        MutableCollection::Set(values) => Value::Set(values.to_persistent()),
        MutableCollection::OrderedSet(values) => {
            Value::OrderedSet(Box::new(values.to_persistent()))
        }
        MutableCollection::SortedSet(values) => Value::SortedSet(Box::new(values.to_persistent())),
        MutableCollection::List(values) => Value::List(values.to_persistent()),
        MutableCollection::Queue(values) => Value::Queue(Box::new(values.to_persistent())),
        MutableCollection::Vector(values) => Value::Vector(values.to_persistent()),
    })
}

fn protocol_to_mutable(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Extension(receiver)] => extension_protocol_call(
            receiver,
            "std.protocol.itomutable.IToMutable",
            "to-mutable",
            arguments,
        ),
        [value] => collection_to_mutable(value),
        _ => Err("IToMutable/to-mutable expects one value".into()),
    }
}

fn protocol_to_persistent(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Extension(receiver)] => extension_protocol_call(
            receiver,
            "std.protocol.itopersistent.IToPersistent",
            "to-persistent",
            arguments,
        ),
        [value] => collection_to_persistent(value),
        _ => Err("IToPersistent/to-persistent expects one value".into()),
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        if let Some(equal) = sequential_equality(self, other) {
            return equal;
        }
        if let Some(equal) = map_equality(self, other) {
            return equal;
        }
        if let Some(equal) = set_equality(self, other) {
            return equal;
        }
        if let Some(equal) = numeric::numeric_equal(self, other) {
            return equal;
        }
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::BigInteger(a), Value::BigInteger(b)) => a == b,
            (Value::Character(a), Value::Character(b)) => a == b,
            (Value::Regex(a), Value::Regex(b)) => a == b,
            (Value::Tagged(a), Value::Tagged(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Keyword(a), Value::Keyword(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::ByteBuffer(a), Value::ByteBuffer(b)) => *a.borrow() == *b.borrow(),
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Promise(a), Value::Promise(b)) => a.same_identity(b),
            (Value::Atom(a), Value::Atom(b)) => a.same_identity(b),
            (Value::Recur(a), Value::Recur(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => a == b,
            (Value::Set(a), Value::Set(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Cons(a), Value::Cons(b)) => a == b,
            (Value::Symbol(a), Value::Symbol(b)) => a == b,
            (Value::Pointer(a), Value::Pointer(b)) => a == b,
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            (Value::Vector(a), Value::Vector(b)) => a == b,
            (Value::MapEntry(a), Value::MapEntry(b)) => a == b,
            (Value::MutableCollection(a), Value::MutableCollection(b)) => Rc::ptr_eq(a, b),
            (Value::Iterator(a), Value::Iterator(b)) => Rc::ptr_eq(a, b),
            (Value::Var(a), Value::Var(b)) => a.same_identity(b),
            (Value::Namespace(a), Value::Namespace(b)) => a.same_identity(b),
            (Value::Extension(a), Value::Extension(b)) => a == b,
            (Value::StructType(a), Value::StructType(b)) => Rc::ptr_eq(a, b),
            (Value::Struct(a), Value::Struct(b)) => {
                Rc::ptr_eq(&a.ty, &b.ty) && a.values == b.values
            }
            (Value::MutableType(a), Value::MutableType(b)) => Rc::ptr_eq(a, b),
            (Value::Mutable(a), Value::Mutable(b)) => a.same_identity(b),
            (Value::Protocol(a), Value::Protocol(b)) => Rc::ptr_eq(a, b),
            (Value::NativeType(a), Value::NativeType(b)) => a.name == b.name,
            (Value::Schema(a), Value::Schema(b)) => a.ast == b.ast,
            (Value::Coroutine(a), Value::Coroutine(b)) => Rc::ptr_eq(a, b),
            (Value::Stream(a), Value::Stream(b)) => Rc::ptr_eq(a, b),
            (Value::Result(a), Value::Result(b)) => a == b,
            (Value::ExceptionInfo(a), Value::ExceptionInfo(b)) => Rc::ptr_eq(a, b),
            (Value::Nil, Value::Nil) => true,
            _ => false,
        }
    }
}

impl Eq for Value {}
impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Value {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if let Some(ordering) = numeric::numeric_total_compare(self, other) {
            return ordering;
        }
        if self == other {
            return std::cmp::Ordering::Equal;
        }
        match (self, other) {
            (Value::Number(left), Value::Number(right)) => return left.cmp(right),
            (Value::Float(left), Value::Float(right)) => return left.total_cmp(right),
            (Value::Character(left), Value::Character(right)) => return left.cmp(right),
            (Value::Bool(left), Value::Bool(right)) => return left.cmp(right),
            (Value::String(left), Value::String(right)) => return left.cmp(right),
            (Value::Keyword(left), Value::Keyword(right)) => return left.cmp(right),
            (Value::BigInteger(left), Value::BigInteger(right)) => return left.cmp(right),
            _ => {}
        }
        fn rank(value: &Value) -> u8 {
            match value {
                Value::Nil => 0,
                Value::Bool(_) => 1,
                Value::Number(_) => 2,
                Value::Float(_) => 3,
                Value::BigInteger(_) => 4,
                Value::Character(_) => 5,
                Value::String(_) => 7,
                Value::Keyword(_) => 8,
                Value::Symbol(_) => 9,
                Value::Pointer(_) => 9,
                Value::List(_)
                | Value::Cons(_)
                | Value::Queue(_)
                | Value::Deque(_)
                | Value::Tuple(_)
                | Value::Vector(_)
                | Value::MapEntry(_)
                | Value::Seq(_) => 10,
                Value::Map(_)
                | Value::OrderedMap(_)
                | Value::SortedMap(_)
                | Value::Trie(_)
                | Value::PriorityMap(_) => 11,
                Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_) => 12,
                Value::Bytes(_) => 13,
                Value::ByteBuffer(_) => 14,
                Value::Regex(_) => 15,
                Value::Tagged(_) => 16,
                Value::Array(_) => 17,
                Value::Object(_) => 18,
                Value::Promise(_) => 19,
                Value::Atom(_) => 26,
                Value::Recur(_) => 20,
                Value::Function(_) => 21,
                Value::Iterator(_) => 22,
                Value::Var(_) => 23,
                Value::Namespace(_) => 24,
                Value::Extension(_) => 25,
                Value::StructType(_) => 27,
                Value::Struct(_) => 28,
                Value::MutableType(_) => 29,
                Value::Mutable(_) => 30,
                Value::Protocol(_) => 31,
                Value::NativeType(_) => 32,
                Value::Schema(_) => 33,
                Value::Coroutine(_) => 33,
                Value::Stream(_) => 34,
                Value::Result(_) => 36,
                Value::ExceptionInfo(_) => 37,
                Value::MutableCollection(_) => 38,
            }
        }
        rank(self)
            .cmp(&rank(other))
            .then_with(|| self.display().cmp(&other.display()))
            .then_with(|| self.stable_hash().cmp(&other.stable_hash()))
    }
}
impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // CHAMP placement needs Java's scale-zero integral layout, while the
        // ordinary Hash contract must remain canonical across numeric types.
        if crate::lang::data::map::champ_placement_hashing() {
            if let Self::Number(value) = self {
                state.write_u64(crate::lang::hash::hash_long_placement(*value) as i64 as u64);
                return;
            }
            if let Self::Float(value) = self {
                if value.is_finite() && value.fract() == 0.0 {
                    if let Ok(integer) = (*value).to_string().parse::<i64>() {
                        state.write_u64(
                            crate::lang::hash::hash_long_placement(integer) as i64 as u64,
                        );
                        return;
                    }
                }
            }
        }
        if let Some(hash) = numeric::numeric_hash(self) {
            state.write_u64(hash as i64 as u64);
            return;
        }
        match self {
            Value::Bool(value) => state.write_u64(crate::lang::hash::hash_bool(*value) as u64),
            Value::Nil => state.write_u64(0),
            _ => state.write_u64(self.stable_hash()),
        }
    }
}

impl crate::lang::hash::JavaHash for Value {
    /// The Java `long` hash of this value under `hash_type`, mirroring
    /// `G.hashFn(t).apply(o)`. See the `lang::hash` module docs for the
    /// parity rules and the documented deviations where Java hashes by
    /// object identity (keywords, pointers, SYSTEM/SIP collection hashes).
    fn java_hash(&self, hash_type: crate::lang::protocol::HashType) -> i64 {
        use crate::lang::hash as jh;
        use crate::lang::protocol::IHash;

        // Opaque (non-parity) identity hash for runtime objects whose Java
        // counterparts hash by object identity. Follows the previous
        // `stable_hash` scheme.
        fn opaque(
            tag: u64,
            write: impl FnOnce(&mut std::collections::hash_map::DefaultHasher),
        ) -> i64 {
            let mut state = std::collections::hash_map::DefaultHasher::new();
            tag.hash(&mut state);
            write(&mut state);
            state.finish() as i64
        }

        match self {
            Self::Nil => 0,
            Self::Bool(v) => jh::hash_bool(*v) as i64,
            Self::Character(v) => jh::hash_char(*v) as i64,
            Self::String(v) => jh::java_string_hash(v) as i64,
            Self::Number(value) => jh::hash_long(*value) as i64,
            Self::Float(value) => jh::hash_double(*value) as i64,
            Self::BigInteger(value) => jh::canonical_decimal_str_hash(&value.to_string()) as i64,
            // Java hashes java.util.regex.Pattern by identity; hash the
            // pattern string instead (deterministic deviation).
            Self::Regex(v) => jh::java_string_hash(v) as i64,
            Self::Keyword(v) => v.java_hash(hash_type),
            Self::Symbol(v) => v.java_hash(hash_type),
            Self::Pointer(v) => v.java_hash(hash_type),
            Self::Bytes(v) => jh::hash_bytes(v) as i64,
            Self::ByteBuffer(v) => jh::hash_bytes(v.borrow().as_slice()) as i64,
            // Java arrays hash by identity; composed deterministically here.
            Self::Array(v) => jh::compose_ordered(
                "SEQUENTIAL",
                v.borrow().iter().map(|item| item.java_hash(hash_type)),
            ),
            Self::Object(v) => jh::compose_unordered(
                "MAP",
                v.borrow().iter().map(|(key, item)| {
                    jh::compose_entry(jh::java_string_hash(key) as i64, item.java_hash(hash_type))
                }),
            ),
            Self::Recur(v) => {
                jh::compose_ordered("SEQUENTIAL", v.iter().map(|item| item.java_hash(hash_type)))
            }
            Self::Tagged(v) => jh::compose_ordered(
                "SEQUENTIAL",
                [v.tag().java_hash(hash_type), v.form().java_hash(hash_type)],
            ),
            Self::Map(v) => v.hash_calc(hash_type) as i64,
            Self::OrderedMap(v) => v.hash_calc(hash_type) as i64,
            Self::SortedMap(v) => v.hash_calc(hash_type) as i64,
            Self::PriorityMap(v) => v.hash_calc(hash_type) as i64,
            Self::Trie(v) => v.hash_calc(hash_type) as i64,
            Self::Set(v) => v.hash_calc(hash_type) as i64,
            Self::OrderedSet(v) => v.hash_calc(hash_type) as i64,
            Self::SortedSet(v) => v.hash_calc(hash_type) as i64,
            Self::List(v) => v.hash_calc(hash_type) as i64,
            Self::Cons(v) => v.hash_calc(hash_type) as i64,
            Self::Deque(v) => v.hash_calc(hash_type) as i64,
            Self::Queue(v) => v.hash_calc(hash_type) as i64,
            Self::Tuple(v) => v.hash_calc(hash_type) as i64,
            Self::Vector(v) => v.hash_calc(hash_type) as i64,
            Self::MapEntry(v) => v.hash_calc(hash_type) as i64,
            Self::Seq(v) => jh::compose_ordered(
                "SEQUENTIAL",
                v.iter().map(|item| match item {
                    Ok(value) => value.java_hash(hash_type),
                    Err(error) => jh::java_string_hash(&error) as i64,
                }),
            ),
            Self::MutableCollection(v) => opaque(32, |s| Rc::as_ptr(v).hash(s)),
            Self::Promise(v) => opaque(8, |s| v.identity_address().hash(s)),
            Self::Atom(v) => opaque(28, |s| v.identity_address().hash(s)),
            Self::Function(v) => opaque(14, |s| Rc::as_ptr(v).hash(s)),
            Self::Iterator(v) => opaque(16, |s| Rc::as_ptr(v).hash(s)),
            Self::Var(v) => opaque(17, |s| v.identity_address().hash(s)),
            Self::Namespace(v) => opaque(27, |s| v.identity_address().hash(s)),
            Self::Extension(v) => opaque(18, |s| {
                v.provider.hash(s);
                v.type_name.hash(s);
                v.handle.hash(s);
            }),
            Self::StructType(v) => opaque(26, |s| Rc::as_ptr(v).hash(s)),
            Self::Struct(v) => opaque(27, |s| {
                Rc::as_ptr(&v.ty).hash(s);
                for value in v.ordered_values() {
                    value.hash(s);
                }
            }),
            Self::MutableType(v) => opaque(28, |s| Rc::as_ptr(v).hash(s)),
            Self::Mutable(v) => opaque(29, |s| v.identity_address().hash(s)),
            Self::Protocol(v) => opaque(30, |s| v.name.hash(s)),
            Self::NativeType(v) => opaque(31, |s| v.name.hash(s)),
            Self::Schema(v) => opaque(34, |s| v.form.to_string().hash(s)),
            Self::Coroutine(v) => opaque(32, |s| Rc::as_ptr(v).hash(s)),
            Self::Stream(v) => opaque(35, |s| Rc::as_ptr(v).hash(s)),
            Self::Result(v) => v.java_hash(hash_type),
            Self::ExceptionInfo(v) => opaque(33, |s| Rc::as_ptr(v).hash(s)),
        }
    }
}

impl Value {
    pub fn display(&self) -> String {
        match self {
            Self::Number(v) => v.to_string(),
            Self::Float(v) => {
                assert!(v.is_finite(), "non-finite number");
                format!("(double {v})")
            }
            Self::BigInteger(v) => v.to_string(),
            Self::Character('\n') => "\\newline".into(),
            Self::Character(' ') => "\\space".into(),
            Self::Character('\t') => "\\tab".into(),
            Self::Character('\u{0008}') => "\\backspace".into(),
            Self::Character('\u{000c}') => "\\formfeed".into(),
            Self::Character('\r') => "\\return".into(),
            Self::Character(v) if v.is_control() => format!("\\u{:04X}", *v as u32),
            Self::Character(v) => format!("\\{v}"),
            Self::Regex(v) => crate::kernel::form::display_regex(v),
            Self::Tagged(value) => uuid_text_from_tagged(value).map_or_else(
                || format!("#{}{}", value.tag().as_str(), value.form().display()),
                str::to_owned,
            ),
            Self::Bool(v) => v.to_string(),
            Self::String(v) => crate::kernel::form::display_string(v),
            Self::Keyword(v) => format!(":{}", v.as_str()),
            Self::Bytes(values) => format!(
                "#bytes[{}]",
                values
                    .iter()
                    .map(|v| (*v as i8).to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::ByteBuffer(values) => {
                let body = values
                    .borrow()
                    .iter()
                    .map(|v| (*v as i8).to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                if body.is_empty() {
                    "(bytes)".into()
                } else {
                    format!("(bytes {body})")
                }
            }
            Self::Array(values) => format!(
                "(array {})",
                values
                    .borrow()
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::Object(values) => format!(
                "(object {})",
                values
                    .borrow()
                    .iter()
                    .map(|(key, value)| format!("\"{}\" {}", key, value.display()))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::Promise(_) => "<promise>".into(),
            Self::Atom(value) => format!("#atom <{}>", value.deref_value().display()),
            Self::Recur(values) => format!(
                "<recur {}>",
                values
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            value @ (Self::Map(_)
            | Self::OrderedMap(_)
            | Self::SortedMap(_)
            | Self::PriorityMap(_)
            | Self::Trie(_)) => {
                format!(
                    "{{{}}}",
                    map_entries(value)
                        .unwrap()
                        .iter()
                        .map(|(k, v)| format!("{} {}", k.display(), v.display()))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            }
            value @ (Self::Set(_) | Self::OrderedSet(_) | Self::SortedSet(_)) => format!(
                "#{{{}}}",
                set_items(value)
                    .unwrap()
                    .iter()
                    .map(|item| item.display())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::Queue(values) => format!(
                "#queue[{}]",
                values
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::Deque(values) => format!(
                "#deque[{}]",
                values
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::Cons(values) => format!(
                "({})",
                values
                    .iter()
                    .map(|value| value.display())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::List(values) => format!(
                "({})",
                values
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::Symbol(v) => v.as_str().to_owned(),
            Self::Pointer(v) => v.display(),
            Self::Function(_) => "<fn>".into(),
            Self::Tuple(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::MapEntry(entry) => entry.display(),
            Self::Vector(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::MutableCollection(values) => {
                let borrowed = values.borrow();
                let Some(values) = borrowed.as_ref() else {
                    return "#<mutable-frozen>".into();
                };
                let kind = match values {
                    MutableCollection::Map(_) => "map",
                    MutableCollection::OrderedMap(_) => "ordered-map",
                    MutableCollection::SortedMap(_) => "sorted-map",
                    MutableCollection::Trie(_) => "trie",
                    MutableCollection::Set(_) => "set",
                    MutableCollection::OrderedSet(_) => "ordered-set",
                    MutableCollection::SortedSet(_) => "sorted-set",
                    MutableCollection::List(_) => "list",
                    MutableCollection::Queue(_) => "queue",
                    MutableCollection::Vector(_) => "vector",
                };
                format!("#<mutable-{kind}>")
            }
            Self::Seq(sequence) => {
                let mut values = sequence.iter();
                let mut displayed = Vec::new();
                for _ in 0..10 {
                    match values.next() {
                        Some(Ok(value)) => displayed.push(value.display()),
                        Some(Err(error)) => {
                            displayed.push(format!("#error[{}]", Value::String(error).display()));
                            break;
                        }
                        None => break,
                    }
                }
                if values.next().is_some() {
                    displayed.push("...".into());
                }
                format!("({})", displayed.join(" "))
            }
            Self::Iterator(_) => "<iterator>".into(),
            Self::Var(value) => value.display(),
            Self::Namespace(value) => format!("#namespace[{}]", value.name().as_str()),
            Self::Extension(value) => format!("#ht[:handle {}]", value.handle),
            Self::StructType(value) => value.name.clone(),
            Self::Struct(value) => format!(
                "#{}{{{}}}",
                value.ty.name,
                value
                    .ty
                    .fields
                    .iter()
                    .filter_map(|field| value.get(field).map(|value| (field, value)))
                    .map(|(field, value)| format!(":{field} {}", value.display()))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::MutableType(value) => value.name.clone(),
            Self::Mutable(value) => format!(
                "#{}{{{}}}",
                value.ty.name,
                value
                    .ty
                    .fields
                    .iter()
                    .zip(value.ordered_values())
                    .map(|(field, value)| format!(":{field} {}", value.display()))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Self::Protocol(value) => format!("#protocol[{}]", value.name),
            Self::NativeType(value) => format!("#<native-type {}>", value.name),
            Self::Schema(value) => format!("(schema {})", value.form),
            Self::Coroutine(value) => {
                let status = match &*value.state.borrow() {
                    CoroutineState::New(_) | CoroutineState::Suspended(_) => "suspended",
                    CoroutineState::Running => "running",
                    CoroutineState::Dead => "dead",
                };
                format!("#<coroutine {status}>")
            }
            Self::Stream(value) => format!(
                "#<stream {}>",
                if value.closed.get() {
                    "closed"
                } else {
                    "ready"
                }
            ),
            Self::Result(value) => value.display(),
            Self::ExceptionInfo(value) => {
                format!(
                    "#error[{} {}]",
                    Self::String(value.message.clone()).display(),
                    value.data.display()
                )
            }
            Self::Nil => "nil".into(),
        }
    }
    pub(crate) fn truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Bool(false))
    }

    /// Stable structural hash used by protocol and collection conformance tests.
    ///
    /// This is the Java-parity RAPID hash: the bit pattern (as `u64`) of the
    /// Java `long` produced by `G.hashRapid` / `hashCalc(RAPID)` on the
    /// equivalent Java value. Value-level hashes are Java `int` results
    /// sign-extended to 64 bits, matching `G.hashValue` returning `long`.
    /// Opaque runtime objects (functions, atoms, promises, …) keep the
    /// previous identity-based `DefaultHasher` scheme — their Java
    /// counterparts hash by object identity, so no parity exists there.
    pub fn stable_hash(&self) -> u64 {
        self.java_hash(crate::lang::hash::DEFAULT_HASH) as u64
    }
}
