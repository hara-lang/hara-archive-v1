use hara_protocol_macros::hara_protocol;

/// Portable map-category protocol.
///
/// The category is expressed through the protocol inheritance metadata.  The
/// Rust trait is intentionally marker-only because native values are
/// classified by the runtime registry rather than by Rust trait impls.
#[hara_protocol(
    namespace = "std.protocol.imaptype",
    name = "IMapType",
    parents = [
        "IColl",
        "ICount",
        "IObjType",
        "IMetadata",
        "ILookup",
        "IAssoc",
        "IDissoc",
        "IFind",
        "IFn"
    ]
)]
pub trait IMapType {}
