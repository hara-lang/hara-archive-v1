use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.idisplay", name = "IDisplay")]
pub trait IDisplay {
    #[hara_method(value = "display", arity = 1)]
    fn display(&self) -> String;
}
