use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iencodable", name = "IEncodable")]
pub trait IEncodable<V> {
    type Output;

    #[hara_method(value = "encode-with", arity = 2)]
    fn encode_with(&self, visitor: V) -> Self::Output;
}
