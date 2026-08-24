use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ipushfirst", name = "IPushFirst")]
pub trait IPushFirst<E> {
    type Output;

    #[hara_method(value = "push-first", arity = 2)]
    fn push_first(&self, value: E) -> Self::Output;
}
