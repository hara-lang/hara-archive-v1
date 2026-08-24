use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iclosed",
    name = "IClosed",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IClosed {
    #[hara_method(value = "closed?", arity = 1)]
    fn closed(&self) -> bool;
}
