use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.icontext", name = "IContext")]
pub trait IContext<A> {
    type Output;

    #[hara_method(value = "call", arity = -1, variadic = true, min_arity = 1)]
    fn call(&mut self, arguments: A) -> Self::Output;
}
