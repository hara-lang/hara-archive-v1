use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iindexed", name = "IIndexed")]
pub trait IIndexed<V> {
    type Index;

    #[hara_method(value = "index-of", arity = 2)]
    fn index_of(&self, value: &V) -> Option<Self::Index>;
}
