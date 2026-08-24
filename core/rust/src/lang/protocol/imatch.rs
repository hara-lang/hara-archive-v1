use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.imatch",
    name = "IMatch",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IMatch {
    type Value;
    type Output;

    #[hara_method(value = "match-value", arity = 2)]
    fn match_value(&self, value: Self::Value) -> Self::Output;
}
