use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iiter", name = "IIter")]
pub trait IIter {
    type Item;
    type Iter: Iterator<Item = Self::Item>;

    #[hara_method(value = "iter", arity = 1)]
    fn iter(&self) -> Self::Iter;
}
