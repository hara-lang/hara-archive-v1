use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.istreamwrite",
    name = "IStreamWrite",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IStreamWrite {
    type Value;
    type Output;

    #[hara_method(value = "write", arity = 2)]
    fn write(&mut self, value: Self::Value) -> Self::Output;
}
