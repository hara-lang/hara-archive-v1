use super::{IAbort, IStream, IStreamWrite};
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.istreamduplex",
    name = "IStreamDuplex",
    parents = ["IStream", "IStreamWrite", "IAbort"],
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IStreamDuplex: IStream + IStreamWrite + IAbort {}
