//! One execution frame: the local slot array plus its operand-stack base.

use super::slot::VmSlot;

#[derive(Debug)]
pub struct Frame {
    locals: Vec<VmSlot>,
    base: usize,
}

impl Frame {
    /// The frame the machine starts with: all slots initialized to `nil`.
    pub(crate) fn entry(local_count: usize) -> Frame {
        Frame {
            locals: vec![VmSlot::Nil; local_count],
            base: 0,
        }
    }

    /// The frame for a function call: `args` fill the parameter slots
    /// `0..arity`, `captures` the capture slots directly above them, and
    /// the remaining slots start as `nil`. Out-of-range writes are dropped
    /// rather than panicking (the validator guarantees they fit; this
    /// defends hand-built programs).
    pub(crate) fn call(
        local_count: usize,
        arity: usize,
        mut args: Vec<VmSlot>,
        captures: Vec<VmSlot>,
        base: usize,
    ) -> Frame {
        Self::call_reusing(Vec::new(), local_count, arity, &mut args, captures, base)
    }

    pub(crate) fn call_reusing(
        mut locals: Vec<VmSlot>,
        local_count: usize,
        arity: usize,
        args: &mut Vec<VmSlot>,
        captures: Vec<VmSlot>,
        base: usize,
    ) -> Frame {
        locals.clear();
        locals.resize(local_count, VmSlot::Nil);
        for (index, value) in args.drain(..).enumerate() {
            if let Some(cell) = locals.get_mut(index) {
                *cell = value;
            }
        }
        for (index, value) in captures.into_iter().enumerate() {
            if let Some(cell) = locals.get_mut(arity + index) {
                *cell = value;
            }
        }
        Frame { locals, base }
    }

    /// Builds a capture-free frame by moving the top `argc` operands
    /// directly into its parameter slots. Static named calls use this path
    /// so arguments do not make an otherwise redundant trip through a
    /// temporary argument vector before entering the callee.
    pub(crate) fn call_static_reusing(
        mut locals: Vec<VmSlot>,
        local_count: usize,
        stack: &mut Vec<VmSlot>,
        argc: usize,
    ) -> Frame {
        let base = stack.len() - argc;
        locals.clear();
        locals.resize(local_count, VmSlot::Nil);
        for (index, value) in stack.drain(base..).enumerate() {
            locals[index] = value;
        }
        Frame { locals, base }
    }

    pub(crate) fn into_locals(self) -> Vec<VmSlot> {
        self.locals
    }

    pub(crate) fn local(&self, slot: u16) -> Option<&VmSlot> {
        self.locals.get(usize::from(slot))
    }

    /// Clones the `count` slots starting at `start`; `None` when the range
    /// exceeds the frame (rejected by validation, defended here).
    pub(crate) fn slot_range(&self, start: usize, count: usize) -> Option<Vec<VmSlot>> {
        let end = start.checked_add(count)?;
        if end > self.locals.len() {
            return None;
        }
        Some(self.locals[start..end].to_vec())
    }

    /// Stores a value into a slot; false when the slot is out of range
    /// (rejected by validation, defended here so the machine never
    /// panics on malformed programs).
    pub(crate) fn store(&mut self, slot: u16, value: VmSlot) -> bool {
        match self.locals.get_mut(usize::from(slot)) {
            Some(cell) => {
                *cell = value;
                true
            }
            None => false,
        }
    }

    /// Borrowed local slots for bounded machine observations.
    pub(crate) fn locals(&self) -> &[VmSlot] {
        &self.locals
    }

    /// Operand-stack base at which this frame was entered.
    pub(crate) fn base(&self) -> usize {
        self.base
    }

    #[cfg(feature = "tracing-jit")]
    pub(crate) fn trace_locals(&self) -> (Vec<crate::jit::TraceValue>, Vec<bool>) {
        let mut scalar = Vec::with_capacity(self.locals.len());
        let mut writable = Vec::with_capacity(self.locals.len());
        for value in &self.locals {
            match value {
                VmSlot::Number(value) => {
                    scalar.push(crate::jit::TraceValue::I64(*value));
                    writable.push(true);
                }
                VmSlot::Bool(value) => {
                    scalar.push(crate::jit::TraceValue::Bool(*value));
                    writable.push(true);
                }
                VmSlot::Nil => {
                    scalar.push(crate::jit::TraceValue::Nil);
                    writable.push(true);
                }
                VmSlot::Value(value)
                    if matches!(
                        value.as_ref(),
                        crate::core::Value::Tuple(_) | crate::core::Value::Vector(_)
                    ) =>
                {
                    scalar.push(crate::jit::TraceValue::Indexed(Box::new(
                        value.as_ref().clone(),
                    )));
                    writable.push(true);
                }
                _ => {
                    scalar.push(crate::jit::TraceValue::Unsupported);
                    writable.push(false);
                }
            }
        }
        (scalar, writable)
    }

    #[cfg(feature = "tracing-jit")]
    pub(crate) fn apply_trace_locals(
        &mut self,
        values: &[crate::jit::TraceValue],
        writable: &[bool],
    ) {
        for (index, (value, writable)) in values.iter().zip(writable).enumerate() {
            if !writable {
                continue;
            }
            self.locals[index] = match value {
                crate::jit::TraceValue::I64(value) => VmSlot::Number(*value),
                crate::jit::TraceValue::Bool(value) => VmSlot::Bool(*value),
                crate::jit::TraceValue::Nil => VmSlot::Nil,
                crate::jit::TraceValue::Indexed(value) => {
                    VmSlot::Value(std::rc::Rc::new(value.as_ref().clone()))
                }
                crate::jit::TraceValue::VectorSlice(slice) => {
                    VmSlot::Value(std::rc::Rc::new(crate::core::Value::Vector(
                        slice.values[slice.start..]
                            .iter()
                            .copied()
                            .map(crate::core::Value::Number)
                            .collect(),
                    )))
                }
                crate::jit::TraceValue::Unsupported => continue,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, VmSlot};

    #[test]
    fn static_call_moves_arguments_and_preserves_caller_stack() {
        let mut stack = vec![VmSlot::Number(9), VmSlot::Number(20), VmSlot::Number(22)];
        let frame = Frame::call_static_reusing(Vec::new(), 3, &mut stack, 2);
        assert!(matches!(stack.as_slice(), [VmSlot::Number(9)]));
        assert_eq!(frame.base(), 1);
        assert!(matches!(frame.local(0), Some(VmSlot::Number(20))));
        assert!(matches!(frame.local(1), Some(VmSlot::Number(22))));
        assert!(matches!(frame.local(2), Some(VmSlot::Nil)));
    }
}
