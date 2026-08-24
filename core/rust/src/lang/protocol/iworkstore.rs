use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iworkstore",
    name = "IWorkStore",
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IWorkStore {
    type Query;
    type Transition;
    type Output;

    #[hara_method(value = "work-query", arity = 2)]
    fn work_query(&self, query: Self::Query) -> Self::Output;
    #[hara_method(value = "work-transact", arity = 2)]
    fn work_transact(&self, transition: Self::Transition) -> Self::Output;
}
