use super::IClose;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.istream",
    name = "IStream",
    parents = ["IClose"]
)]
pub trait IStream: IClose {
    type Item;

    #[hara_method(value = "next", arity = 1)]
    fn next(&mut self) -> Option<Self::Item>;
}
