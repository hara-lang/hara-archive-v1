use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iabort",
    name = "IAbort",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IAbort {
    type Error;
    type Output;

    #[hara_method(value = "abort", arity = 2)]
    fn abort(&self, error: Self::Error) -> Self::Output;
}
