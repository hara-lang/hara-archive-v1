use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iencode", name = "IEncode")]
pub trait IEncode<O> {
    type Output;

    #[hara_method(value = "encode", arity = 2)]
    fn encode(&self, options: O) -> Self::Output;
}
