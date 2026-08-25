use super::IFind;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.ilookup",
    name = "ILookup",
    parents = ["IFind"]
)]
pub trait ILookup<K, V>: IFind<K, Output = (K, V)>
where
    K: Clone,
    V: Clone,
{
    type Keys: Iterator<Item = K>;
    type Values: Iterator<Item = V>;
    fn keys(&self) -> Self::Keys;
    fn vals(&self) -> Self::Values;
    #[hara_method(
        value = "lookup",
        arity = -1,
        variadic = true,
        min_arity = 2,
        max_arity = 3,
        whole_wasm
    )]
    fn lookup(&self, key: &K) -> Option<V> {
        self.find(key).map(|(_, value)| value)
    }
    fn lookup_or(&self, key: &K, not_found: V) -> V {
        self.lookup(key).unwrap_or(not_found)
    }
}
