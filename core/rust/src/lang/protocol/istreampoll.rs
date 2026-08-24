use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.istreampoll",
    name = "IStreamPoll",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IStreamPoll {
    type Item;

    #[hara_method(value = "poll", arity = 1)]
    fn poll(&mut self) -> Option<Self::Item>;
}
