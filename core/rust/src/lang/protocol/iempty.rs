use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iempty", name = "IEmpty")]
pub trait IEmpty {
    type Output;

    #[hara_method(value = "empty", arity = 1)]
    fn empty(&self) -> Self::Output;
}
