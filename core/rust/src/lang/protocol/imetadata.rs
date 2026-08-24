use hara_protocol_macros::hara_protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetaType {
    Object,
    Map,
    String,
}

#[hara_protocol(
    namespace = "std.protocol.imetadata",
    name = "IMetadata",
    availability = "inventory-only"
)]
pub trait IMetadata: Sized {
    type Metadata: Clone;

    #[hara_method(value = "meta", arity = 1)]
    fn meta(&self) -> Option<&Self::Metadata>;
    #[hara_method(value = "with-meta", arity = 2)]
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self;

    #[hara_method(value = "metatype", arity = 1)]
    fn metatype(&self) -> MetaType {
        MetaType::Object
    }
}
