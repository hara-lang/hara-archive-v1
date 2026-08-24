use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ipopfirst", name = "IPopFirst")]
pub trait IPopFirst {
    type Output;

    #[hara_method(value = "pop-first", arity = 1)]
    fn pop_first(&self) -> Self::Output;
}
