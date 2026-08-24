use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iindexedkv", name = "IIndexedKV")]
pub trait IIndexedKV<K, V> {
    #[hara_method(value = "index-of-key", arity = 2)]
    fn index_of_key(&self, key: &K) -> Option<usize>;
    #[hara_method(value = "index-of-val", arity = 2)]
    fn index_of_val(&self, value: &V) -> Option<usize>;
}
