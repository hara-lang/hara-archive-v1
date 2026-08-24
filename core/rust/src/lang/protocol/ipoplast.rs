use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ipoplast", name = "IPopLast")]
pub trait IPopLast {
    type Output;

    #[hara_method(value = "pop-last", arity = 1)]
    fn pop_last(&self) -> Self::Output;
}
