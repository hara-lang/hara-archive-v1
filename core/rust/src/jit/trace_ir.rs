use crate::core::{IntrinsicOp, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum TraceValue {
    I64(i64),
    Bool(bool),
    Nil,
    Indexed(Box<Value>),
    VectorSlice(Box<NumericVectorSlice>),
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NumericVectorSlice {
    pub values: Vec<i64>,
    pub start: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TraceOp {
    GuardLocalI64 { local: u16 },
    GuardLocalBool { local: u16 },
    GuardLocalNil { local: u16 },
    GuardLocalVectorI64 { local: u16 },
    LoadLocal { local: u16 },
    ConstantI64(i64),
    ConstantBool(bool),
    ConstantNil,
    ConstantVectorI64 { vector: u16 },
    BinaryI64(IntrinsicOp),
    VectorCountI64,
    VectorFirstI64,
    VectorRestI64,
    VectorSecondI64,
    VectorNthI64,
    StoreLocal { local: u16 },
    GuardTruthy { expected: bool },
    Pop,
    LoopBackedge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    pub function: u16,
    pub header: u32,
    pub resume_ip: u32,
    pub operations: Vec<TraceOp>,
    pub vectors: Vec<Vec<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReason {
    WrongTag,
    BranchChanged,
    Overflow,
    DivisionByZero,
    IndexOutOfBounds,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExitSnapshot {
    pub function: u16,
    pub instruction: u32,
    pub locals: Vec<TraceValue>,
    pub stack: Vec<TraceValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TraceOutcome {
    Completed {
        iterations: u32,
    },
    SideExit {
        reason: ExitReason,
        iterations: u32,
        snapshot: ExitSnapshot,
    },
}

#[cfg(test)]
mod tests {
    use super::TraceValue;

    #[test]
    fn trace_values_keep_heap_values_indirect() {
        assert!(std::mem::size_of::<TraceValue>() <= 16);
    }
}
