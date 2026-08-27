//! Named-call and immediate-function specialization.

use crate::kernel::{Form, Span};
use crate::vm::error::{CompileError, CompileErrorKind};
use crate::vm::opcode::Instruction;

use super::{Child, Compiler};

impl Compiler {
    pub(super) fn compile_named_call(
        &mut self,
        name: &str,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if self.ctx().scopes.resolve(name).is_none() {
            if let Some(target) = crate::core::canonical_intrinsic_callable_symbol(name) {
                return self.compile_intrinsic_call(&target, children, span);
            }
            if let Some(target) = self.inline_target(name) {
                return self.compile_forwarded_call(&target, children, span);
            }
        }
        let argc = (children.len() - 1) as u8;
        if self.ctx().name.as_deref() == Some(name) && self.ctx().captures.is_empty() {
            let prototype = self.ctx().proto_id as u16;
            let accepts = {
                let proto = &self.functions[usize::from(prototype)];
                (!proto.variadic && proto.arity == u16::from(argc))
                    || (proto.variadic && u16::from(argc) >= proto.arity)
            };
            if accepts {
                self.compile_call_arguments(children, span)?;
                if self.ctx().fallthrough {
                    self.emit(
                        Instruction::CallStatic { prototype, argc },
                        Some(span.start),
                    );
                }
                return Ok(());
            }
        }
        match self.ctx().scopes.resolve(name) {
            Some(slot) => self.emit(Instruction::LoadLocal(slot), Some(span.start)),
            None if self.visible_global(name) => {
                let index = self.global_name_constant(name, span)?;
                self.emit(Instruction::GetGlobal(index), Some(span.start))
            }
            None if self.visible_bytecode_callable(name) => {
                let index = self.name_constant(name, span)?;
                self.emit(Instruction::BuiltinValue(index), Some(span.start))
            }
            None if self.visible_namespace(name) => {
                let index = self.name_constant(name, span)?;
                self.emit(Instruction::NamespaceValue(index), Some(span.start))
            }
            None => {
                return Err(CompileError::new(
                    CompileErrorKind::UnboundSymbol,
                    format!("unbound symbol: {name}"),
                    Some(span.start),
                ))
            }
        };
        self.compile_call_arguments(children, span)?;
        if self.ctx().fallthrough {
            self.emit(Instruction::Call { argc }, Some(span.start));
        }
        Ok(())
    }

    fn inline_target(&self, name: &str) -> Option<String> {
        if let Some(target) = self.inline_globals.get(name) {
            return Some(target.clone());
        }
        crate::core::namespace_registry()
            .ok()
            .and_then(|registry| {
                let current = registry
                    .find(&self.namespace)
                    .unwrap_or_else(|| registry.current());
                registry
                    .resolve(&crate::lang::data::Symbol::parse(name))
                    .or_else(|| current.resolve(&crate::lang::data::Symbol::parse(name)))
            })
            .and_then(|var| var.hara_metadata())
            .and_then(|metadata| match metadata.get_keyword("inline-target") {
                Some(crate::lang::data::MetadataValue::Symbol(target)) => {
                    Some(target.as_str().to_owned())
                }
                _ => None,
            })
    }

    fn compile_forwarded_call(
        &mut self,
        target: &str,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if let Some(canonical) = crate::core::canonical_intrinsic_callable_symbol(target) {
            return self.compile_intrinsic_call(&canonical, children, span);
        }
        let argc = (children.len() - 1) as u8;
        let index = self.global_name_constant(target, span)?;
        self.emit(Instruction::GetGlobal(index), Some(span.start));
        self.compile_call_arguments(children, span)?;
        if self.ctx().fallthrough {
            self.emit(Instruction::Call { argc }, Some(span.start));
        }
        Ok(())
    }

    fn compile_intrinsic_call(
        &mut self,
        target: &str,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        let argc = children.len() - 1;
        if argc > usize::from(u8::MAX) {
            return Err(CompileError::new(
                CompileErrorKind::Limit,
                "intrinsic calls support at most 255 arguments",
                Some(span.start),
            ));
        }
        for argument in &children[1..] {
            self.compile_form(argument.form, argument.span, argument.children, false)?;
        }
        if !self.ctx().fallthrough {
            return Ok(());
        }
        let is_protocol = target.starts_with("std.protocol.");
        let target = self.name_constant(target, span)?;
        let instruction = if is_protocol {
            Instruction::ProtocolCall {
                target,
                argc: argc as u8,
            }
        } else {
            Instruction::IntrinsicCall {
                target,
                argc: argc as u8,
            }
        };
        self.emit(instruction, Some(span.start));
        Ok(())
    }

    pub(super) fn compile_immediate_fn_call(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<bool, CompileError> {
        let callee = &children[0];
        let Form::List(elements) = callee.form else {
            return Ok(false);
        };
        if !matches!(elements.first(), Some(Form::Symbol(name)) if name == "fn") {
            return Ok(false);
        }
        let fn_children = self.list_children(elements, callee.span, callee.children);
        if fn_children.len() < 3 {
            return Ok(false);
        }
        let Form::Vector(params) = fn_children[1].form else {
            return Ok(false);
        };
        if params.len() != children.len() - 1
            || params
                .iter()
                .any(|param| !matches!(param, Form::Symbol(name) if name != "&"))
            || fn_children[2..]
                .iter()
                .any(|body| contains_recur(body.form))
        {
            return Ok(false);
        }
        if params.len() > crate::vm::program::MAX_PRIMITIVE_ARGUMENTS {
            return Err(CompileError::new(
                CompileErrorKind::Limit,
                format!(
                    "calls support at most {} arguments",
                    crate::vm::program::MAX_PRIMITIVE_ARGUMENTS
                ),
                Some(span.start),
            ));
        }
        for argument in &children[1..] {
            self.compile_form(argument.form, argument.span, argument.children, false)?;
        }
        if !self.ctx().fallthrough {
            return Ok(true);
        }
        let param_children =
            self.list_children(params, fn_children[1].span, fn_children[1].children);
        self.ctx_mut().scopes.push_scope();
        let result = (|| {
            let mut slots = Vec::with_capacity(params.len());
            for param in &param_children {
                let Form::Symbol(name) = param.form else {
                    unreachable!("parameter shape checked above")
                };
                slots.push(self.ctx_mut().scopes.declare(name).map_err(|error| {
                    CompileError::new(error.kind(), error.message(), Some(param.span.start))
                })?);
            }
            for (slot, param) in slots.iter().zip(&param_children).rev() {
                self.emit(Instruction::StoreLocal(*slot), Some(param.span.start));
            }
            self.compile_sequence(&fn_children[2..], false)
        })();
        self.ctx_mut().scopes.pop_scope();
        result?;
        Ok(true)
    }
}

fn contains_recur(form: &Form) -> bool {
    match form {
        Form::List(values) => {
            matches!(values.first(), Some(Form::Symbol(name)) if name == "recur")
                || values.iter().any(contains_recur)
        }
        Form::Vector(values) | Form::Set(values) => values.iter().any(contains_recur),
        Form::Map(values) => values
            .iter()
            .any(|(key, value)| contains_recur(key) || contains_recur(value)),
        Form::Tagged(_, value) | Form::Metadata(_, value) => contains_recur(value),
        _ => false,
    }
}
