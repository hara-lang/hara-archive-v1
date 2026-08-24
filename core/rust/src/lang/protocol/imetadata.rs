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
    availability = "portable"
)]
pub trait IMetadata: Sized {
    type Metadata: Clone;

    fn meta(&self) -> Option<&Self::Metadata>;
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self;

    #[hara_method(value = "metatype", arity = 1)]
    fn metatype(&self) -> MetaType {
        MetaType::Object
    }
}
