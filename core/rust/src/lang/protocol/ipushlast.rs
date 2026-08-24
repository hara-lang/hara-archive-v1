use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ipushlast", name = "IPushLast")]
pub trait IPushLast<E> {
    type Output;

    #[hara_method(value = "push-last", arity = 2)]
    fn push_last(&self, value: E) -> Self::Output;
}
