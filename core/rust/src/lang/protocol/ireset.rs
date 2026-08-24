use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ireset", name = "IReset")]
pub trait IReset<V> {
    type Error;

    #[hara_method(value = "reset", arity = 2)]
    fn reset(&self, value: V) -> Result<V, Self::Error>;
}
