use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iinvokein", name = "IInvokeIn")]
pub trait IInvokeIn<C, A> {
    type Output;

    #[hara_method(value = "invoke-in", arity = -1, variadic = true, min_arity = 2)]
    fn invoke_in(&self, context: &mut C, arguments: A) -> Self::Output;
}
