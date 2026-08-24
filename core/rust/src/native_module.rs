//! Runtime adapter for publication-linked `hara-abi` native modules.

use crate::core::{Promise, Value};
use hara_abi::{Error, NativeModule, TaskEvent};
use num_bigint::BigInt;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Registry {
    modules: Rc<RefCell<HashMap<String, Arc<dyn NativeModule>>>>,
}

impl Registry {
    pub fn install(&self, module: Arc<dyn NativeModule>) -> Result<(), String> {
        let service = module.identity().export.clone();
        let mut modules = self.modules.borrow_mut();
        if modules.contains_key(&service) {
            return Err(format!("native-module/duplicate: {service}"));
        }
        modules.insert(service, module);
        Ok(())
    }

    pub fn services(&self) -> Vec<String> {
        let mut services = self.modules.borrow().keys().cloned().collect::<Vec<_>>();
        services.sort();
        services
    }

    pub fn invoke(
        &self,
        service: String,
        operation: String,
        arguments: Vec<Value>,
    ) -> Result<Value, String> {
        let module = self
            .modules
            .borrow()
            .get(&service)
            .cloned()
            .ok_or_else(|| format!("native-module/unavailable: {service}"))?;
        if !module
            .operations()
            .iter()
            .any(|candidate| *candidate == operation)
        {
            return Err(format!(
                "native-module/operation-unknown: {service}/{operation}"
            ));
        }
        let arguments = arguments
            .iter()
            .map(to_abi)
            .collect::<Result<Vec<_>, _>>()?;
        let task = module.start(&operation, arguments).map_err(error_message)?;
        let promise = Promise::new();
        install_hooks(&promise, module, task);
        Ok(Value::Promise(promise))
    }
}

fn install_hooks(promise: &Promise, module: Arc<dyn NativeModule>, task: u64) {
    let destination = promise.clone();
    let polling = module.clone();
    promise.set_poller(Rc::new(move || {
        settle(&destination, polling.as_ref(), task, polling.poll(task));
    }));
    let destination = promise.clone();
    let waiting = module.clone();
    promise.set_waiter(Rc::new(move || {
        settle(
            &destination,
            waiting.as_ref(),
            task,
            waiting.wait(task, None),
        );
    }));
    promise.set_cancel_hook(Rc::new(move || {
        let _ = module.cancel(task);
        module.drop_task(task);
    }));
}

fn settle(
    promise: &Promise,
    module: &dyn NativeModule,
    task: u64,
    event: Result<TaskEvent, Error>,
) {
    match event {
        Ok(TaskEvent::Pending) => {}
        Ok(TaskEvent::Resolved(value)) => {
            match from_abi(value) {
                Ok(value) => {
                    promise.resolve(value);
                }
                Err(error) => {
                    promise.reject(error);
                }
            }
            module.drop_task(task);
        }
        Ok(TaskEvent::Rejected(error)) | Err(error) => {
            promise.reject(error_message(error));
            module.drop_task(task);
        }
    }
}

fn error_message(error: Error) -> String {
    format!("{}: {}", error.code, error.detail)
}

fn to_abi(value: &Value) -> Result<hara_abi::Value, String> {
    use hara_abi::Value as Abi;
    Ok(match value {
        Value::Nil => Abi::Nil,
        Value::Bool(value) => Abi::Boolean(*value),
        Value::Number(value) => Abi::Integer(*value),
        Value::BigInteger(value) => Abi::BigInteger(value.to_string()),
        Value::Float(value) => Abi::Float(crate::numeric::finite_float(*value)?),
        Value::String(value) => Abi::String(value.clone()),
        Value::Bytes(value) => Abi::Bytes(value.clone()),
        Value::ByteBuffer(value) => Abi::Bytes(value.borrow().clone()),
        Value::Keyword(value) => Abi::Keyword(value.as_str().into()),
        Value::Vector(values) => {
            Abi::Vector(values.iter().map(to_abi).collect::<Result<Vec<_>, _>>()?)
        }
        Value::Tuple(values) => {
            Abi::Vector(values.iter().map(to_abi).collect::<Result<Vec<_>, _>>()?)
        }
        Value::List(values) => {
            Abi::Vector(values.iter().map(to_abi).collect::<Result<Vec<_>, _>>()?)
        }
        Value::Map(values) => {
            let mut output = BTreeMap::new();
            for (key, value) in values.iter() {
                let key = match key {
                    Value::String(value) => value.clone(),
                    Value::Keyword(value) => value.as_str().into(),
                    _ => return Err("native-module/value-unsupported: record key".into()),
                };
                output.insert(key, to_abi(value)?);
            }
            Abi::Record(output)
        }
        _ => {
            return Err(format!(
                "native-module/value-unsupported: {}",
                value.display()
            ))
        }
    })
}

fn from_abi(value: hara_abi::Value) -> Result<Value, String> {
    use hara_abi::Value as Abi;
    Ok(match value {
        Abi::Nil => Value::Nil,
        Abi::Boolean(value) => Value::Bool(value),
        Abi::Integer(value) => Value::Number(value),
        Abi::BigInteger(value) => {
            let value = BigInt::parse_bytes(value.as_bytes(), 10)
                .ok_or_else(|| "native-module/value-invalid: big integer".to_string())?;
            crate::numeric::compact_integer(value)
        }
        Abi::Float(value) => Value::Float(crate::numeric::finite_float(value)?),
        Abi::String(value) => Value::String(value),
        Abi::Bytes(value) => Value::Bytes(value),
        Abi::Keyword(value) => Value::Keyword(value.into()),
        Abi::Vector(values) => Value::Vector(
            values
                .into_iter()
                .map(from_abi)
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        ),
        Abi::Record(values) => Value::Map(
            values
                .into_iter()
                .map(|(key, value)| Ok((Value::String(key), from_abi(value)?)))
                .collect::<Result<Vec<_>, String>>()?
                .into_iter()
                .collect(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hara_abi::{NativeIdentity, TaskId};
    use std::sync::Mutex;

    struct Echo {
        identity: NativeIdentity,
        result: Mutex<Option<hara_abi::Value>>,
    }

    impl NativeModule for Echo {
        fn identity(&self) -> &NativeIdentity {
            &self.identity
        }
        fn operations(&self) -> &[&str] {
            &["echo"]
        }
        fn capabilities(&self) -> &[&str] {
            &[]
        }
        fn start(
            &self,
            _operation: &str,
            mut arguments: Vec<hara_abi::Value>,
        ) -> Result<TaskId, Error> {
            *self.result.lock().unwrap() = arguments.pop();
            Ok(1)
        }
        fn poll(&self, _task: TaskId) -> Result<TaskEvent, Error> {
            Ok(self
                .result
                .lock()
                .unwrap()
                .take()
                .map(TaskEvent::Resolved)
                .unwrap_or(TaskEvent::Pending))
        }
        fn cancel(&self, _task: TaskId) -> Result<(), Error> {
            Ok(())
        }
        fn drop_task(&self, _task: TaskId) {}
        fn shutdown(&self) {}
    }

    #[test]
    fn registry_returns_runtime_promises_without_leaking_module_values() {
        let registry = Registry::default();
        registry
            .install(Arc::new(Echo {
                identity: NativeIdentity::new("gh:example:echo", "test.echo", "echo", "test/1")
                    .unwrap(),
                result: Mutex::new(None),
            }))
            .unwrap();
        let Value::Promise(promise) = registry
            .invoke(
                "test.echo".into(),
                "echo".into(),
                vec![Value::BigInteger(
                    BigInt::parse_bytes(b"9223372036854775808", 10).unwrap(),
                )],
            )
            .unwrap()
        else {
            panic!("promise")
        };
        assert!(matches!(
            promise.state(),
            crate::core::PromiseState::Fulfilled(Value::BigInteger(value))
                if value == BigInt::parse_bytes(b"9223372036854775808", 10).unwrap()
        ));
    }
}
