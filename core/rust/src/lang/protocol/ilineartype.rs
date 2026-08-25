use hara_protocol_macros::hara_protocol;

/// Portable linear-collection category protocol.
#[hara_protocol(
    namespace = "std.protocol.ilineartype",
    name = "ILinearType",
    parents = [
        "ISequential",
        "IColl",
        "IPeekFirst",
        "IPeekLast",
        "ICons",
        "IConj",
        "INth",
        "ICount"
    ]
)]
pub trait ILinearType {}
