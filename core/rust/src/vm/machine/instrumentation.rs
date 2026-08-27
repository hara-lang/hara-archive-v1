//! Low-overhead, opt-in instrumentation for the production bytecode machine.
//!
//! Unlike `machine::observation`, this module does not project values, locals,
//! handlers, or source strings on every instruction. It emits copy-only scalar
//! events through a monomorphized probe and keeps the ordinary `Machine::run`
//! path unchanged. The boundary API executes one real dispatch operation and
//! leaves all expensive inspection to matching shared-hub subscriptions.

use crate::vm::opcode::Instruction;

#[path = "instrumentation/ring.rs"]
mod ring;
#[path = "instrumentation/run.rs"]
mod run;
#[path = "instrumentation/step.rs"]
mod step;

pub use ring::{EventRing, SampledProbe, VmEvent};
pub use step::{VmBoundary, VmBoundaryOutcome};

pub const BYTECODE_METRICS_SCHEMA: &str = "hal.bytecode-metrics/0-alpha";
pub const BYTECODE_EVENTS_SCHEMA: &str = "hal.bytecode-events/0-alpha";

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Opcode {
    Constant,
    Nil,
    True,
    False,
    LoadLocal,
    StoreLocal,
    Pop,
    Dup,
    IntrinsicCall,
    Jump,
    JumpIfFalse,
    Closure,
    Call,
    CallStatic,
    Throw,
    Rethrow,
    GetGlobal,
    DefGlobal,
    SetGlobal,
    VarGlobal,
    DeclareGlobal,
    DefStruct,
    DefMutable,
    MutableFieldGet,
    MutableFieldSet,
    InstanceOf,
    MakeMultiArity,
    BuildVector,
    BuildMap,
    BuildSet,
    BuildList,
    ConcatList,
    ToVector,
    DefMacro,
    DefProtocol,
    ExtendType,
    DefMulti,
    DefMethod,
    IntrinsicValue,
    BuiltinValue,
    NamespaceValue,
    NamespaceOperation,
    DynamicBind,
    DynamicUnbind,
    Await,
    HostCall,
    DotCall,
    ProtocolCall,
    Yield,
    Return,
}

impl Opcode {
    pub const COUNT: usize = 50;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Constant,
        Self::Nil,
        Self::True,
        Self::False,
        Self::LoadLocal,
        Self::StoreLocal,
        Self::Pop,
        Self::Dup,
        Self::IntrinsicCall,
        Self::Jump,
        Self::JumpIfFalse,
        Self::Closure,
        Self::Call,
        Self::CallStatic,
        Self::Throw,
        Self::Rethrow,
        Self::GetGlobal,
        Self::DefGlobal,
        Self::SetGlobal,
        Self::VarGlobal,
        Self::DeclareGlobal,
        Self::DefStruct,
        Self::DefMutable,
        Self::MutableFieldGet,
        Self::MutableFieldSet,
        Self::InstanceOf,
        Self::MakeMultiArity,
        Self::BuildVector,
        Self::BuildMap,
        Self::BuildSet,
        Self::BuildList,
        Self::ConcatList,
        Self::ToVector,
        Self::DefMacro,
        Self::DefProtocol,
        Self::ExtendType,
        Self::DefMulti,
        Self::DefMethod,
        Self::IntrinsicValue,
        Self::BuiltinValue,
        Self::NamespaceValue,
        Self::NamespaceOperation,
        Self::DynamicBind,
        Self::DynamicUnbind,
        Self::Await,
        Self::HostCall,
        Self::DotCall,
        Self::ProtocolCall,
        Self::Yield,
        Self::Return,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn as_keyword(self) -> &'static str {
        match self {
            Self::Constant => "constant",
            Self::Nil => "nil",
            Self::True => "true",
            Self::False => "false",
            Self::LoadLocal => "load-local",
            Self::StoreLocal => "store-local",
            Self::Pop => "pop",
            Self::Dup => "dup",
            Self::IntrinsicCall => "intrinsic-call",
            Self::Jump => "jump",
            Self::JumpIfFalse => "jump-if-false",
            Self::Closure => "closure",
            Self::Call => "call",
            Self::CallStatic => "call-static",
            Self::Throw => "throw",
            Self::Rethrow => "rethrow",
            Self::GetGlobal => "get-global",
            Self::DefGlobal => "def-global",
            Self::SetGlobal => "set-global",
            Self::VarGlobal => "var-global",
            Self::DeclareGlobal => "declare-global",
            Self::DefStruct => "def-struct",
            Self::DefMutable => "def-mutable",
            Self::MutableFieldGet => "mutable-field-get",
            Self::MutableFieldSet => "mutable-field-set",
            Self::InstanceOf => "instance-of",
            Self::MakeMultiArity => "make-multi-arity",
            Self::BuildVector => "build-vector",
            Self::BuildMap => "build-map",
            Self::BuildSet => "build-set",
            Self::BuildList => "build-list",
            Self::ConcatList => "concat-list",
            Self::ToVector => "to-vector",
            Self::DefMacro => "def-macro",
            Self::DefProtocol => "def-protocol",
            Self::ExtendType => "extend-type",
            Self::DefMulti => "def-multi",
            Self::DefMethod => "def-method",
            Self::IntrinsicValue => "intrinsic-value",
            Self::BuiltinValue => "builtin-value",
            Self::NamespaceValue => "namespace-value",
            Self::NamespaceOperation => "namespace-operation",
            Self::DynamicBind => "dynamic-bind",
            Self::DynamicUnbind => "dynamic-unbind",
            Self::Await => "await",
            Self::HostCall => "host-call",
            Self::DotCall => "dot-call",
            Self::ProtocolCall => "protocol-call",
            Self::Yield => "yield",
            Self::Return => "return",
        }
    }

    pub(super) fn from_instruction(instruction: &Instruction) -> Self {
        match instruction {
            Instruction::Constant(_) => Self::Constant,
            Instruction::Nil => Self::Nil,
            Instruction::True => Self::True,
            Instruction::False => Self::False,
            Instruction::LoadLocal(_) => Self::LoadLocal,
            Instruction::StoreLocal(_) => Self::StoreLocal,
            Instruction::Pop => Self::Pop,
            Instruction::Dup => Self::Dup,
            Instruction::IntrinsicCall { .. } => Self::IntrinsicCall,
            Instruction::Jump(_) => Self::Jump,
            Instruction::JumpIfFalse(_) => Self::JumpIfFalse,
            Instruction::Closure { .. } => Self::Closure,
            Instruction::Call { .. } => Self::Call,
            Instruction::CallStatic { .. } => Self::CallStatic,
            Instruction::Throw => Self::Throw,
            Instruction::Rethrow => Self::Rethrow,
            Instruction::GetGlobal(_) => Self::GetGlobal,
            Instruction::DefGlobal { .. } => Self::DefGlobal,
            Instruction::SetGlobal(_) => Self::SetGlobal,
            Instruction::VarGlobal(_) => Self::VarGlobal,
            Instruction::DeclareGlobal(_) => Self::DeclareGlobal,
            Instruction::DefStruct { .. } => Self::DefStruct,
            Instruction::DefMutable { .. } => Self::DefMutable,
            Instruction::MutableFieldGet(_) => Self::MutableFieldGet,
            Instruction::MutableFieldSet(_) => Self::MutableFieldSet,
            Instruction::InstanceOf => Self::InstanceOf,
            Instruction::MakeMultiArity { .. } => Self::MakeMultiArity,
            Instruction::BuildVector(_) => Self::BuildVector,
            Instruction::BuildMap(_) => Self::BuildMap,
            Instruction::BuildSet(_) => Self::BuildSet,
            Instruction::BuildList(_) => Self::BuildList,
            Instruction::ConcatList(_) => Self::ConcatList,
            Instruction::ToVector => Self::ToVector,
            Instruction::DefMacro { .. } => Self::DefMacro,
            Instruction::DefProtocol(_) => Self::DefProtocol,
            Instruction::ExtendType(_) => Self::ExtendType,
            Instruction::DefMulti(_) => Self::DefMulti,
            Instruction::DefMethod(_) => Self::DefMethod,
            Instruction::IntrinsicValue(_) => Self::IntrinsicValue,
            Instruction::BuiltinValue(_) => Self::BuiltinValue,
            Instruction::NamespaceValue(_) => Self::NamespaceValue,
            Instruction::NamespaceOperation(_) => Self::NamespaceOperation,
            Instruction::DynamicBind(_) => Self::DynamicBind,
            Instruction::DynamicUnbind(_) => Self::DynamicUnbind,
            Instruction::Await => Self::Await,
            Instruction::HostCall => Self::HostCall,
            Instruction::DotCall { .. } => Self::DotCall,
            Instruction::ProtocolCall { .. } => Self::ProtocolCall,
            Instruction::Yield => Self::Yield,
            Instruction::Return => Self::Return,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstructionEvent {
    pub function: u16,
    pub ip: u32,
    pub opcode: Opcode,
    pub stack_depth: u32,
    pub call_depth: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    CallEnter,
    CallReturn,
    ExceptionUnwind,
    MachineSuspend,
    MachineResume,
}

impl TransitionKind {
    pub const fn as_keyword(self) -> &'static str {
        match self {
            Self::CallEnter => "call/enter",
            Self::CallReturn => "call/return",
            Self::ExceptionUnwind => "exception/unwind",
            Self::MachineSuspend => "machine/suspend",
            Self::MachineResume => "machine/resume",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionEvent {
    pub kind: TransitionKind,
    pub from_function: u16,
    pub from_ip: u32,
    pub to_function: u16,
    pub to_ip: u32,
    pub stack_depth: u32,
    pub call_depth: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalKind {
    Return,
    Fail,
}

impl TerminalKind {
    pub const fn as_keyword(self) -> &'static str {
        match self {
            Self::Return => "machine/return",
            Self::Fail => "machine/fail",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalEvent {
    pub kind: TerminalKind,
    pub function: u16,
    pub ip: u32,
    pub stack_depth: u32,
    pub call_depth: u16,
}

pub trait VmProbe {
    #[inline(always)]
    fn on_instruction(&mut self, _event: InstructionEvent) {}

    #[inline(always)]
    fn on_transition(&mut self, _event: TransitionEvent) {}

    #[inline(always)]
    fn on_terminal(&mut self, _event: TerminalEvent) {}
}

#[derive(Default)]
pub struct NoProbe;

impl VmProbe for NoProbe {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpcodeCount {
    pub opcode: &'static str,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeMetrics {
    pub schema: &'static str,
    pub instructions: u64,
    pub opcode_counts: [u64; Opcode::COUNT],
    pub calls: u64,
    pub returns: u64,
    pub unwinds: u64,
    pub suspensions: u64,
    pub resumptions: u64,
    pub terminal_returns: u64,
    pub failures: u64,
    pub max_stack_depth: u32,
    pub max_call_depth: u16,
}

impl Default for BytecodeMetrics {
    fn default() -> Self {
        Self {
            schema: BYTECODE_METRICS_SCHEMA,
            instructions: 0,
            opcode_counts: [0; Opcode::COUNT],
            calls: 0,
            returns: 0,
            unwinds: 0,
            suspensions: 0,
            resumptions: 0,
            terminal_returns: 0,
            failures: 0,
            max_stack_depth: 0,
            max_call_depth: 0,
        }
    }
}

impl BytecodeMetrics {
    pub fn opcode_count(&self, opcode: Opcode) -> u64 {
        self.opcode_counts[opcode.index()]
    }

    pub fn named_opcode_counts(&self) -> impl Iterator<Item = OpcodeCount> + '_ {
        Opcode::ALL.into_iter().filter_map(|opcode| {
            let count = self.opcode_count(opcode);
            (count > 0).then_some(OpcodeCount {
                opcode: opcode.as_keyword(),
                count,
            })
        })
    }
}

#[derive(Default)]
pub struct CounterProbe {
    metrics: BytecodeMetrics,
}

impl CounterProbe {
    pub fn metrics(&self) -> &BytecodeMetrics {
        &self.metrics
    }

    pub fn into_metrics(self) -> BytecodeMetrics {
        self.metrics
    }

    pub fn opcode_count(&self, opcode: Opcode) -> u64 {
        self.metrics.opcode_count(opcode)
    }

    fn observe_depths(&mut self, stack_depth: u32, call_depth: u16) {
        self.metrics.max_stack_depth = self.metrics.max_stack_depth.max(stack_depth);
        self.metrics.max_call_depth = self.metrics.max_call_depth.max(call_depth);
    }
}

impl VmProbe for CounterProbe {
    #[inline(always)]
    fn on_instruction(&mut self, event: InstructionEvent) {
        self.metrics.instructions = self.metrics.instructions.saturating_add(1);
        self.metrics.opcode_counts[event.opcode.index()] =
            self.metrics.opcode_counts[event.opcode.index()].saturating_add(1);
        self.observe_depths(event.stack_depth, event.call_depth);
    }

    #[inline(always)]
    fn on_transition(&mut self, event: TransitionEvent) {
        match event.kind {
            TransitionKind::CallEnter => self.metrics.calls = self.metrics.calls.saturating_add(1),
            TransitionKind::CallReturn => {
                self.metrics.returns = self.metrics.returns.saturating_add(1)
            }
            TransitionKind::ExceptionUnwind => {
                self.metrics.unwinds = self.metrics.unwinds.saturating_add(1)
            }
            TransitionKind::MachineSuspend => {
                self.metrics.suspensions = self.metrics.suspensions.saturating_add(1)
            }
            TransitionKind::MachineResume => {
                self.metrics.resumptions = self.metrics.resumptions.saturating_add(1)
            }
        }
        self.observe_depths(event.stack_depth, event.call_depth);
    }

    #[inline(always)]
    fn on_terminal(&mut self, event: TerminalEvent) {
        match event.kind {
            TerminalKind::Return => {
                self.metrics.terminal_returns = self.metrics.terminal_returns.saturating_add(1)
            }
            TerminalKind::Fail => self.metrics.failures = self.metrics.failures.saturating_add(1),
        }
        self.observe_depths(event.stack_depth, event.call_depth);
    }
}

#[cfg(test)]
#[path = "instrumentation/tests.rs"]
mod tests;
