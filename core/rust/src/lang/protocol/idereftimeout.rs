use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.idereftimeout", name = "IDerefTimeout")]
pub trait IDerefTimeout<V> {
    #[hara_method(value = "deref-timeout", arity = 3)]
    fn deref_timeout(&self, milliseconds: u64, timeout_value: V) -> V;
}
