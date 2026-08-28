//! Binary-safe invocation of already-loaded, fully qualified Hara Vars.
//!
//! This boundary deliberately does not parse, compile, macroexpand, load, or
//! evaluate source text. Embedding hosts remain responsible for a closed Var
//! allowlist before calling it.

use crate::core::{self, PromiseState, Value};
use crate::lang::data::Symbol;
use crate::{hta, Runtime};
use std::error::Error;
use std::fmt;
use std::rc::Rc;

pub const MAX_INVOKE_HTA_RESULT_BYTES: usize = 256 * 1024;
const MAX_PROMISE_UNWRAP_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeHtaError {
    InvalidQualifiedVar,
    MalformedInput(String),
    NoncanonicalInput,
    ArgumentsNotVector,
    NamespaceMissing(String),
    VarMissing(String),
    VarNotCallable(String),
    Execution(String),
    PromiseRejected(String),
    PromisePending,
    PromiseDepthExceeded,
    UnsupportedResult(String),
    ResultTooLarge { actual: usize, maximum: usize },
    SessionMissing(String),
    BrokerClosed,
    BrokerStopped,
}

impl InvokeHtaError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidQualifiedVar => "invoke-hta/qualified-var-invalid",
            Self::MalformedInput(_) => "invoke-hta/input-malformed",
            Self::NoncanonicalInput => "invoke-hta/input-noncanonical",
            Self::ArgumentsNotVector => "invoke-hta/arguments-not-vector",
            Self::NamespaceMissing(_) => "invoke-hta/namespace-missing",
            Self::VarMissing(_) => "invoke-hta/var-missing",
            Self::VarNotCallable(_) => "invoke-hta/var-not-callable",
            Self::Execution(_) => "invoke-hta/execution-failed",
            Self::PromiseRejected(_) => "invoke-hta/promise-rejected",
            Self::PromisePending => "invoke-hta/promise-pending",
            Self::PromiseDepthExceeded => "invoke-hta/promise-depth-exceeded",
            Self::UnsupportedResult(_) => "invoke-hta/result-unsupported",
            Self::ResultTooLarge { .. } => "invoke-hta/result-too-large",
            Self::SessionMissing(_) => "invoke-hta/session-missing",
            Self::BrokerClosed => "invoke-hta/broker-closed",
            Self::BrokerStopped => "invoke-hta/broker-stopped",
        }
    }
}

impl fmt::Display for InvokeHtaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code())?;
        match self {
            Self::MalformedInput(detail)
            | Self::Execution(detail)
            | Self::PromiseRejected(detail)
            | Self::UnsupportedResult(detail) => write!(formatter, ": {detail}"),
            Self::NamespaceMissing(namespace) => write!(formatter, ": {namespace}"),
            Self::VarMissing(path) | Self::VarNotCallable(path) => {
                write!(formatter, ": {path}")
            }
            Self::ResultTooLarge { actual, maximum } => {
                write!(formatter, ": {actual} exceeds {maximum} bytes")
            }
            Self::SessionMissing(session) => write!(formatter, ": {session}"),
            _ => Ok(()),
        }
    }
}

impl Error for InvokeHtaError {}

impl Runtime {
    pub fn invoke_hta(
        &mut self,
        qualified_var: &str,
        arguments_hta: &[u8],
    ) -> Result<Vec<u8>, InvokeHtaError> {
        let (namespace_name, var_name) = split_qualified_var(qualified_var)?;
        let decoded = match hta::decode_canonical(arguments_hta) {
            Ok(value) => value,
            Err(error) if error.starts_with("hta/value-noncanonical:") => {
                return Err(InvokeHtaError::NoncanonicalInput)
            }
            Err(error) => return Err(InvokeHtaError::MalformedInput(error)),
        };
        let arguments = match decoded {
            Value::Vector(values) => values.iter().cloned().collect::<Vec<_>>(),
            _ => return Err(InvokeHtaError::ArgumentsNotVector),
        };

        let namespace = self
            .namespace_registry
            .find(namespace_name)
            .ok_or_else(|| InvokeHtaError::NamespaceMissing(namespace_name.to_owned()))?;
        let symbol = Symbol::parse(var_name);
        let var = namespace
            .resolve(&symbol)
            .ok_or_else(|| InvokeHtaError::VarMissing(qualified_var.to_owned()))?;
        let function = match var.deref_value() {
            Value::Function(function) => function,
            _ => return Err(InvokeHtaError::VarNotCallable(qualified_var.to_owned())),
        };

        let result = self
            .invoke_loaded_function(function, arguments)
            .map_err(InvokeHtaError::Execution)?;
        encode_result(settle_result(result)?)
    }

    fn invoke_loaded_function(
        &mut self,
        function: Rc<core::Function>,
        arguments: Vec<Value>,
    ) -> Result<Value, String> {
        let namespace_source = self.namespace_source();
        core::with_capability_providers(
            self.providers.file(),
            self.providers.socket(),
            self.providers.process(),
            self.providers.kernel(),
            || {
                core::with_package_catalog(&self.package_catalog, || {
                    core::with_promise_provider(self.providers.promise(), || {
                        core::with_macros(self.macros.clone(), || {
                            core::with_namespace_registry(&self.namespace_registry, || {
                                core::with_namespace_source(namespace_source, || {
                                    core::with_protocols(&self.protocols, || {
                                        if let Some(handler) = &self.native_host_handler {
                                            return core::with_host_calls(handler.clone(), || {
                                                core::invoke_function_sync(function, arguments)
                                            });
                                        }
                                        core::invoke_function_sync(function, arguments)
                                    })
                                })
                            })
                        })
                    })
                })
            },
        )
    }
}

fn split_qualified_var(value: &str) -> Result<(&str, &str), InvokeHtaError> {
    let Some((namespace, name)) = value.split_once('/') else {
        return Err(InvokeHtaError::InvalidQualifiedVar);
    };
    if namespace.is_empty()
        || name.is_empty()
        || name.contains('/')
        || value.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(InvokeHtaError::InvalidQualifiedVar);
    }
    Ok((namespace, name))
}

fn settle_result(mut value: Value) -> Result<Value, InvokeHtaError> {
    for _ in 0..MAX_PROMISE_UNWRAP_DEPTH {
        let Value::Promise(promise) = value else {
            return Ok(value);
        };
        value = match promise.wait_state() {
            PromiseState::Fulfilled(value) => value,
            PromiseState::Rejected(error) => {
                return Err(InvokeHtaError::PromiseRejected(error.message().to_owned()))
            }
            PromiseState::Pending => return Err(InvokeHtaError::PromisePending),
        };
    }
    Err(InvokeHtaError::PromiseDepthExceeded)
}

fn encode_result(result: Value) -> Result<Vec<u8>, InvokeHtaError> {
    let encoded = hta::encode(&result).map_err(InvokeHtaError::UnsupportedResult)?;
    if encoded.len() > MAX_INVOKE_HTA_RESULT_BYTES {
        return Err(InvokeHtaError::ResultTooLarge {
            actual: encoded.len(),
            maximum: MAX_INVOKE_HTA_RESULT_BYTES,
        });
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fulfilled_and_rejected_promises_are_typed() {
        let fulfilled = core::Promise::new();
        assert!(fulfilled.resolve(Value::Number(42)));
        assert_eq!(
            settle_result(Value::Promise(fulfilled)),
            Ok(Value::Number(42))
        );

        let rejected = core::Promise::new();
        assert!(rejected.reject("no"));
        assert_eq!(
            settle_result(Value::Promise(rejected)),
            Err(InvokeHtaError::PromiseRejected("no".to_owned()))
        );
    }

    #[test]
    fn encoded_results_are_bounded() {
        let result = Value::String("x".repeat(MAX_INVOKE_HTA_RESULT_BYTES));
        assert!(matches!(
            encode_result(result),
            Err(InvokeHtaError::ResultTooLarge {
                maximum: MAX_INVOKE_HTA_RESULT_BYTES,
                ..
            })
        ));
    }
}
