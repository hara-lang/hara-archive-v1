use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.icas", name = "ICas")]
pub trait ICas<V> {
    type Error;

    #[hara_method(value = "cas", arity = 3)]
    fn cas(&self, old_value: &V, new_value: V) -> Result<bool, Self::Error>;
}
