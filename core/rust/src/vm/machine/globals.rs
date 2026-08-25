//! Global, struct, and multi-arity instruction helpers.

use crate::lang::data::Symbol;
use std::rc::Rc;

use super::{
    constant_named_fields, constant_string, Machine, Program, Value, VmClosure, VmMultiArity,
    VmSlot,
};

impl Machine {
    #[inline(never)]
    pub(super) fn exec_build_collection(
        &mut self,
        program: &Program,
        count: u16,
        map: bool,
        set: bool,
    ) -> Result<(), String> {
        let count = usize::from(count);
        if self.stack.len() < count {
            return Err("stack underflow".into());
        }
        let start = self.stack.len() - count;
        let values = self
            .stack
            .drain(start..)
            .map(|value| Machine::into_value(self.program.clone(), value))
            .collect::<Vec<_>>();
        let value = if map {
            crate::core::vm_build_map(values)?
        } else if set {
            crate::core::vm_build_set(values)?
        } else {
            crate::core::vm_build_vector(values)?
        };
        let _ = program;
        self.stack.push(value.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_build_list(&mut self, count: u16, concatenate: bool) -> Result<(), String> {
        let count = usize::from(count);
        if self.stack.len() < count {
            return Err("stack underflow".into());
        }
        let start = self.stack.len() - count;
        let values = self
            .stack
            .drain(start..)
            .map(|value| Machine::into_value(self.program.clone(), value))
            .collect::<Vec<_>>();
        let value = if concatenate {
            crate::core::vm_concat_list(values)?
        } else {
            crate::core::vm_build_list(values)
        };
        self.stack.push(value.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_to_vector(&mut self) -> Result<(), String> {
        let Some(value) = self.stack.pop() else {
            return Err("stack underflow".into());
        };
        let value = Machine::into_value(self.program.clone(), value);
        self.stack.push(crate::core::vm_to_vector(value)?.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_get_global(&mut self, program: &Program, index: u32) -> Result<(), String> {
        let Some(name) = constant_string(program, index) else {
            return Err(format!("constant index {index} out of range"));
        };
        let var = crate::core::vm_resolve_global(name)?;
        let value = var.deref_value();
        let slot = Machine::callable_key(&value)
            .and_then(|key| self.vm_globals.get(&key).cloned())
            .unwrap_or_else(|| value.into());
        self.stack.push(slot);
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_def_global(
        &mut self,
        program: &Program,
        name: u32,
        metadata: Option<u16>,
    ) -> Result<(), String> {
        let Some(name) = constant_string(program, name) else {
            return Err(format!("constant index {name} out of range"));
        };
        let Some(value) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let metadata = metadata.map(|index| program.var_metadata[usize::from(index)].clone());
        let runtime_value = Machine::into_value(self.program.clone(), value.clone());
        let var = crate::core::vm_def_global(name, runtime_value.clone(), metadata)?;
        self.remember_vm_global(&runtime_value, value.clone());
        self.stack.push(Value::Var(var).into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_def_macro(
        &mut self,
        program: &Program,
        name: u32,
        metadata: Option<u16>,
    ) -> Result<(), String> {
        let Some(name) = constant_string(program, name) else {
            return Err(format!("constant index {name} out of range"));
        };
        let Some(value) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let metadata = metadata.map(|index| program.var_metadata[usize::from(index)].clone());
        let runtime_value = Machine::into_value(self.program.clone(), value.clone());
        crate::core::vm_def_macro(name, runtime_value.clone(), metadata)?;
        self.remember_vm_global(&runtime_value, value.clone());
        self.stack.push(value);
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_set_global(&mut self, program: &Program, index: u32) -> Result<(), String> {
        let Some(name) = constant_string(program, index) else {
            return Err(format!("constant index {index} out of range"));
        };
        let Some(value) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let var = crate::core::namespace_registry().and_then(|registry| {
            registry
                .resolve(&Symbol::parse(name))
                .ok_or_else(|| format!("unbound var: {name}"))
        })?;
        if !crate::core::binding_is_local(&var) {
            return Err(format!(
                "Cannot replace referred Var without ns omission: {name}"
            ));
        }
        let runtime_value = Machine::into_value(self.program.clone(), value.clone());
        var.reset_value(runtime_value.clone());
        self.remember_vm_global(&runtime_value, value.clone());
        self.stack.push(value);
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_var_global(&mut self, program: &Program, index: u32) -> Result<(), String> {
        let Some(name) = constant_string(program, index) else {
            return Err(format!("constant index {index} out of range"));
        };
        let var = crate::core::vm_resolve_global(name)?;
        self.stack.push(Value::Var(var).into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_declare_global(
        &mut self,
        program: &Program,
        index: u32,
    ) -> Result<(), String> {
        let Some(name) = constant_string(program, index) else {
            return Err(format!("constant index {index} out of range"));
        };
        crate::core::vm_declare_global(name)?;
        self.stack.push(VmSlot::Nil);
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_def_struct(
        &mut self,
        program: &Program,
        name: u32,
        fields: u32,
    ) -> Result<(), String> {
        let Some(name) = constant_string(program, name) else {
            return Err(format!("constant index {name} out of range"));
        };
        let Some(field_names) = constant_named_fields(program, fields, "defstruct")? else {
            return Err(format!("constant index {fields} out of range"));
        };
        let value = crate::core::vm_defstruct(name, field_names)?;
        self.stack.push(value.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_def_mutable(
        &mut self,
        program: &Program,
        name: u32,
        fields: u32,
    ) -> Result<(), String> {
        let Some(name) = constant_string(program, name) else {
            return Err(format!("constant index {name} out of range"));
        };
        let Some(field_names) = constant_named_fields(program, fields, "defmutable")? else {
            return Err(format!("constant index {fields} out of range"));
        };
        let value = crate::core::vm_defmutable(name, field_names)?;
        self.stack.push(value.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_mutable_field_get(
        &mut self,
        program: &Program,
        index: u32,
    ) -> Result<(), String> {
        let Some(field) = constant_string(program, index) else {
            return Err(format!("constant index {index} out of range"));
        };
        let Some(value) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let value = Machine::into_value(self.program.clone(), value);
        let value = crate::core::mutable_field_value(&value, field)?;
        self.stack.push(value.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_mutable_field_set(
        &mut self,
        program: &Program,
        index: u32,
    ) -> Result<(), String> {
        let Some(field) = constant_string(program, index) else {
            return Err(format!("constant index {index} out of range"));
        };
        let Some(replacement) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let Some(receiver) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let replacement = Machine::into_value(self.program.clone(), replacement);
        let receiver = Machine::into_value(self.program.clone(), receiver);
        let value = crate::core::mutable_field_set(&receiver, field, replacement)?;
        self.stack.push(value.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_instance_of(&mut self) -> Result<(), String> {
        let Some(value) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let Some(ty) = self.stack.pop() else {
            return Err("stack underflow".to_string());
        };
        let ty = Machine::into_value(self.program.clone(), ty);
        let value = Machine::into_value(self.program.clone(), value);
        let value = crate::core::named_instance_of(&ty, &value)?;
        self.stack.push(value.into());
        Ok(())
    }

    #[inline(never)]
    pub(super) fn exec_make_multi_arity(
        &mut self,
        program: &Program,
        name: u32,
        count: u8,
    ) -> Result<(), String> {
        let count = usize::from(count);
        if self.stack.len() < count {
            return Err("stack underflow".to_string());
        }
        let Some(name) = constant_string(program, name).map(str::to_owned) else {
            return Err(format!("constant index {name} out of range"));
        };
        let start = self.stack.len() - count;
        if self.stack[start..]
            .iter()
            .any(|value| !matches!(value, VmSlot::InlineClosure { .. } | VmSlot::Closure(_)))
        {
            return Err("multi-arity clauses must be functions".to_string());
        }
        let clauses = self
            .stack
            .drain(start..)
            .map(|value| match value {
                VmSlot::InlineClosure { prototype, .. } => Rc::new(VmClosure {
                    prototype,
                    captures: Vec::new(),
                }),
                VmSlot::Closure(closure) => closure,
                _ => unreachable!("checked above"),
            })
            .collect();
        self.stack
            .push(VmSlot::MultiArity(Rc::new(VmMultiArity { name, clauses })));
        Ok(())
    }
}
