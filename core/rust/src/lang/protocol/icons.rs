use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.icons", name = "ICons")]
pub trait ICons<E> {
    type Output;

    #[hara_method(value = "cons", arity = 2)]
    fn cons(&self, value: E) -> Self::Output;
}
