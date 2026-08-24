use hara_protocol_macros::hara_protocol;

/// Portable set-category protocol.
#[hara_protocol(
    namespace = "std.protocol.isettype",
    name = "ISetType",
    parents = ["IColl", "IObjType", "IDissoc", "IFind", "IFn"]
)]
pub trait ISetType {}
