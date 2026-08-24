use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ideref", name = "IDeref")]
pub trait IDeref {
    type Output;

    #[hara_method(value = "deref", arity = 1)]
    fn deref(&self) -> Self::Output;
}
