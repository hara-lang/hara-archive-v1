use super::IPersistent;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.itomutable",
    name = "IToMutable",
    parents = ["IPersistent"]
)]
pub trait IToMutable: IPersistent {
    type Mutable;

    #[hara_method(value = "to-mutable", arity = 1)]
    fn to_mutable(&self) -> Self::Mutable;
}
