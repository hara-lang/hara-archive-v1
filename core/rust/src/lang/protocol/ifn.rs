use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ifn", name = "IFn")]
pub trait IFn<A> {
    type Output;

    #[hara_method(value = "invoke", arity = -1, variadic = true, min_arity = 1)]
    fn invoke(&self, arguments: A) -> Self::Output;
}
