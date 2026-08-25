use super::{IConj, IDisplay, IEmpty, IEquality, IHash};
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.icoll",
    name = "IColl",
    parents = ["IEquality", "IConj", "IEmpty", "IHash", "IDisplay"],
    availability = "portable"
)]
pub trait IColl<E>:
    IntoIterator<Item = E> + IEquality + IConj<E> + IEmpty + IHash + IDisplay
{
    #[hara_method(value = "start-string", arity = 1)]
    fn start_string(&self) -> &'static str;
    #[hara_method(value = "end-string", arity = 1)]
    fn end_string(&self) -> &'static str;

    #[hara_method(value = "sep-string", arity = 1)]
    fn separator(&self) -> &'static str {
        " "
    }
}
