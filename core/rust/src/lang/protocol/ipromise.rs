use super::{IDeref, IDerefTimeout};
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.ipromise",
    name = "IPromise",
    parents = ["IDeref", "IDerefTimeout"]
)]
pub trait IPromise<V, F>: IDeref<Output = V> + IDerefTimeout<V> + Sized {
    type State;
    type Error;

    #[hara_method(value = "state", arity = 1)]
    fn state(&self) -> Self::State;
    #[hara_method(value = "value", arity = 1)]
    fn value(&self) -> Result<V, Self::Error>;
    #[hara_method(value = "then", arity = 2)]
    fn then(&self, function: F) -> Self;
    #[hara_method(value = "catch", arity = 2)]
    fn catch(&self, function: F) -> Self;
    #[hara_method(value = "finally", arity = 2)]
    fn r#finally(&self, function: F) -> Self;
    #[hara_method(value = "cancel", arity = 1)]
    fn cancel(&self) -> bool;
}
