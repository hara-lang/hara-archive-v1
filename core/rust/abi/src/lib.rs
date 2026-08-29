//! Dependency-free values and identities shared across Hara ABI boundaries.

use std::collections::BTreeMap;
use std::time::Duration;

pub const HTA_V1: &str = "hta.v1";

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Boolean(bool),
    String(String),
    Integer(i64),
    BigInteger(String),
    Float(f64),
    Bytes(Vec<u8>),
    Keyword(String),
    Vector(Vec<Value>),
    Record(RecordValue),
}

pub type RecordValue = BTreeMap<String, Value>;

/// The complete immutable Hara data profile used by HTA and durable stores.
///
/// This is intentionally separate from [`Value`], whose closed shape is the
/// stable ABI accepted by existing native provider crates.
#[derive(Clone, Debug, PartialEq)]
pub enum ImmutableValue {
    Nil,
    Boolean(bool),
    String(String),
    Integer(i64),
    Float(f64),
    Character(char),
    BigInteger(String),
    Regex(String),
    Bytes(Vec<u8>),
    Keyword(String),
    Symbol(String),
    List(Vec<ImmutableValue>),
    Vector(Vec<ImmutableValue>),
    /// A dedicated two-value entry produced by map and lookup operations.
    MapEntry(Vec<ImmutableValue>),
    /// Legacy compact tuple representation retained for decoding old HTA data.
    Tuple(Vec<ImmutableValue>),
    Cons(Vec<ImmutableValue>),
    Queue(Vec<ImmutableValue>),
    Set(Vec<ImmutableValue>),
    OrderedSet(Vec<ImmutableValue>),
    SortedSet(Vec<ImmutableValue>),
    Map(Vec<(ImmutableValue, ImmutableValue)>),
    OrderedMap(Vec<(ImmutableValue, ImmutableValue)>),
    SortedMap(Vec<(ImmutableValue, ImmutableValue)>),
    Trie(Vec<(String, ImmutableValue)>),
    Record(ImmutableRecordValue),
    Tagged {
        tag: String,
        form: Box<ImmutableValue>,
    },
    ExceptionInfo {
        message: String,
        data: Box<ImmutableValue>,
        cause: Option<Box<ImmutableValue>>,
        provenance: ExceptionProvenance,
    },
    Struct {
        name: String,
        fields: Vec<String>,
        values: Vec<ImmutableValue>,
    },
    Pointer {
        context: String,
        fields: ImmutableRecordValue,
    },
    /// A qualified binding identity. The bound value is never transferred.
    VarRef(String),
}

pub type ImmutableRecordValue = BTreeMap<String, ImmutableValue>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionSite {
    pub namespace: Option<String>,
    pub resource: Option<String>,
    pub line: u64,
    pub column: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExceptionProvenance {
    pub created_at: Option<ExceptionSite>,
    pub throws: Vec<ExceptionSite>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: String,
    pub detail: String,
}

impl Error {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
        }
    }
}

/// Opaque task identifier owned by one linked native module.
pub type TaskId = u64;

/// Observable state of an asynchronous native module call.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskEvent {
    Pending,
    Resolved(Value),
    Rejected(Error),
}

/// Dependency-free contract implemented by publication-linked native crates.
pub trait NativeModule: Send + Sync {
    fn identity(&self) -> &NativeIdentity;
    fn operations(&self) -> &[&str];
    fn capabilities(&self) -> &[&str];
    fn start(&self, operation: &str, arguments: Vec<Value>) -> Result<TaskId, Error>;
    fn poll(&self, task: TaskId) -> Result<TaskEvent, Error>;

    fn wait(&self, task: TaskId, timeout: Option<Duration>) -> Result<TaskEvent, Error> {
        let _ = timeout;
        self.poll(task)
    }

    fn cancel(&self, task: TaskId) -> Result<(), Error>;
    fn drop_task(&self, task: TaskId);
    fn shutdown(&self);
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NativeIdentity {
    pub package: String,
    pub export: String,
    pub crate_name: String,
    pub abi: String,
}

impl NativeIdentity {
    pub fn new(
        package: impl Into<String>,
        export: impl Into<String>,
        crate_name: impl Into<String>,
        abi: impl Into<String>,
    ) -> Result<Self, Error> {
        let identity = Self {
            package: package.into(),
            export: export.into(),
            crate_name: crate_name.into(),
            abi: abi.into(),
        };
        for (label, value) in [
            ("package", identity.package.as_str()),
            ("export", identity.export.as_str()),
            ("crate", identity.crate_name.as_str()),
            ("abi", identity.abi.as_str()),
        ] {
            if value.is_empty() || value.chars().any(char::is_whitespace) {
                return Err(Error::new(
                    "native-identity-invalid",
                    format!("{label} must be a non-empty identifier"),
                ));
            }
        }
        Ok(identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_identities_are_exact_and_portable() {
        let identity = NativeIdentity::new(
            "gh:greenways-ai:hoplite-store-sqlite",
            "hoplite/store",
            "hoplite-store-sqlite",
            "hoplite-auth-store/0-alpha",
        )
        .unwrap();
        assert_eq!(identity.crate_name, "hoplite-store-sqlite");
        assert_eq!(
            NativeIdentity::new("", "hoplite/store", "crate", "abi")
                .unwrap_err()
                .code,
            "native-identity-invalid"
        );
    }

    #[test]
    fn abi_values_cover_portable_database_payloads() {
        let value = Value::Record(BTreeMap::from([
            ("ok".into(), Value::Boolean(true)),
            (
                "rows".into(),
                Value::Vector(vec![Value::Vector(vec![Value::Integer(1), Value::Nil])]),
            ),
            (
                "big".into(),
                Value::BigInteger("9223372036854775808".into()),
            ),
            ("numeric".into(), Value::Float(12.50)),
        ]));
        assert!(matches!(value, Value::Record(_)));
    }
}
