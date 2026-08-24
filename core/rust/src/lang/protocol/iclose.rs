use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iclose", name = "IClose")]
pub trait IClose {
    type Error;

    #[hara_method(value = "close", arity = 1)]
    fn close(&mut self) -> Result<(), Self::Error>;
}
