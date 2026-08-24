use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.istringlike",
    name = "IStringLike",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IStringLike {
    type Output;

    #[hara_method(value = "to-string", arity = 1)]
    fn to_string_value(&self) -> Self::Output;
    #[hara_method(value = "from-string", arity = 2)]
    fn from_string(&self, value: &str) -> Self::Output;
}
