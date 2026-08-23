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

/// Closed accounting inventory for ordinary runtime-owned callable Vars.
///
/// This intentionally lives separately from `DIRECT_CALLABLE_CATALOG`: runtime
/// startup and tests compare the two sets so a new evaluator/native operation
/// cannot silently acquire a structural fallback.
pub(crate) const RUNTIME_CALLABLE_INVENTORY: &[&str] = &[
    "%",
    "*",
    "+",
    "-",
    "/",
    "<",
    "<=",
    "=",
    ">",
    ">=",
    "alter-var-root",
    "any?",
    "apply",
    "array",
    "assoc",
    "assoc-in",
    "atom",
    "bit-and",
    "bit-not",
    "bit-or",
    "bit-shift-left",
    "bit-shift-right",
    "bit-xor",
    "boolean",
    "boolean?",
    "bytes",
    "capture",
    "cas!",
    "char?",
    "comp",
    "comp2",
    "comp3",
    "compare",
    "complement",
    "concat",
    "conj",
    "cons",
    "constantly",
    "count",
    "current-namespace",
    "cycle",
    "dec",
    "deref",
    "dissoc",
    "double?",
    "drop",
    "drop-while",
    "empty",
    "empty?",
    "eval",
    "eval-in-ns",
    "even?",
    "every?",
    "ex",
    "ex-cause",
    "ex-class",
    "ex-data",
    "ex-info",
    "ex-message",
    "ex-native-type",
    "ex-provenance",
    "false?",
    "filter",
    "first",
    "fn?",
    "function?",
    "get",
    "get-in",
    "hash-map",
    "hash-set",
    "identity",
    "inc",
    "instance?",
    "integer?",
    "interleave",
    "intern-var",
    "interpose",
    "iter",
    "iter-next",
    "iter-next?",
    "iter?",
    "iterate",
    "keep",
    "key",
    "keys",
    "keyword",
    "keyword?",
    "last",
    "list",
    "list?",
    "load-string",
    "long?",
    "map",
    "map?",
    "mapcat",
    "meta",
    "mod",
    "name",
    "namespace",
    "neg?",
    "nil?",
    "not",
    "not-empty",
    "not=",
    "ns-alias-state",
    "ns-loaded?",
    "ns-state",
    "ns:create",
    "nth",
    "number?",
    "object",
    "odd?",
    "p",
    "pair",
    "partition",
    "partition-all",
    "partition-pair",
    "peek",
    "pointer",
    "pos?",
    "pr-str",
    "println",
    "promise",
    "promise/delay",
    "promise/new",
    "promise?",
    "quot",
    "range",
    "read-string",
    "rem",
    "repeat",
    "repeatedly",
    "reset!",
    "resolve",
    "rest",
    "reverse",
    "second",
    "seq",
    "seq?",
    "set?",
    "satisfies?",
    "str",
    "string?",
    "swap!",
    "symbol",
    "symbol?",
    "take",
    "take-while",
    "true?",
    "tup",
    "type",
    "update",
    "update-in",
    "val",
    "vals",
    "var-sym",
    "vector",
    "vector?",
    "with-meta",
    "zero?",
    "zip",
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

pub(crate) const NATIVE_TYPES: &[(&str, &[&str])] = &[
    (
        "Maths",
        &[
            "abs", "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "ceil", "cos",
            "cosh", "exp", "floor", "pow", "sin", "sinh", "sqrt", "tan", "tanh",
        ],
    ),
    ("Num", &["long", "double", "parse-long", "parse-double"]),
    (
        "Bits",
        &["and", "or", "xor", "not", "shift-left", "shift-right"],
    ),
    (
        "Kernel",
        &[
            "session-create",
            "session-close",
            "session-list",
            "session-info",
            "session-eval",
            "session-namespace",
            "session-complete",
            "resource-register",
            "resource-remove",
            "resource-list",
            "filesystem-create",
            "filesystem-attach",
            "filesystem-detach",
            "filesystem-info",
            "filesystem-close",
            "capabilities",
        ],
    ),
    (
        "Sandbox",
        &["open", "eval", "call", "cancel", "status", "close"],
    ),
    (
        "Package",
        &["catalog", "find", "ensure", "load", "unload", "state"],
    ),
    (
        "String",
        &[
            "length",
            "blank?",
            "includes?",
            "starts-with?",
            "ends-with?",
            "char-at",
            "slice",
            "index-of",
            "last-index-of",
            "join",
            "split",
            "split-lines",
            "repeat",
            "replace",
            "replace-first",
            "trim",
            "trim-left",
            "trim-right",
            "upper",
            "lower",
            "capitalize",
            "decapitalize",
            "pad-left",
            "pad-right",
            "reverse",
            "encode-utf8",
            "decode-utf8",
            "to-fixed",
        ],
    ),
    (
        "Bytes",
        &[
            "new",
            "instance?",
            "count",
            "get",
            "set",
            "copy",
            "slice",
            "u8",
            "s8",
        ],
    ),
    (
        "Crypto",
        &[
            "sha256",
            "sha512",
            "hmac-sha256",
            "hmac-sha512",
            "random-bytes",
            "secure-equal?",
            "ed25519-keypair",
            "ed25519-public",
            "ed25519-sign",
            "ed25519-verify",
            "x25519-keypair",
            "x25519-public",
            "x25519-shared",
            "p256-keypair",
            "p256-public",
            "p256-sign",
            "p256-verify",
            "p256-shared",
        ],
    ),
    (
        "OS",
        &[
            "platform", "arch", "cwd", "env", "getenv", "time-ms", "time-ns",
        ],
    ),
    (
        "Process",
        &[
            "spawn",
            "instance?",
            "alive?",
            "write",
            "close-input",
            "stdout",
            "stderr",
            "stdout-stream",
            "stderr-stream",
            "wait",
            "kill",
        ],
    ),
    (
        "File",
        &[
            "parent",
            "join",
            "resolve",
            "read",
            "write",
            "exists?",
            "stat",
            "entries",
            "list",
            "walk",
            "mkdir",
            "delete",
            "copy",
            "move",
            "temp-file",
            "temp-directory",
        ],
    ),
    (
        "Socket",
        &[
            "connect",
            "listen",
            "endpoint",
            "events",
            "next",
            "send",
            "close",
            "receive-stream",
        ],
    ),
    (
        "Promise",
        &["run", "new", "from", "all", "delay", "instance?"],
    ),
    ("Coroutine", &["create", "yield", "await", "instance?"]),
    ("Stream", &["create", "generate", "next", "instance?"]),
    (
        "Arr",
        &[
            "new",
            "instance?",
            "get",
            "set",
            "push-first",
            "push-last",
            "pop-first",
            "pop-last",
            "insert",
            "remove",
            "clone",
            "slice",
            "map",
            "filter",
            "fold-left",
            "fold-right",
        ],
    ),
    (
        "Obj",
        &[
            "new",
            "instance?",
            "get",
            "set",
            "has?",
            "delete",
            "clone",
            "assign",
            "keys",
            "vals",
            "pairs",
        ],
    ),
    (
        "Runtime",
        &[
            "load-string",
            "macroexpand-1",
            "gensym",
            "var-sym",
            "current",
            "snapshot",
            "vars",
            "namespaces",
            "namespace",
            "module",
            "resolve",
            "alias-state",
            "intern-var",
            "eval-in",
            "eval",
        ],
    ),
    ("Printer", &["p", "println", "capture"]),
    (
        "Document",
        &[
            "element",
            "text",
            "fragment",
            "annotate",
            "pass",
            "escaped",
            "group",
            "line",
            "break",
            "nest",
            "align",
            "normalize",
            "valid?",
            "render",
        ],
    ),
    ("Edn", &["read", "read-forms", "write", "pretty"]),
    ("Json", &["read", "write", "pretty"]),
    ("Host", &["call", "describe", "capabilities", "capability?"]),
    (
        "Test",
        &[
            "catalog",
            "config",
            "context",
            "events",
            "compare",
            "run",
            "result",
            "passed?",
            "actual",
            "expected",
            "failures",
            "failure-seq",
            "failure-count",
            "failure",
            "failure?",
        ],
    ),
    (
        "RegExp",
        &[
            "instance?",
            "compile",
            "pattern",
            "find?",
            "find",
            "matches",
            "replace",
            "split",
        ],
    ),
    ("UUID", &["instance?"]),
    (
        "Result",
        &[
            "create",
            "synchronize",
            "instance?",
            "success?",
            "error?",
            "status",
            "data",
            "error-value",
            "context",
            "with-context",
        ],
    ),
    (
        "Schema",
        &[
            "compile",
            "of",
            "instance?",
            "kind",
            "form",
            "ast",
            "origin",
        ],
    ),
    ("Error", &["new", "message", "class"]),
    (
        "Base",
        &[
            "list",
            "vector",
            "vec",
            "set",
            "tuple",
            "hash-map",
            "hash-set",
            "atom",
            "pointer",
            "symbol",
            "keyword",
            "reduced",
            "unreduced",
            "apply",
            "not",
            "boolean",
            "compare",
            "reduced?",
            "nil?",
            "boolean?",
            "string?",
            "char?",
            "number?",
            "integer?",
            "long?",
            "double?",
            "keyword?",
            "symbol?",
            "pointer?",
            "atom?",
            "function?",
            "bytes?",
            "array?",
            "object?",
            "list?",
            "cons?",
            "vector?",
            "tuple?",
            "map?",
            "set?",
            "sequential?",
            "coll?",
            "satisfies?",
            "type",
            "instance?",
        ],
    ),
    (
        "Algo",
        &[
            "deque",
            "ordered-map",
            "ordered-set",
            "priority-map",
            "queue",
            "sorted-map",
            "sorted-set",
            "trie",
            "deque?",
            "ordered-map?",
            "ordered-set?",
            "priority-map?",
            "queue?",
            "sorted-map?",
            "sorted-set?",
            "trie?",
        ],
    ),
    (
        "Iter",
        &[
            "iter",
            "iter?",
            "iter-finite?",
            "iter-materialize",
            "iter-next?",
            "iter-next",
            "iter-close",
            "iter-concat",
            "iter-map",
            "iter-filter",
            "iter-take-while",
            "iter-drop-while",
            "iter-mapcat",
            "iter-keep",
            "iter-interpose",
            "iter-interleave",
            "iter-every?",
            "iter-any?",
            "iter-take",
            "iter-drop",
            "iter-zip",
            "iter-cycle",
            "iter-partition-pair",
            "iter-partition-all",
            "iter-partition",
            "iter-range",
            "iter-constantly",
            "iter-repeatedly",
            "iter-iterate",
        ],
    ),
];

pub(crate) fn native_type_values() -> Vec<(String, Value)> {
    NATIVE_TYPES
        .iter()
        .map(|(name, methods)| {
            (
                (*name).to_owned(),
                Value::NativeType(Rc::new(NativeType {
                    name: format!("std.native.{name}"),
                    methods: methods.iter().map(|method| (*method).to_owned()).collect(),
                    metadata: None,
                })),
            )
        })
        .collect()
}

pub(crate) const FOUNDATION_PROTOCOLS: &[(&str, &[(&str, usize)])] = &[
    (
        "IApplicable",
        &[
            ("apply-in", 3),
            ("apply-default", 1),
            ("transform-in", 3),
            ("transform-out", 4),
        ],
    ),
    ("IAssoc", &[("assoc", 3)]),
    ("ICas", &[("cas", 3)]),
    ("IClose", &[("close", 1)]),
    ("IStream", &[("next", 1)]),
    ("IStreamWrite", &[("write", 2)]),
    ("IAbort", &[("abort", 2)]),
    ("IStreamPoll", &[("poll", 1)]),
    ("IStreamOffer", &[("offer", 2)]),
    ("IClosed", &[("closed?", 1)]),
    ("IFlush", &[("flush", 1)]),
    ("IStreamDuplex", &[]),
    (
        "IComponent",
        &[
            ("props", 1),
            ("status", 1),
            ("started?", 1),
            ("stopped?", 1),
            ("start", 1),
            ("stop", 1),
            ("kill", 1),
            ("remote?", 1),
        ],
    ),
    ("IWork", &[("work-spec", 1)]),
    ("IWorkExecutor", &[("work-execute", 2)]),
    ("IWorkStore", &[("work-query", 2), ("work-transact", 2)]),
    ("IWorkRef", &[("work-id", 1)]),
    ("IWorkHost", &[("work-submit", 4), ("work-resolve", 2)]),
    (
        "IWorkRun",
        &[
            ("work-status", 1),
            ("work-result", 1),
            ("work-events", 2),
            ("work-cancel", 2),
        ],
    ),
    ("IConj", &[("conj", 2)]),
    ("ICons", &[("cons", 2)]),
    ("IContext", &[("call", usize::MAX)]),
    ("ICoroutine", &[("status", 1), ("resume", usize::MAX)]),
    (
        "IContextLifeCycle",
        &[
            ("has-module?", 2),
            ("setup-module", 2),
            ("teardown-module", 2),
            ("has-pointer?", 2),
            ("setup-pointer", 2),
            ("teardown-pointer", 2),
        ],
    ),
    ("ICount", &[("count", 1)]),
    (
        "IDeps",
        &[("dep-get", 2), ("dep-entries", 2), ("dep-keys", 1)],
    ),
    ("IDeref", &[("deref", 1)]),
    ("IDerefTimeout", &[("deref-timeout", 3)]),
    ("IDisplay", &[("display", 1)]),
    ("IDissoc", &[("dissoc", 2)]),
    ("IEmpty", &[("empty", 1)]),
    ("IEncodable", &[("encode-with", 2)]),
    ("IEncode", &[("encode", 2)]),
    (
        "IEncodeVisitor",
        &[
            ("visit-nil", 1),
            ("visit-boolean", 2),
            ("visit-number", 2),
            ("visit-character", 2),
            ("visit-string", 2),
            ("visit-keyword", 2),
            ("visit-symbol", 2),
            ("visit-seq", 2),
            ("visit-vector", 2),
            ("visit-map", 2),
            ("visit-set", 2),
            ("visit-tagged", 3),
            ("visit-unknown", 2),
        ],
    ),
    ("IEquality", &[("equality", 2)]),
    ("IExInfo", &[("data", 1)]),
    ("IFind", &[("find", 2)]),
    ("IFn", &[("invoke", usize::MAX)]),
    ("IHash", &[("hash", 1)]),
    ("IHashCached", &[("hash-current", 1), ("hash-put", 2)]),
    ("IIndexed", &[("index-of", 2)]),
    ("IIndexedKV", &[("index-of-key", 2), ("index-of-val", 2)]),
    ("IInvokeIn", &[("invoke-in", usize::MAX)]),
    ("IIter", &[("iter", 1)]),
    ("IIterator", &[("iter-next?", 1), ("iter-next", 1)]),
    ("ILookup", &[("lookup", usize::MAX)]),
    ("IMatch", &[("match-value", 2)]),
    ("IStringLike", &[("to-string", 1), ("from-string", 2)]),
    ("IMutable", &[]),
    ("INamespaced", &[("name", 1), ("namespace", 1)]),
    ("INth", &[("nth", 2)]),
    ("IOFn", &[]),
    ("IObjType", &[("meta", 1), ("with-meta", 2)]),
    ("IPair", &[("key", 1), ("value", 1)]),
    ("IPeekFirst", &[("peek-first", 1)]),
    ("IPeekLast", &[("peek-last", 1)]),
    ("IPersistent", &[]),
    (
        "IPromise",
        &[
            ("state", 1),
            ("value", 1),
            ("then", 2),
            ("catch", 2),
            ("finally", 2),
            ("cancel", 1),
        ],
    ),
    ("IPointer", &[("ptr-context", 1)]),
    ("IPopFirst", &[("pop-first", 1)]),
    ("IPopLast", &[("pop-last", 1)]),
    ("IPushFirst", &[("push-first", 2)]),
    ("IPushLast", &[("push-last", 2)]),
    ("IRealize", &[("realized?", 1), ("realize", 1)]),
    ("IReduce", &[("reduce", usize::MAX)]),
    ("IReset", &[("reset", 2)]),
    (
        "ISpace",
        &[
            ("context-set", 4),
            ("context-unset", 2),
            ("context-list", 1),
            ("context-get", 2),
            ("rt-active", 1),
            ("rt-get", 2),
            ("rt-start", 2),
            ("rt-started?", 2),
            ("rt-stopped?", 2),
            ("rt-stop", 2),
        ],
    ),
    ("IToMutable", &[("to-mutable", 1)]),
    ("IToPersistent", &[("to-persistent", 1)]),
    (
        "IWatch",
        &[("watch-add", 3), ("watch-remove", 2), ("watch-list", 1)],
    ),
];

pub(crate) fn builtin_protocol_namespace(protocol: &str) -> String {
    format!("std.protocol.{}", protocol.to_ascii_lowercase())
}

pub(crate) fn builtin_protocol_name(protocol: &str) -> String {
    format!("{}/{}", builtin_protocol_namespace(protocol), protocol)
}

pub(crate) fn builtin_protocol_parents(protocol: &str) -> Vec<String> {
    let parents: &[&str] = match protocol {
        "ICoroutine" | "IStream" => &["IClose"],
        "IHashCached" => &["IHash"],
        "IIterator" => &["IIter"],
        "ILookup" => &["IFind"],
        "IOFn" => &["IFn"],
        "IObjType" => &["IHash", "IDisplay"],
        "IPromise" => &["IDeref", "IDerefTimeout"],
        "IToMutable" => &["IPersistent"],
        "IToPersistent" => &["IMutable"],
        "IStreamDuplex" => &["IStream", "IStreamWrite", "IAbort"],
        "IWorkHost" => &["IComponent"],
        "IWorkRun" => &["IWorkRef", "IClosed"],
        _ => &[],
    };
    parents
        .iter()
        .map(|parent| builtin_protocol_name(parent))
        .collect()
}

fn canonical_protocol_name(protocol: &str) -> String {
    let simple = protocol.strip_prefix("std.foundation/").unwrap_or(protocol);
    if FOUNDATION_PROTOCOLS
        .iter()
        .any(|(candidate, _)| *candidate == simple)
    {
        builtin_protocol_name(simple)
    } else {
        protocol.to_owned()
    }
}

pub(crate) fn foundation_protocol_values() -> Vec<(String, Value)> {
    FOUNDATION_PROTOCOLS
        .iter()
        .map(|(name, methods)| {
            (
                (*name).to_owned(),
                Value::Protocol(Rc::new(GuestProtocol {
                    name: builtin_protocol_name(name),
                    methods: methods
                        .iter()
                        .map(|(method, arity)| ((*method).to_owned(), *arity))
                        .collect(),
                    parents: builtin_protocol_parents(name),
                })),
            )
        })
        .collect()
}

pub(crate) fn builtin_protocol_method_values() -> Vec<(String, String, Value)> {
    FOUNDATION_PROTOCOLS
        .iter()
        .flat_map(|(protocol, methods)| {
            methods.iter().map(move |(method, arity)| {
                let namespace = builtin_protocol_namespace(protocol);
                let protocol_name = builtin_protocol_name(protocol);
                let method_name = (*method).to_owned();
                let display_name = format!("{namespace}/{method}");
                let arity_display_name = display_name.clone();
                let (minimum_arity, maximum_arity) =
                    builtin_protocol_arity_range(protocol, method, *arity);
                (
                    namespace,
                    (*method).to_owned(),
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

fn builtin_protocol_arity_range(
    protocol: &str,
    method: &str,
    declared_arity: usize,
) -> (usize, Option<usize>) {
    if declared_arity != usize::MAX {
        return (declared_arity, Some(declared_arity));
    }
    match (protocol, method) {
        ("ILookup", "lookup") | ("IReduce", "reduce") => (2, Some(3)),
        ("IInvokeIn", "invoke-in") => (2, None),
        _ => (1, None),
    }
}

#[cfg(test)]
mod native_work_protocol_tests {
    use super::*;

    fn methods(name: &str) -> Vec<(&'static str, usize)> {
        FOUNDATION_PROTOCOLS
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, methods)| methods.to_vec())
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
        assert_eq!(canonical_protocol_name("IFn"), "std.protocol.ifn/IFn");
        assert_eq!(
            canonical_protocol_name("std.foundation/IFn"),
            "std.protocol.ifn/IFn"
        );
        assert_eq!(
            canonical_protocol_name("std.protocol.ifn/IFn"),
            "std.protocol.ifn/IFn"
        );
        assert_eq!(
            canonical_protocol_name("std.protocol.application/Portable"),
            "std.protocol.application/Portable"
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
