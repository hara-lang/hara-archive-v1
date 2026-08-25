use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iassoc", name = "IAssoc")]
pub trait IAssoc<K, V> {
    type Output;

    #[hara_method(value = "assoc", arity = 3, whole_wasm)]
    fn assoc(&self, key: K, value: V) -> Self::Output;
}
