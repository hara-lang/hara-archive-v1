use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ipair", name = "IPair")]
pub trait IPair<K, V> {
    #[hara_method(value = "key", arity = 1)]
    fn key(&self) -> &K;
    #[hara_method(value = "value", arity = 1)]
    fn value(&self) -> &V;
}
