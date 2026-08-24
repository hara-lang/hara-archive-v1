use super::IClose;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.icoroutine",
    name = "ICoroutine",
    parents = ["IClose"]
)]
pub trait ICoroutine<A>: IClose {
    type Status;
    type Output;

    #[hara_method(value = "status", arity = 1)]
    fn status(&self) -> Self::Status;
    #[hara_method(value = "resume", arity = -1, variadic = true, min_arity = 1)]
    fn resume(&self, arguments: A) -> Result<Self::Output, Self::Error>;
}
