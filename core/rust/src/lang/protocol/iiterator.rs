use super::IIter;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iiterator",
    name = "IIterator",
    parents = ["IIter"]
)]
pub trait IIterator: IIter + Iterator<Item = <Self as IIter>::Item> {
    #[hara_method(value = "iter-next", arity = 1)]
    fn iter_next(&mut self) -> Option<<Self as IIter>::Item> {
        self.next()
    }

    #[hara_method(value = "iter-next?", arity = 1)]
    fn iter_next_available(&mut self) -> bool;
}
