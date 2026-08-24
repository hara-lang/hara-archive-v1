use super::IFn;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iofn", name = "IOFn", parents = ["IFn"])]
pub trait IOFn<A>: IFn<A> {}
