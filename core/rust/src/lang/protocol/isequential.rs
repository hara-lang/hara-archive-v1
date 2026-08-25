use hara_protocol_macros::hara_protocol;

/// Portable ordered-sequence category protocol.
///
/// Sequential values are iterable and ordered, but the category deliberately
/// does not imply a count or positional lookup capability.
#[hara_protocol(
    namespace = "std.protocol.isequential",
    name = "ISequential",
    parents = ["IEquality", "IHash", "IObjType"]
)]
pub trait ISequential {}
