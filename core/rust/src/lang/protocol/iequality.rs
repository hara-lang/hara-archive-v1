use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iequality", name = "IEquality")]
pub trait IEquality<Rhs: ?Sized = Self> {
    #[hara_method(value = "equality", arity = 2)]
    fn equality(&self, other: &Rhs) -> bool;
}
