#[cfg(test)]
use crate::core::Value;
use crate::{vm, Runtime};

pub(in crate::task::production) fn validate_bundle(
    bytes: &[u8],
    entrypoints: &[String],
) -> Result<Runtime, String> {
    let mut runtime = Runtime::core();
    vm::eval_bytecode_bundle(&mut runtime, bytes)?;
    for entrypoint in entrypoints {
        prepare_entrypoint(&runtime, entrypoint)?;
    }
    Ok(runtime)
}

#[cfg(test)]
pub(super) fn invoke_zero_arity(runtime: &Runtime, symbol: &str) -> Result<Value, String> {
    prepare_entrypoint(runtime, symbol)?.invoke(Vec::new())
}

fn prepare_entrypoint(runtime: &Runtime, symbol: &str) -> Result<vm::PreparedCall, String> {
    vm::prepare_call(&runtime.namespace_registry, symbol, 0).map_err(|error| {
        if error.contains("not callable") {
            format!("production entrypoint is not invokable: {symbol}")
        } else {
            format!("production entrypoint is missing: {symbol}: {error}")
        }
    })
}
