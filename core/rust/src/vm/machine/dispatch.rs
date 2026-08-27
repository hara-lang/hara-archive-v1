//! Single-instruction VM dispatch.
//!
//! The outer `Machine::run` loop remains in `machine.rs`; this child only
//! decodes and executes one validated instruction and returns the next
//! control action.

use super::*;

/// Result of executing one instruction. Call actions only carry their
/// collected operands: the nested machine runs from the thin `run` loop
/// after the fat `dispatch` frame has exited, keeping the native stack
/// cost per guest call level small (issue #223).
pub(super) enum Dispatch {
    Next(usize),
    Unwound(usize),
    Call {
        callee: VmSlot,
        args: Vec<VmSlot>,
    },
    CallStatic {
        prototype: u16,
        args: Vec<VmSlot>,
        captures: Vec<VmSlot>,
    },
    CallStaticDirect {
        prototype: u16,
        argc: u8,
    },
    Returned(VmSlot),
    Failed(VmError),
    Suspended(Promise),
    Yielded(Value),
}

impl Machine {
    /// Executes one instruction, returning where the `run` loop
    /// continues. Call instructions only collect their operands into a
    /// [`Dispatch`] action: the actual call happens in `run` after this
    /// frame has exited. The hot dispatch is inlined into the run loop now
    /// that guest calls no longer recurse through the native stack.
    #[inline(always)]
    pub(super) fn dispatch(
        &mut self,
        program: &Rc<Program>,
        function: &FunctionPrototype,
        instruction: &Instruction,
    ) -> Dispatch {
        macro_rules! guarded {
            ($expr:expr) => {
                match $expr {
                    Ok(()) => {}
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            };
        }
        let mut next_ip = self.ip + 1;
        match instruction {
            Instruction::Constant(index) => {
                let Some(value) = program.constants.get(*index as usize) else {
                    match self.raise(function, format!("constant index {index} out of range")) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                };
                self.stack.push(value.clone().into());
            }
            Instruction::Nil => self.stack.push(VmSlot::Nil),
            Instruction::True => self.stack.push(VmSlot::Bool(true)),
            Instruction::False => self.stack.push(VmSlot::Bool(false)),
            Instruction::LoadLocal(slot) => {
                let Some(value) = self.frame.local(*slot) else {
                    match self.raise(function, format!("local slot {slot} out of range")) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                };
                self.stack.push(value.clone());
            }
            Instruction::StoreLocal(slot) => {
                let Some(value) = self.stack.pop() else {
                    match self.raise(function, "stack underflow") {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                };
                if !self.frame.store(*slot, value) {
                    match self.raise(function, format!("local slot {slot} out of range")) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                }
            }
            Instruction::Pop => {
                if self.stack.pop().is_none() {
                    match self.raise(function, "stack underflow") {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                }
            }
            Instruction::Dup => {
                let Some(value) = self.stack.last().cloned() else {
                    return Dispatch::Failed(self.error(function, "stack underflow"));
                };
                self.stack.push(value);
            }
            Instruction::IntrinsicCall { target, argc }
            | Instruction::ProtocolCall { target, argc } => {
                let Some(name) = constant_string(program, *target) else {
                    return Dispatch::Failed(self.error(
                        function,
                        format!("intrinsic target constant {target} is invalid"),
                    ));
                };
                let argc = usize::from(*argc);
                if self.stack.len() < argc {
                    match self.raise(function, "stack underflow") {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                }
                #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
                if matches!(instruction, Instruction::ProtocolCall { .. })
                    || crate::core::IntrinsicOp::from_symbol(name).is_some()
                {
                    crate::direct_native::record_native_target(name);
                }
                if matches!(instruction, Instruction::ProtocolCall { .. })
                    && name == "std.protocol.ideref.IDeref/deref"
                    && argc == 1
                {
                    if let Some(Value::Promise(promise)) =
                        self.stack.last().and_then(VmSlot::runtime_value)
                    {
                        if matches!(promise.state(), PromiseState::Pending) {
                            return Dispatch::Suspended(promise);
                        }
                    }
                }
                let result = if name == "=" && argc == 2 {
                    let right = self.stack.pop().expect("intrinsic arity checked above");
                    let left = self.stack.pop().expect("intrinsic arity checked above");
                    match (left, right) {
                        (VmSlot::Closure(left), VmSlot::Closure(right)) => {
                            Ok(VmSlot::Bool(Rc::ptr_eq(&left, &right)))
                        }
                        (
                            VmSlot::InlineClosure { identity: left, .. },
                            VmSlot::InlineClosure {
                                identity: right, ..
                            },
                        ) => Ok(VmSlot::Bool(left == right)),
                        (VmSlot::MultiArity(left), VmSlot::MultiArity(right)) => {
                            Ok(VmSlot::Bool(Rc::ptr_eq(&left, &right)))
                        }
                        (
                            VmSlot::Closure(_)
                            | VmSlot::InlineClosure { .. }
                            | VmSlot::MultiArity(_),
                            _,
                        )
                        | (
                            _,
                            VmSlot::Closure(_)
                            | VmSlot::InlineClosure { .. }
                            | VmSlot::MultiArity(_),
                        ) => Ok(VmSlot::Bool(false)),
                        (left, right) => {
                            match (left.into_runtime_value(), right.into_runtime_value()) {
                                (Some(left), Some(right)) => {
                                    crate::core::apply_intrinsic_name(name, &[left, right])
                                        .map(VmSlot::from)
                                }
                                _ => Err("= expects values".into()),
                            }
                        }
                    }
                } else {
                    self.scratch.clear();
                    let argument_base = self.stack.len() - argc;
                    for value in self.stack.drain(argument_base..) {
                        self.scratch
                            .push(Machine::into_value(self.program.clone(), value));
                    }
                    if matches!(instruction, Instruction::ProtocolCall { .. }) {
                        crate::core::protocol_intrinsic_call(name, &self.scratch).map(VmSlot::from)
                    } else {
                        crate::core::apply_intrinsic_name(name, &self.scratch).map(VmSlot::from)
                    }
                };
                match result {
                    Ok(value) => self.stack.push(value),
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::Jump(target) => next_ip = *target as usize,
            Instruction::JumpIfFalse(target) => {
                let Some(condition) = self.stack.pop() else {
                    match self.raise(function, "stack underflow") {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                };
                if !condition.truthy() {
                    next_ip = *target as usize;
                }
            }
            Instruction::DefProtocol(index)
            | Instruction::ExtendType(index)
            | Instruction::DefMulti(index)
            | Instruction::DefMethod(index) => {
                let Some(form) = program.constants.get(*index as usize) else {
                    return Dispatch::Failed(
                        self.error(function, format!("constant index {index} out of range")),
                    );
                };
                let operator = match instruction {
                    Instruction::DefProtocol(_) => "defprotocol",
                    Instruction::ExtendType(_) => "extend-type",
                    Instruction::DefMulti(_) => "defmulti",
                    Instruction::DefMethod(_) => "defmethod",
                    _ => unreachable!("declaration instruction was matched"),
                };
                match crate::core::eval_bytecode_declaration(operator, form) {
                    Ok(value) => self.stack.push(value.into()),
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::IntrinsicValue(target) => {
                let Some(name) = constant_string(program, *target) else {
                    return Dispatch::Failed(self.error(
                        function,
                        format!("intrinsic target constant {target} is invalid"),
                    ));
                };
                let Some(value) = crate::core::direct_function_value(name) else {
                    return Dispatch::Failed(self.error(
                        function,
                        format!("missing direct intrinsic callable: {name}"),
                    ));
                };
                self.stack.push(value.into());
            }
            Instruction::BuiltinValue(index) => {
                let Some(Value::String(name)) = program.constants.get(*index as usize) else {
                    return Dispatch::Failed(self.error(
                        function,
                        format!("builtin name constant {index} is invalid"),
                    ));
                };
                match crate::core::bytecode_callable_value(name) {
                    Ok(value) => self.stack.push(value.into()),
                    Err(message) => {
                        return Dispatch::Failed(self.error(function, message));
                    }
                }
            }
            Instruction::NamespaceValue(index) => {
                let Some(name) = constant_string(program, *index) else {
                    return Dispatch::Failed(self.error(
                        function,
                        format!("namespace name constant {index} is invalid"),
                    ));
                };
                match crate::core::vm_resolve_namespace_value(name) {
                    Ok(value) => self.stack.push(value.into()),
                    Err(message) => return Dispatch::Failed(self.error(function, message)),
                }
            }
            Instruction::NamespaceOperation(index) => {
                let Some(value) = program.constants.get(*index as usize) else {
                    return Dispatch::Failed(
                        self.error(function, format!("constant index {index} out of range")),
                    );
                };
                match crate::core::eval_bytecode_management(value) {
                    Ok(value) => self.stack.push(value.into()),
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::DynamicBind(index) => {
                let Some(Value::String(name)) = program.constants.get(*index as usize) else {
                    return Dispatch::Failed(self.error(
                        function,
                        format!("binding name constant {index} is invalid"),
                    ));
                };
                let Some(value) = self.stack.pop() else {
                    return Dispatch::Failed(self.error(function, "stack underflow"));
                };
                match crate::core::bytecode_dynamic_bind(
                    name,
                    Self::into_value(program.clone(), value),
                ) {
                    Ok(()) => self.stack.push(VmSlot::Nil),
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::DynamicUnbind(index) => {
                let Some(Value::String(name)) = program.constants.get(*index as usize) else {
                    return Dispatch::Failed(self.error(
                        function,
                        format!("binding name constant {index} is invalid"),
                    ));
                };
                match crate::core::bytecode_dynamic_unbind(name) {
                    Ok(()) => self.stack.push(VmSlot::Nil),
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::Closure {
                prototype,
                captures,
            } => {
                guarded!(self.exec_closure(program, *prototype, *captures));
            }
            Instruction::Call { argc } => match self.collect_call(*argc) {
                Ok((callee, args)) => return Dispatch::Call { callee, args },
                Err(message) => match self.raise(function, message) {
                    Ok(target) => return Dispatch::Unwound(target),
                    Err(error) => return Dispatch::Failed(error),
                },
            },
            Instruction::CallStatic { prototype, argc } => {
                let direct = program
                    .functions
                    .get(usize::from(*prototype))
                    .is_some_and(|proto| {
                        proto.capture_count == 0 && !proto.async_function && !proto.variadic
                    });
                if direct {
                    if self.stack.len() < usize::from(*argc) {
                        match self.raise(function, "stack underflow") {
                            Ok(target) => return Dispatch::Unwound(target),
                            Err(error) => return Dispatch::Failed(error),
                        }
                    }
                    return Dispatch::CallStaticDirect {
                        prototype: *prototype,
                        argc: *argc,
                    };
                }
                match self.collect_call_static(program, function, *prototype, *argc) {
                    Ok((prototype, args, captures)) => {
                        return Dispatch::CallStatic {
                            prototype,
                            args,
                            captures,
                        };
                    }
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::Throw => {
                let Some(value) = self.stack.pop() else {
                    match self.raise(function, "stack underflow") {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                };
                let value = Self::into_value(program.clone(), value);
                let position = function.source_map.position(self.ip);
                let site = position.map(|position| crate::core::ExceptionSite {
                    namespace: program.namespace.clone(),
                    resource: None,
                    line: position.line,
                    column: position.column,
                });
                if !matches!(value, Value::ExceptionInfo(_)) {
                    return Dispatch::Failed(
                        self.error(function, "throw expects an Exception value created by ex"),
                    );
                }
                let message = crate::core::thrown_error_at(value, site);
                match self.raise(function, message) {
                    Ok(target) => return Dispatch::Unwound(target),
                    Err(error) => return Dispatch::Failed(error),
                }
            }
            Instruction::Rethrow => {
                let Some(value) = self.stack.pop() else {
                    match self.raise(function, "stack underflow") {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                };
                let message = match value.into_runtime_value() {
                    Some(value @ Value::ExceptionInfo(_)) => {
                        let position = function.source_map.position(self.ip);
                        crate::core::thrown_error_at(
                            value,
                            position.map(|position| crate::core::ExceptionSite {
                                namespace: self.program.namespace.clone(),
                                resource: None,
                                line: position.line,
                                column: position.column,
                            }),
                        )
                    }
                    Some(Value::String(message)) => message,
                    Some(_) => "rethrow expects an Exception or pending error value".to_string(),
                    None => {
                        // Defensive: the compiler only emits Rethrow behind a
                        // pending-error flag it set to a message string.
                        "rethrow expects a string message".to_string()
                    }
                };
                match self.raise(function, message) {
                    Ok(target) => return Dispatch::Unwound(target),
                    Err(error) => return Dispatch::Failed(error),
                }
            }
            Instruction::GetGlobal(index) => {
                guarded!(self.exec_get_global(program, *index));
            }
            Instruction::DefGlobal { name, metadata } => {
                guarded!(self.exec_def_global(program, *name, *metadata));
            }
            Instruction::SetGlobal(index) => {
                guarded!(self.exec_set_global(program, *index));
            }
            Instruction::VarGlobal(index) => {
                guarded!(self.exec_var_global(program, *index));
            }
            Instruction::DeclareGlobal(index) => {
                guarded!(self.exec_declare_global(program, *index));
            }
            Instruction::DefStruct { name, fields } => {
                guarded!(self.exec_def_struct(program, *name, *fields));
            }
            Instruction::DefMutable { name, fields } => {
                guarded!(self.exec_def_mutable(program, *name, *fields));
            }
            Instruction::MutableFieldGet(index) => {
                guarded!(self.exec_mutable_field_get(program, *index));
            }
            Instruction::MutableFieldSet(index) => {
                guarded!(self.exec_mutable_field_set(program, *index));
            }
            Instruction::InstanceOf => {
                guarded!(self.exec_instance_of());
            }
            Instruction::MakeMultiArity { name, count } => {
                guarded!(self.exec_make_multi_arity(program, *name, *count));
            }
            Instruction::BuildVector(count) => {
                guarded!(self.exec_build_collection(program, *count, false, false));
            }
            Instruction::BuildMap(pairs) => {
                guarded!(self.exec_build_collection(program, pairs.saturating_mul(2), true, false));
            }
            Instruction::BuildSet(count) => {
                guarded!(self.exec_build_collection(program, *count, false, true));
            }
            Instruction::BuildList(count) => {
                guarded!(self.exec_build_list(*count, false));
            }
            Instruction::ConcatList(count) => {
                guarded!(self.exec_build_list(*count, true));
            }
            Instruction::ToVector => {
                guarded!(self.exec_to_vector());
            }
            Instruction::DefMacro { name, metadata } => {
                guarded!(self.exec_def_macro(program, *name, *metadata));
            }
            Instruction::Await => {
                let Some(value) = self.stack.last() else {
                    return Dispatch::Failed(self.error(function, "stack underflow"));
                };
                let Some(Value::Promise(promise)) = value.runtime_value() else {
                    match self.raise(function, "await expects a promise") {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    }
                };
                match promise.state() {
                    PromiseState::Pending => return Dispatch::Suspended(promise),
                    PromiseState::Fulfilled(value) => {
                        self.stack.pop();
                        self.stack.push(value.into());
                    }
                    PromiseState::Rejected(error) => {
                        self.stack.pop();
                        match self.raise(function, error.message()) {
                            Ok(target) => return Dispatch::Unwound(target),
                            Err(error) => return Dispatch::Failed(error),
                        }
                    }
                }
            }
            Instruction::Yield => {
                let Some(value) = self.stack.pop() else {
                    return Dispatch::Failed(self.error(function, "stack underflow"));
                };
                return Dispatch::Yielded(Self::into_value(program.clone(), value));
            }
            Instruction::HostCall => {
                if self.stack.len() < 3 {
                    return Dispatch::Failed(self.error(function, "stack underflow"));
                }
                let values = self.stack.split_off(self.stack.len() - 3);
                let mut values = values
                    .into_iter()
                    .map(|value| value.into_runtime_value().ok_or("Host/call expects values"));
                let service = match values.next().expect("three host arguments") {
                    Ok(value) => value,
                    Err(message) => return Dispatch::Failed(self.error(function, message)),
                };
                let target = match values.next().expect("three host arguments") {
                    Ok(value) => value,
                    Err(message) => return Dispatch::Failed(self.error(function, message)),
                };
                let arguments = match values.next().expect("three host arguments") {
                    Ok(value) => value,
                    Err(message) => return Dispatch::Failed(self.error(function, message)),
                };
                match crate::core::call_host_value(service, target, arguments) {
                    Ok(value) => self.stack.push(value.into()),
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::DotCall { method, argc } => {
                let count = usize::from(*argc) + 1;
                if self.stack.len() < count {
                    return Dispatch::Failed(self.error(function, "stack underflow"));
                }
                let Some(method) = constant_string(program, *method) else {
                    return Dispatch::Failed(
                        self.error(function, "dot method constant is invalid"),
                    );
                };
                let values = self.stack.split_off(self.stack.len() - count);
                let mut values = values
                    .into_iter()
                    .map(|value| Machine::into_value(self.program.clone(), value));
                let receiver = values.next().expect("dot receiver was counted");
                match crate::core::dot_call_values(receiver, method, values.collect()) {
                    Ok(value) => self.stack.push(value.into()),
                    Err(message) => match self.raise(function, message) {
                        Ok(target) => return Dispatch::Unwound(target),
                        Err(error) => return Dispatch::Failed(error),
                    },
                }
            }
            Instruction::Return => {
                return match self.stack.pop() {
                    Some(value) => Dispatch::Returned(value),
                    None => Dispatch::Failed(self.error(function, "stack underflow")),
                };
            }
        }
        Dispatch::Next(next_ip)
    }
}
