use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.istreamoffer",
    name = "IStreamOffer",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IStreamOffer {
    type Value;

    #[hara_method(value = "offer", arity = 2)]
    fn offer(&mut self, value: Self::Value) -> bool;
}
