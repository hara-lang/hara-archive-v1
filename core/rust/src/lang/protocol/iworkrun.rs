use super::{IClosed, IStream, IWorkRef};
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iworkrun",
    name = "IWorkRun",
    parents = ["IWorkRef", "IClosed"],
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IWorkRun: IWorkRef + IClosed {
    type Status;
    type Promise;
    type Options;
    type Stream: IStream;
    type Reason;

    #[hara_method(value = "work-status", arity = 1)]
    fn work_status(&self) -> Self::Status;
    #[hara_method(value = "work-result", arity = 1)]
    fn work_result(&self) -> Self::Promise;
    #[hara_method(value = "work-events", arity = 2)]
    fn work_events(&self, options: Self::Options) -> Self::Stream;
    #[hara_method(value = "work-cancel", arity = 2)]
    fn work_cancel(&self, reason: Self::Reason) -> Self::Promise;
}
