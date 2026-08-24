use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iexinfo", name = "IExInfo")]
pub trait IExInfo {
    type Data;

    #[hara_method(value = "data", arity = 1)]
    fn data(&self) -> Self::Data;
}
