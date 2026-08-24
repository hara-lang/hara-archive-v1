/// Closed inventory of ordinary Rust runtime callables.
///
/// These values are the only unqualified callable Vars installed before the
/// canonical Foundation source is loaded. Every entry names its arity,
/// target availability, ownership origin, and direct value-level
/// implementation. Syntax, macros, control forms, namespace declarations, and
/// explicit semantic evaluation remain structurally handled and are absent
/// unless the catalog marks them as an explicit semantic or namespace-mutation
/// operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectCallableArity {
    Exact(usize),
    Between { minimum: usize, maximum: usize },
    AtLeast(usize),
    Even,
    EvenAtLeast(usize),
    OddAtLeast(usize),
    Any,
}

impl DirectCallableArity {
    fn accepts(self, count: usize) -> bool {
        match self {
            Self::Exact(expected) => count == expected,
            Self::Between { minimum, maximum } => (minimum..=maximum).contains(&count),
            Self::AtLeast(minimum) => count >= minimum,
            Self::Even => count % 2 == 0,
            Self::EvenAtLeast(minimum) => count >= minimum && count % 2 == 0,
            Self::OddAtLeast(minimum) => count >= minimum && count % 2 == 1,
            Self::Any => true,
        }
    }

    fn description(self) -> String {
        match self {
            Self::Exact(expected) => format!("exactly {expected}"),
            Self::Between { minimum, maximum } => format!("between {minimum} and {maximum}"),
            Self::AtLeast(minimum) => format!("at least {minimum}"),
            Self::Even => "an even number of".into(),
            Self::EvenAtLeast(minimum) => format!("an even number of, at least {minimum}"),
            Self::OddAtLeast(minimum) => format!("an odd number of, at least {minimum}"),
            Self::Any => "any number of".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectCallableAvailability {
    AllTargets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectCallableOrigin {
    BootstrapLibrary,
    RuntimePrimitive,
    ExplicitSemantic,
    NamespaceMutation,
}

impl DirectCallableOrigin {
    pub(crate) fn var_origin(self) -> VarOrigin {
        match self {
            Self::BootstrapLibrary => VarOrigin::RustLibrary,
            Self::RuntimePrimitive | Self::ExplicitSemantic | Self::NamespaceMutation => {
                VarOrigin::RuntimePrimitive
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn ordinary(self) -> bool {
        matches!(self, Self::BootstrapLibrary | Self::RuntimePrimitive)
    }
}

pub(crate) type DirectCallableFunction =
    fn(&DirectCallableSpec, Vec<Value>) -> Result<Value, String>;

#[derive(Debug, Clone, Copy)]
pub(crate) enum DirectCallableImplementation {
    Basic,
    Exception,
    Runtime(DirectRuntimeCallable),
    Operation(DirectCallableFunction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectRuntimeCallable {
    AlterVarRoot,
    Array,
    Bytes,
    Capture,
    Compose2,
    Compose3,
    Conj,
    Cons,
    CurrentNamespace,
    Deref,
    Dissoc,
    Empty,
    Iter,
    IterNext,
    IterNextPredicate,
    IterPredicate,
    List,
    LoadString,
    Name,
    Namespace,
    NamespaceLoaded,
    NamespaceState,
    NamespaceCreate,
    Object,
    Peek,
    Print,
    PrintRepresentation,
    Println,
    Promise,
    PromisePredicate,
    ReadString,
    Resolve,
    Seq,
    SeqPredicate,
    String,
    Tuple,
    Type,
    WithMeta,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DirectCallableSpec {
    pub(crate) symbol: &'static str,
    pub(crate) arity: DirectCallableArity,
    pub(crate) availability: DirectCallableAvailability,
    pub(crate) origin: DirectCallableOrigin,
    pub(crate) implementation: DirectCallableImplementation,
}

macro_rules! direct {
    ($symbol:literal, $arity:expr, $origin:ident, Basic) => {
        DirectCallableSpec {
            symbol: $symbol,
            arity: $arity,
            availability: DirectCallableAvailability::AllTargets,
            origin: DirectCallableOrigin::$origin,
            implementation: DirectCallableImplementation::Basic,
        }
    };
    ($symbol:literal, $arity:expr, $origin:ident, Exception) => {
        DirectCallableSpec {
            symbol: $symbol,
            arity: $arity,
            availability: DirectCallableAvailability::AllTargets,
            origin: DirectCallableOrigin::$origin,
            implementation: DirectCallableImplementation::Exception,
        }
    };
    ($symbol:literal, $arity:expr, $origin:ident, Operation($implementation:path)) => {
        DirectCallableSpec {
            symbol: $symbol,
            arity: $arity,
            availability: DirectCallableAvailability::AllTargets,
            origin: DirectCallableOrigin::$origin,
            implementation: DirectCallableImplementation::Operation($implementation),
        }
    };
    ($symbol:literal, $arity:expr, $origin:ident, $implementation:ident) => {
        DirectCallableSpec {
            symbol: $symbol,
            arity: $arity,
            availability: DirectCallableAvailability::AllTargets,
            origin: DirectCallableOrigin::$origin,
            implementation: DirectCallableImplementation::Runtime(
                DirectRuntimeCallable::$implementation,
            ),
        }
    };
}

#[cfg(feature = "bytecode-vm")]
pub(crate) const BYTECODE_PROTOCOL_PREDICATES: &[&str] = &[
    "coll?",
    "iterable?",
    "iterator?",
    "counted?",
    "reducible?",
    "indexed?",
    "associative?",
    "findable?",
    "lookupable?",
    "derefable?",
    "resettable?",
    "casable?",
    "watchable?",
    "fn?",
    "applicable?",
    "mutable?",
    "persistent?",
];

#[cfg(feature = "bytecode-vm")]
pub(crate) const BYTECODE_BOOTSTRAP_ONLY_CALLABLES: &[&str] =
    &["gensym", "macroexpand-1", "ns-publics"];

#[cfg(feature = "bytecode-vm")]
pub(crate) fn foundation_bootstrap_callable_names() -> impl Iterator<Item = &'static str> {
    RUNTIME_CALLABLE_INVENTORY
        .iter()
        .copied()
        .chain(BYTECODE_BOOTSTRAP_ONLY_CALLABLES.iter().copied())
}

#[cfg(feature = "bytecode-vm")]
pub(crate) fn is_bytecode_callable(name: &str) -> bool {
    let local = name.rsplit_once('/').map_or(name, |(_, local)| local);
    direct_callable_spec(name).is_some()
        || direct_callable_spec(local).is_some()
        || BYTECODE_BOOTSTRAP_ONLY_CALLABLES.contains(&local)
        || Primitive::from_symbol(name).is_some()
        || named_predicate_protocol(local).is_some()
        || NATIVE_TYPES
            .iter()
            .any(|(_, methods)| methods.contains(&local))
        || protocol_declarations().iter().any(|declaration| {
            declaration
                .methods
                .iter()
                .any(|method| method.name == local)
        })
        || ["disj", "special-symbol?", "the-ns", "ns-name"].contains(&local)
}
