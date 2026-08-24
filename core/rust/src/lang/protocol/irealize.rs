use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.irealize", name = "IRealize")]
pub trait IRealize<V> {
    #[hara_method(value = "realized?", arity = 1)]
    fn is_realized(&self) -> bool;
    #[hara_method(value = "realize", arity = 1)]
    fn realize(&self) -> V;
}
