use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ipeekfirst", name = "IPeekFirst")]
pub trait IPeekFirst<E> {
    #[hara_method(value = "peek-first", arity = 1)]
    fn peek_first(&self) -> Option<E>;
}
