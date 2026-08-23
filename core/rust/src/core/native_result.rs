use super::{
    caught_error, map_entries, protocol_deref, protocol_deref_timeout, thrown_error, ExceptionInfo,
    PromiseRejection, PromiseState, Value,
};

fn native_equal(left: &Value, right: &Value) -> bool {
    left == right
}
use crate::lang::data::{Keyword, Map as PMap};
use crate::lang::hash::{self as jh, JavaHash};
use crate::lang::protocol::HashType;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResultStatus {
    Success,
    Error,
}

impl ResultStatus {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResultValue {
    pub status: ResultStatus,
    pub data: Value,
    pub error: Option<Rc<ExceptionInfo>>,
    pub context: Value,
}

impl ResultValue {
    pub fn success(data: Value, context: Value) -> Result<Self, String> {
        Ok(Self {
            status: ResultStatus::Success,
            data,
            error: None,
            context: validate_context(context)?,
        })
    }

    pub fn error(error: Value, context: Value) -> Result<Self, String> {
        Ok(Self {
            status: ResultStatus::Error,
            data: Value::Nil,
            error: Some(normalize_error(error)),
            context: validate_context(context)?,
        })
    }

    pub fn status_value(&self) -> Value {
        Value::Keyword(Keyword::from(self.status.keyword()))
    }

    pub fn error_value(&self) -> Value {
        self.error
            .as_ref()
            .map(|error| Value::ExceptionInfo(error.clone()))
            .unwrap_or(Value::Nil)
    }

    pub fn is_success(&self) -> bool {
        self.status == ResultStatus::Success
    }

    pub fn is_error(&self) -> bool {
        self.status == ResultStatus::Error
    }

    pub fn is_timeout(&self) -> bool {
        if !self.is_error() {
            return false;
        }
        let Some(error) = &self.error else {
            return false;
        };
        let code_key = Value::Keyword(Keyword::from("code"));
        map_entries(error.data.as_ref()).is_some_and(|entries| {
            entries.into_iter().any(|(key, value)| {
                key == code_key
                    && matches!(
                        value,
                        Value::Keyword(code) if code.as_str() == "result/timeout"
                    )
            })
        })
    }

    pub fn with_context(&self, additional: Value) -> Result<Self, String> {
        let additional = validate_context(additional)?;
        let mut merged = PMap::new();
        for (key, value) in map_entries(&self.context)
            .expect("validated Result context")
            .into_iter()
            .chain(
                map_entries(&additional)
                    .expect("validated additional Result context")
                    .into_iter(),
            )
        {
            merged = merged.assoc_value(key, value);
        }
        let mut updated = self.clone();
        updated.context = Value::Map(merged);
        Ok(updated)
    }

    pub(crate) fn transport_context(&self) -> Value {
        let display = Value::Keyword(Keyword::from("display"));
        Value::Map(PMap::from_iter(
            map_entries(&self.context)
                .expect("validated Result context")
                .into_iter()
                .filter(|(key, _)| key != &display),
        ))
    }

    pub(crate) fn deref_value(&self) -> Result<Value, String> {
        match self.status {
            ResultStatus::Success => Ok(self.data.clone()),
            ResultStatus::Error => self
                .error
                .as_ref()
                .map(|error| Err(thrown_error(Value::ExceptionInfo(error.clone()))))
                .unwrap_or_else(|| Err("invalid Result/error without a native Error".into())),
        }
    }

    pub fn display(&self) -> String {
        format!(
            "#hara/Result[{} {} {} {}]",
            self.status_value().display(),
            self.data.display(),
            self.error_value().display(),
            self.context.display()
        )
    }

    pub fn compare(&self, other: &Self) -> Ordering {
        self.status
            .cmp(&other.status)
            .then_with(|| self.data.cmp(&other.data))
            .then_with(|| compare_error(self.error.as_deref(), other.error.as_deref()))
    }

    pub fn java_hash(&self, hash_type: HashType) -> i64 {
        jh::compose_ordered(
            "RESULT",
            [
                match self.status {
                    ResultStatus::Success => 1,
                    ResultStatus::Error => 2,
                },
                self.data.java_hash(hash_type),
                self.error
                    .as_deref()
                    .map_or(0, |error| error_hash(error, hash_type)),
            ],
        )
    }
}

const DEREF_UNSUPPORTED: &str = "IDeref/deref has no implementation for this value";
const DEREF_TIMEOUT_UNSUPPORTED: &str =
    "IDerefTimeout/deref-timeout expects a dereferenceable value, milliseconds, and timeout value";

pub(super) fn synchronize_value(
    value: Value,
    timeout: Option<u64>,
    context: Value,
) -> Result<Value, String> {
    let context = validate_context(context)?;
    if let Value::Result(result) = value {
        if map_entries(&context)
            .expect("validated Result context")
            .is_empty()
        {
            return Ok(Value::Result(result));
        }
        return Ok(Value::Result(Rc::new(result.with_context(context)?)));
    }

    let result = match value {
        Value::Promise(promise) => synchronize_promise(promise, timeout, context)?,
        value => match timeout {
            Some(milliseconds) => synchronize_timed(value, milliseconds, context)?,
            None => synchronize_untimed(value, context)?,
        },
    };
    Ok(Value::Result(Rc::new(result)))
}

fn synchronize_untimed(value: Value, context: Value) -> Result<ResultValue, String> {
    match protocol_deref(std::slice::from_ref(&value)) {
        Ok(data) => ResultValue::success(data, context),
        Err(error) if error == DEREF_UNSUPPORTED => ResultValue::success(value, context),
        Err(error) => ResultValue::error(caught_error(&error), context),
    }
}

fn synchronize_timed(
    value: Value,
    milliseconds: u64,
    context: Value,
) -> Result<ResultValue, String> {
    let marker = Value::Array(Rc::new(RefCell::new(Vec::new())));
    let milliseconds_value = Value::Number(i64::try_from(milliseconds).unwrap_or(i64::MAX));
    match protocol_deref_timeout(&[value.clone(), milliseconds_value, marker.clone()]) {
        Ok(resolved) if same_marker(&resolved, &marker) => {
            timeout_result(milliseconds, context, None)
        }
        Ok(data) => ResultValue::success(data, context),
        Err(error) if error == DEREF_TIMEOUT_UNSUPPORTED => {
            if matches!(value, Value::Pointer(_)) {
                timeout_unsupported_result(milliseconds, context)
            } else {
                ResultValue::success(value, context)
            }
        }
        Err(error) => ResultValue::error(caught_error(&error), context),
    }
}

fn synchronize_promise(
    promise: super::Promise,
    timeout: Option<u64>,
    context: Value,
) -> Result<ResultValue, String> {
    let state = match timeout {
        Some(milliseconds) => promise.wait_state_timeout(Duration::from_millis(milliseconds)),
        None => promise.wait_state(),
    };
    match state {
        PromiseState::Fulfilled(data) => ResultValue::success(data, context),
        PromiseState::Rejected(error) => {
            ResultValue::error(promise_rejection_value(error), context)
        }
        PromiseState::Pending => timeout_result(
            timeout.expect("only timed Promise synchronization can remain pending"),
            context,
            Some(promise),
        ),
    }
}

fn promise_rejection_value(error: PromiseRejection) -> Value {
    error.value()
}

fn timeout_result(
    milliseconds: u64,
    context: Value,
    promise: Option<super::Promise>,
) -> Result<ResultValue, String> {
    let mut details = vec![
        (
            Value::Keyword(Keyword::from("result/timeout")),
            Value::Number(i64::try_from(milliseconds).unwrap_or(i64::MAX)),
        ),
        (
            Value::Keyword(Keyword::from("result/cancellation-requested")),
            Value::Bool(promise.is_some()),
        ),
    ];

    if let Some(promise) = promise {
        match catch_unwind(AssertUnwindSafe(|| promise.cancel())) {
            Ok(cancelled) => details.push((
                Value::Keyword(Keyword::from("result/cancelled")),
                Value::Bool(cancelled),
            )),
            Err(payload) => {
                details.push((
                    Value::Keyword(Keyword::from("result/cancelled")),
                    Value::Bool(false),
                ));
                details.push((
                    Value::Keyword(Keyword::from("result/cancellation-error")),
                    Value::String(panic_message(payload)),
                ));
            }
        }
    }

    ResultValue::error(
        result_error(
            "result/timeout",
            "Result synchronization timed out",
            milliseconds,
        ),
        context_with(context, details),
    )
}

fn timeout_unsupported_result(milliseconds: u64, context: Value) -> Result<ResultValue, String> {
    ResultValue::error(
        result_error(
            "result/timeout-unsupported",
            "Timed synchronization is unsupported for this dereferenceable value",
            milliseconds,
        ),
        context_with(
            context,
            [(
                Value::Keyword(Keyword::from("result/timeout")),
                Value::Number(i64::try_from(milliseconds).unwrap_or(i64::MAX)),
            )],
        ),
    )
}

fn result_error(code: &str, message: &str, milliseconds: u64) -> Value {
    Value::ExceptionInfo(Rc::new(ExceptionInfo {
        message: message.into(),
        data: Box::new(Value::Map(PMap::from_iter([
            (
                Value::Keyword(Keyword::from("code")),
                Value::Keyword(Keyword::from(code)),
            ),
            (
                Value::Keyword(Keyword::from("message")),
                Value::String(message.into()),
            ),
            (
                Value::Keyword(Keyword::from("timeout")),
                Value::Number(i64::try_from(milliseconds).unwrap_or(i64::MAX)),
            ),
        ]))),
        cause: None,
        provenance: Rc::new(RefCell::new(Default::default())),
    }))
}

fn context_with(context: Value, entries: impl IntoIterator<Item = (Value, Value)>) -> Value {
    let mut merged = PMap::new();
    for (key, value) in map_entries(&context).expect("validated Result context") {
        merged = merged.assoc_value(key, value);
    }
    for (key, value) in entries {
        merged = merged.assoc_value(key, value);
    }
    Value::Map(merged)
}

fn same_marker(left: &Value, right: &Value) -> bool {
    matches!(
        (left, right),
        (Value::Array(left), Value::Array(right)) if Rc::ptr_eq(left, right)
    )
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Promise cancellation panicked".into())
}

impl PartialEq for ResultValue {
    fn eq(&self, other: &Self) -> bool {
        self.status == other.status
            && native_equal(&self.data, &other.data)
            && error_equal(self.error.as_deref(), other.error.as_deref())
    }
}

impl Eq for ResultValue {}

fn validate_context(context: Value) -> Result<Value, String> {
    map_entries(&context)
        .is_some()
        .then_some(context)
        .ok_or_else(|| "Result context must be a map".into())
}

fn normalize_error(value: Value) -> Rc<ExceptionInfo> {
    match value {
        Value::ExceptionInfo(error) => error,
        value => {
            let message = match &value {
                Value::String(text) => text.clone(),
                _ => value.display(),
            };
            Rc::new(ExceptionInfo {
                message,
                data: Box::new(Value::Map(PMap::from_iter([(
                    Value::Keyword(Keyword::from("error/value")),
                    value,
                )]))),
                cause: None,
                provenance: Rc::new(RefCell::new(Default::default())),
            })
        }
    }
}

fn error_equal(left: Option<&ExceptionInfo>, right: Option<&ExceptionInfo>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.message == right.message
                && native_equal(&left.data, &right.data)
                && match (&left.cause, &right.cause) {
                    (None, None) => true,
                    (Some(left), Some(right)) => native_equal(left, right),
                    _ => false,
                }
        }
        _ => false,
    }
}

fn compare_error(left: Option<&ExceptionInfo>, right: Option<&ExceptionInfo>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(left), Some(right)) => left
            .message
            .cmp(&right.message)
            .then_with(|| left.data.cmp(&right.data))
            .then_with(|| left.cause.cmp(&right.cause)),
    }
}

fn error_hash(error: &ExceptionInfo, hash_type: HashType) -> i64 {
    jh::compose_ordered(
        "RESULT_ERROR",
        [
            jh::java_string_hash("hara/Error") as i64,
            jh::java_string_hash(&error.message) as i64,
            error.data.java_hash(hash_type),
            error
                .cause
                .as_deref()
                .map_or(0, |cause| cause.java_hash(hash_type)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(key: &str, value: Value) -> Value {
        Value::Map(PMap::from_iter([(
            Value::Keyword(Keyword::from(key)),
            value,
        )]))
    }

    #[test]
    fn native_result_equality_and_hash_ignore_context() {
        let left = ResultValue::success(
            Value::Number(42),
            context("source", Value::String("left".into())),
        )
        .expect("left Result");
        let right = ResultValue::success(
            Value::Number(42),
            context("source", Value::String("right".into())),
        )
        .expect("right Result");
        assert_eq!(left, right);
        assert_eq!(
            left.java_hash(crate::lang::hash::DEFAULT_HASH),
            right.java_hash(crate::lang::hash::DEFAULT_HASH)
        );
        assert_eq!(
            left.deref_value().expect("success deref"),
            Value::Number(42)
        );
    }

    #[test]
    fn native_result_context_merge_uses_supplied_keys() {
        let result = ResultValue::success(
            Value::Number(7),
            Value::Map(PMap::from_iter([
                (
                    Value::Keyword(Keyword::from("source")),
                    Value::String("left".into()),
                ),
                (Value::Keyword(Keyword::from("kept")), Value::Bool(true)),
            ])),
        )
        .expect("Result");
        let updated = result
            .with_context(Value::Map(PMap::from_iter([
                (
                    Value::Keyword(Keyword::from("source")),
                    Value::String("right".into()),
                ),
                (Value::Keyword(Keyword::from("added")), Value::Number(1)),
            ])))
            .expect("merged Result");
        let source =
            super::super::map_value(&updated.context, &Value::Keyword(Keyword::from("source")))
                .expect("source context");
        assert!(matches!(source, Value::String(value) if value.as_str() == "right"));
        assert_eq!(result, updated);
    }

    #[test]
    fn synchronize_raw_existing_and_nested_results() {
        let raw = synchronize_value(Value::Number(42), None, Value::Map(PMap::new()))
            .expect("raw synchronization");
        let Value::Result(raw) = raw else {
            panic!("expected Result");
        };
        assert!(raw.is_success());
        assert_eq!(raw.data, Value::Number(42));

        let existing = Rc::new(
            ResultValue::success(
                Value::Number(7),
                context("source", Value::String("left".into())),
            )
            .expect("existing Result"),
        );
        let synchronized = synchronize_value(
            Value::Result(existing.clone()),
            None,
            context("source", Value::String("right".into())),
        )
        .expect("existing synchronization");
        let Value::Result(synchronized) = synchronized else {
            panic!("expected Result");
        };
        assert_eq!(synchronized.as_ref(), existing.as_ref());
        let source = super::super::map_value(
            &synchronized.context,
            &Value::Keyword(Keyword::from("source")),
        )
        .expect("source context");
        assert!(matches!(source, Value::String(value) if value == "right"));

        let promise = super::super::Promise::new();
        promise.resolve(Value::Result(existing.clone()));
        let wrapped = synchronize_value(Value::Promise(promise), None, Value::Map(PMap::new()))
            .expect("nested synchronization");
        let Value::Result(wrapped) = wrapped else {
            panic!("expected Result");
        };
        assert!(matches!(
            &wrapped.data,
            Value::Result(value) if Rc::ptr_eq(value, &existing)
        ));
    }

    #[test]
    fn synchronize_captures_rejection_timeout_and_cancellation_failure() {
        let error = Rc::new(ExceptionInfo {
            message: "rejected".into(),
            data: Box::new(context("code", Value::Keyword(Keyword::from("rejected")))),
            cause: None,
            provenance: Rc::new(RefCell::new(Default::default())),
        });
        let rejected = super::super::Promise::new();
        rejected.reject_value(Value::ExceptionInfo(error.clone()));
        let captured = synchronize_value(Value::Promise(rejected), None, Value::Map(PMap::new()))
            .expect("rejection synchronization");
        let Value::Result(captured) = captured else {
            panic!("expected Result");
        };
        assert!(captured.is_error());
        assert!(!captured.is_timeout());
        assert!(matches!(
            captured.error_value(),
            Value::ExceptionInfo(value) if Rc::ptr_eq(&value, &error)
        ));

        let timed = super::super::Promise::new();
        let timeout = synchronize_value(
            Value::Promise(timed.clone()),
            Some(0),
            Value::Map(PMap::new()),
        )
        .expect("timeout synchronization");
        let Value::Result(timeout) = timeout else {
            panic!("expected Result");
        };
        assert!(timeout.is_error());
        assert!(timeout.is_timeout());
        let Value::ExceptionInfo(timeout_error) = timeout.error_value() else {
            panic!("expected timeout Error");
        };
        let code = super::super::map_value(
            timeout_error.data.as_ref(),
            &Value::Keyword(Keyword::from("code")),
        )
        .expect("timeout code");
        assert_eq!(code, &Value::Keyword(Keyword::from("result/timeout")));
        assert!(matches!(timed.state(), PromiseState::Rejected(_)));

        let cancellation_failure = super::super::Promise::new();
        cancellation_failure.set_cancel_hook(Rc::new(|| panic!("cannot cancel")));
        let timeout = synchronize_value(
            Value::Promise(cancellation_failure),
            Some(0),
            Value::Map(PMap::new()),
        )
        .expect("cancellation failure synchronization");
        let Value::Result(timeout) = timeout else {
            panic!("expected Result");
        };
        assert!(super::super::map_value(
            &timeout.context,
            &Value::Keyword(Keyword::from("result/cancellation-error")),
        )
        .is_some());
    }

    #[test]
    fn native_result_error_preserves_native_error_and_deref_throws() {
        let error = Rc::new(ExceptionInfo {
            message: "boom".into(),
            data: Box::new(context("code", Value::Keyword(Keyword::from("boom")))),
            cause: None,
            provenance: Rc::new(RefCell::new(Default::default())),
        });
        let result =
            ResultValue::error(Value::ExceptionInfo(error.clone()), Value::Map(PMap::new()))
                .expect("error Result");
        assert!(result.is_error());
        let preserved = match result.error_value() {
            Value::ExceptionInfo(preserved) => preserved,
            other => panic!("expected native Error, got {}", other.display()),
        };
        assert_eq!(preserved.message, error.message);
        assert_eq!(preserved.data.display(), error.data.display());
        assert!(result.deref_value().is_err());
        assert!(result.display().contains("#hara/Result[:error"));
    }

    #[test]
    fn native_result_string_errors_keep_unquoted_messages() {
        let result = ResultValue::error(Value::String("boom".into()), Value::Map(PMap::new()))
            .expect("error Result");
        let Value::ExceptionInfo(error) = result.error_value() else {
            panic!("expected native Error");
        };
        assert_eq!(error.message, "boom");
    }
}
