use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.idissoc", name = "IDissoc")]
pub trait IDissoc<K> {
    type Output;

    #[hara_method(value = "dissoc", arity = 2)]
    fn dissoc(&self, key: &K) -> Self::Output;
}
