//! Typed instruction set for the experimental bytecode VM.
//!
//! Instructions are a typed enum, not packed bytes: the milestone
//! prioritises exact validation and disassembly over encoding density.
//! Jump operands are absolute instruction indexes.

/// One VM instruction.
///
/// Stack effects (validated before execution):
///
/// - `Constant`, `Nil`, `True`, `False`, `LoadLocal`: push 1.
/// - `StoreLocal`, `Pop`, `JumpIfFalse`: pop 1.
/// - `IntrinsicCall`, `ProtocolCall`: pop `argc`, push 1 (net `1 - argc`).
/// - `Closure`: pops `captures`, pushes 1 (net `1 - captures`).
/// - `Call`: pops `argc` arguments plus the callee, pushes 1 (net `-argc`).
/// - `CallStatic`: pops `argc`, pushes 1 (net `1 - argc`).
/// - `Jump`: no change.
/// - `Throw`, `Rethrow`: pop 1; terminal (unwind).
/// - `GetGlobal`, `VarGlobal`, `DefStruct`, `DefMutable`, `DeclareGlobal`: push 1.
/// - `DefGlobal`, `SetGlobal`, `MutableFieldGet`: pop 1, push 1 (net 0).
/// - `MutableFieldSet`: pops receiver and replacement, pushes replacement (net -1).
/// - `InstanceOf`: pops 2, pushes 1 (net -1).
/// - `MakeMultiArity`: pops `count`, pushes 1 (net `1 - count`).
/// - `Return`: terminal; requires stack height exactly 1.
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    /// Pushes `constants[index]` onto the operand stack.
    Constant(u32),
    /// Pushes `Value::Nil`.
    Nil,
    /// Pushes `Value::Bool(true)`.
    True,
    /// Pushes `Value::Bool(false)`.
    False,
    /// Pushes the value of local slot `slot`.
    LoadLocal(u16),
    /// Pops the top of the stack into local slot `slot`.
    StoreLocal(u16),
    /// Discards the top of the stack.
    Pop,
    /// Duplicates the top stack value.
    Dup,
    /// Pops `argc` arguments and invokes the named runtime intrinsic.
    IntrinsicCall {
        target: u32,
        argc: u8,
    },
    /// Unconditional jump to an absolute instruction index.
    Jump(u32),
    /// Pops the condition and jumps when it is not truthy
    /// (`Value::truthy`: only `nil` and `false` are false).
    JumpIfFalse(u32),
    /// Pops `captures` captured values and pushes a function value for
    /// `prototype`.
    Closure {
        prototype: u16,
        captures: u8,
    },
    /// Pops `argc` arguments and then the callee, invokes the function
    /// value through the shared `call_function` boundary, and pushes the
    /// result.
    Call {
        argc: u8,
    },
    /// Pops `argc` arguments and calls `prototype` directly, copying the
    /// current frame's capture slots as the callee's captures (`defn`
    /// self-recursion).
    CallStatic {
        prototype: u16,
        argc: u8,
    },
    /// Pops one value and raises it as a guest exception through the
    /// shared `core::thrown_error` boundary. Terminal: unwinds to the
    /// innermost covering try entry or fails the run.
    Throw,
    /// Pops a string and raises that exact message without touching the
    /// thrown-value side channel, preserving error identity across an
    /// unmatched finally boundary. Terminal; only emitted in finally
    /// resume sequences.
    Rethrow,
    /// Pushes the dereferenced value of the var named by the string
    /// constant at `constants[index]`, resolved through the namespace
    /// registry at execution time.
    GetGlobal(u32),
    /// Pops a value, interns it as a `Var` in the current namespace
    /// (optional hara metadata from the program's var-metadata table),
    /// and pushes the interned Var back (`def` returns the Var).
    DefGlobal {
        name: u32,
        metadata: Option<u16>,
    },
    /// Pops a value, resets the root of the var named by the string
    /// constant at `constants[index]`, and pushes the value.
    SetGlobal(u32),
    /// Pushes the `Value::Var` named by the string constant at
    /// `constants[index]` (`var` / `#'`).
    VarGlobal(u32),
    /// Interns a nil var for the string constant at `constants[index]`
    /// when the name is not already bound (`declare`), pushing nil.
    /// Never resets an existing var.
    DeclareGlobal(u32),
    /// Creates the struct type named by the constant at `name` (qualified
    /// to the current namespace), interns the constructor vars, and
    /// pushes the type value. `fields` indexes a vector constant of
    /// field-name strings.
    DefStruct {
        name: u32,
        fields: u32,
    },
    /// Creates the mutable named type at `name`, interns its constructors,
    /// and pushes nil. Appended after the original HBC0 opcode set.
    DefMutable {
        name: u32,
        fields: u32,
    },
    /// Pops a mutable instance and pushes the declared field value named by
    /// the string constant at `constants[index]` (`field`).
    MutableFieldGet(u32),
    /// Pops the replacement and mutable receiver, performs one direct field
    /// mutation, and pushes the replacement value (`set!` field place).
    MutableFieldSet(u32),
    /// Pops the value and then a named type, and pushes whether the
    /// value is an instance (`instance?`).
    InstanceOf,
    /// Pops `count` function values and pushes the arity dispatcher
    /// named by the string constant at `constants[name]`, built through
    /// the shared `core::arity_dispatcher` boundary.
    MakeMultiArity {
        name: u32,
        count: u8,
    },
    /// Pops `count` values and constructs a compact tuple, upgrading to a vector above arity 8.
    BuildVector(u16),
    /// Pops `pairs * 2` alternating keys and values and constructs an
    /// persistent hash map.
    BuildMap(u16),
    /// Pops `count` values and constructs an insertion-ordered set.
    BuildSet(u16),
    BuildList(u16),
    ConcatList(u16),
    ToVector,
    /// Pops a function value, interns it as a macro Var, registers it in the
    /// active Runtime macro registry, and pushes the function back.
    DefMacro {
        name: u32,
        metadata: Option<u16>,
    },
    /// Defines a protocol from a validated structured declaration constant.
    DefProtocol(u32),
    /// Extends a struct type from a validated structured declaration constant.
    ExtendType(u32),
    /// Defines a multimethod from a validated structured declaration constant.
    DefMulti(u32),
    /// Adds one multimethod implementation from a validated declaration.
    DefMethod(u32),
    /// Pushes a first-class callable for a runtime intrinsic.
    IntrinsicValue(u32),
    /// Pushes a first-class callable implemented by the structural runtime.
    BuiltinValue(u32),
    /// Pushes a namespace value resolved from the current namespace's alias
    /// table (or from the registry by name), matching the evaluator's bare
    /// namespace alias form.
    NamespaceValue(u32),
    /// Applies a validated `ns`, `ns+`, or `require` form retained in the
    /// constant pool and pushes its result. This is the bytecode management
    /// seam used for nested namespace operations; top-level declarations are
    /// prepared by the runtime before ordinary code is compiled.
    NamespaceOperation(u32),
    /// Binds a dynamic Var to the value on top of the stack and leaves nil.
    DynamicBind(u32),
    /// Removes the most recent binding for a dynamic Var and leaves nil.
    DynamicUnbind(u32),
    /// Replaces a settled promise with its value, raises a rejection, or
    /// suspends the current VM fiber while preserving the complete machine.
    Await,
    /// Suspends the current bytecode coroutine with the value on top of
    /// the stack. Resumption replaces it with the caller-supplied value.
    Yield,
    /// Pops service, method, and argument-vector values and returns the
    /// provider's ordinary Promise value.
    HostCall,
    /// Invokes a native collection method with an evaluated receiver and arguments.
    DotCall {
        method: u32,
        argc: u8,
    },
    /// Pops `argc` protocol arguments, including the receiver, and dispatches
    /// the canonical protocol method through the active protocol registry.
    ProtocolCall {
        target: u32,
        argc: u8,
    },
    /// Returns the top of the stack as the function result.
    Return,
}

impl Instruction {
    /// The jump target, when the instruction transfers control.
    pub fn jump_target(&self) -> Option<u32> {
        match self {
            Instruction::Jump(target) | Instruction::JumpIfFalse(target) => Some(*target),
            _ => None,
        }
    }

    /// Whether control falls through to the next instruction.
    pub(crate) fn falls_through(&self) -> bool {
        !matches!(
            self,
            Instruction::Jump(_) | Instruction::Return | Instruction::Throw | Instruction::Rethrow
        )
    }

    /// Static stack effect of the instruction; `None` for the terminals
    /// (`Return` requires height exactly 1, `Throw`/`Rethrow` pop 1), which
    /// are validated separately.
    pub(crate) fn stack_effect(&self) -> Option<i32> {
        Some(match self {
            Instruction::Constant(_)
            | Instruction::Nil
            | Instruction::True
            | Instruction::False
            | Instruction::LoadLocal(_) => 1,
            Instruction::StoreLocal(_) | Instruction::Pop | Instruction::JumpIfFalse(_) => -1,
            Instruction::Dup => 1,
            Instruction::IntrinsicCall { argc, .. }
            | Instruction::ProtocolCall { argc, .. }
            | Instruction::CallStatic { argc, .. } => 1 - i32::from(*argc),
            Instruction::Closure { captures, .. } => 1 - i32::from(*captures),
            Instruction::Call { argc } => -i32::from(*argc),
            Instruction::GetGlobal(_)
            | Instruction::VarGlobal(_)
            | Instruction::DefStruct { .. }
            | Instruction::DefMutable { .. }
            | Instruction::DeclareGlobal(_) => 1,
            Instruction::DefProtocol(_)
            | Instruction::ExtendType(_)
            | Instruction::DefMulti(_)
            | Instruction::DefMethod(_) => 1,
            Instruction::IntrinsicValue(_) => 1,
            Instruction::BuiltinValue(_) => 1,
            Instruction::NamespaceValue(_) | Instruction::NamespaceOperation(_) => 1,
            Instruction::DynamicBind(_) => 0,
            Instruction::DynamicUnbind(_) => 1,
            Instruction::Yield => 0,
            Instruction::DefGlobal { .. }
            | Instruction::SetGlobal(_)
            | Instruction::MutableFieldGet(_) => 0,
            Instruction::MutableFieldSet(_) => -1,
            Instruction::InstanceOf => -1,
            Instruction::MakeMultiArity { count, .. } => 1 - i32::from(*count),
            Instruction::BuildVector(count)
            | Instruction::BuildSet(count)
            | Instruction::BuildList(count)
            | Instruction::ConcatList(count) => 1 - i32::from(*count),
            Instruction::BuildMap(pairs) => 1 - (2 * i32::from(*pairs)),
            Instruction::ToVector => 0,
            Instruction::DefMacro { .. } => 0,
            Instruction::Await => 0,
            Instruction::HostCall => -2,
            Instruction::DotCall { argc, .. } => -i32::from(*argc),
            Instruction::Jump(_) => 0,
            Instruction::Return | Instruction::Throw | Instruction::Rethrow => return None,
        })
    }
}

impl std::fmt::Display for Instruction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Instruction::Constant(index) => write!(formatter, "Constant {index}"),
            Instruction::Nil => formatter.write_str("Nil"),
            Instruction::True => formatter.write_str("True"),
            Instruction::False => formatter.write_str("False"),
            Instruction::LoadLocal(slot) => write!(formatter, "LoadLocal {slot}"),
            Instruction::StoreLocal(slot) => write!(formatter, "StoreLocal {slot}"),
            Instruction::Pop => formatter.write_str("Pop"),
            Instruction::Dup => formatter.write_str("Dup"),
            Instruction::IntrinsicCall { target, argc } => {
                write!(formatter, "IntrinsicCall target {target} argc {argc}")
            }
            Instruction::Jump(target) => write!(formatter, "Jump {target:04}"),
            Instruction::JumpIfFalse(target) => write!(formatter, "JumpIfFalse {target:04}"),
            Instruction::Closure {
                prototype,
                captures,
            } => {
                write!(formatter, "Closure {prototype:04} captures {captures}")
            }
            Instruction::Call { argc } => write!(formatter, "Call {argc}"),
            Instruction::CallStatic { prototype, argc } => {
                write!(formatter, "CallStatic {prototype:04} {argc}")
            }
            Instruction::Throw => formatter.write_str("Throw"),
            Instruction::Rethrow => formatter.write_str("Rethrow"),
            Instruction::GetGlobal(index) => write!(formatter, "GetGlobal {index}"),
            Instruction::DefGlobal { name, metadata } => match metadata {
                Some(metadata) => write!(formatter, "DefGlobal {name} meta {metadata}"),
                None => write!(formatter, "DefGlobal {name}"),
            },
            Instruction::SetGlobal(index) => write!(formatter, "SetGlobal {index}"),
            Instruction::VarGlobal(index) => write!(formatter, "VarGlobal {index}"),
            Instruction::DeclareGlobal(index) => write!(formatter, "DeclareGlobal {index}"),
            Instruction::DefStruct { name, fields } => {
                write!(formatter, "DefStruct {name} fields {fields}")
            }
            Instruction::DefMutable { name, fields } => {
                write!(formatter, "DefMutable {name} fields {fields}")
            }
            Instruction::MutableFieldGet(index) => {
                write!(formatter, "MutableFieldGet {index}")
            }
            Instruction::MutableFieldSet(index) => {
                write!(formatter, "MutableFieldSet {index}")
            }
            Instruction::InstanceOf => formatter.write_str("InstanceOf"),
            Instruction::MakeMultiArity { name, count } => {
                write!(formatter, "MakeMultiArity {name} count {count}")
            }
            Instruction::BuildVector(count) => write!(formatter, "BuildVector {count}"),
            Instruction::BuildMap(count) => write!(formatter, "BuildMap {count}"),
            Instruction::BuildSet(count) => write!(formatter, "BuildSet {count}"),
            Instruction::BuildList(count) => write!(formatter, "BuildList {count}"),
            Instruction::ConcatList(count) => write!(formatter, "ConcatList {count}"),
            Instruction::ToVector => formatter.write_str("ToVector"),
            Instruction::DefMacro { name, metadata } => match metadata {
                Some(metadata) => write!(formatter, "DefMacro {name} meta {metadata}"),
                None => write!(formatter, "DefMacro {name}"),
            },
            Instruction::DefProtocol(index) => write!(formatter, "DefProtocol {index}"),
            Instruction::ExtendType(index) => write!(formatter, "ExtendType {index}"),
            Instruction::DefMulti(index) => write!(formatter, "DefMulti {index}"),
            Instruction::DefMethod(index) => write!(formatter, "DefMethod {index}"),
            Instruction::IntrinsicValue(target) => {
                write!(formatter, "IntrinsicValue target {target}")
            }
            Instruction::BuiltinValue(index) => write!(formatter, "BuiltinValue {index}"),
            Instruction::NamespaceValue(index) => write!(formatter, "NamespaceValue {index}"),
            Instruction::NamespaceOperation(index) => {
                write!(formatter, "NamespaceOperation {index}")
            }
            Instruction::DynamicBind(index) => write!(formatter, "DynamicBind {index}"),
            Instruction::DynamicUnbind(index) => write!(formatter, "DynamicUnbind {index}"),
            Instruction::Await => formatter.write_str("Await"),
            Instruction::Yield => formatter.write_str("Yield"),
            Instruction::HostCall => formatter.write_str("HostCall"),
            Instruction::DotCall { method, argc } => {
                write!(formatter, "DotCall {method} {argc}")
            }
            Instruction::ProtocolCall { target, argc } => {
                write!(formatter, "ProtocolCall target {target} argc {argc}")
            }
            Instruction::Return => formatter.write_str("Return"),
        }
    }
}
