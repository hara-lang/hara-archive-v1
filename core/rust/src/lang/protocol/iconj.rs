use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iconj", name = "IConj")]
pub trait IConj<E> {
    type Output;

    #[hara_method(value = "conj", arity = 2)]
    fn conj(&self, value: E) -> Self::Output;
}
