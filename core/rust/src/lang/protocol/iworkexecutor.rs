use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iworkexecutor",
    name = "IWorkExecutor",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IWorkExecutor {
    type Request;
    type Output;

    #[hara_method(value = "work-execute", arity = 2)]
    fn work_execute(&self, request: Self::Request) -> Self::Output;
}
