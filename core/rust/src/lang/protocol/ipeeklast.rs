use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ipeeklast", name = "IPeekLast")]
pub trait IPeekLast<E> {
    #[hara_method(value = "peek-last", arity = 1)]
    fn peek_last(&self) -> Option<E>;
}
