use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iwork",
    name = "IWork",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IWork {
    type Spec;

    #[hara_method(value = "work-spec", arity = 1)]
    fn work_spec(&self) -> Self::Spec;
}
