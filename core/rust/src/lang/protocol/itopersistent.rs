use super::IMutable;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.itopersistent",
    name = "IToPersistent",
    parents = ["IMutable"]
)]
pub trait IToPersistent: IMutable {
    type Persistent;

    #[hara_method(value = "to-persistent", arity = 1)]
    fn to_persistent(&mut self) -> Self::Persistent;
}
