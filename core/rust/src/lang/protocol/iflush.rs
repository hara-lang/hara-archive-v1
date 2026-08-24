use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iflush",
    name = "IFlush",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IFlush {
    type Output;

    #[hara_method(value = "flush", arity = 1)]
    fn flush(&mut self) -> Self::Output;
}
