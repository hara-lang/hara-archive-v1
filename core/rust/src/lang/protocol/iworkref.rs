use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iworkref",
    name = "IWorkRef",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IWorkRef {
    type Id;

    #[hara_method(value = "work-id", arity = 1)]
    fn work_id(&self) -> Self::Id;
}
