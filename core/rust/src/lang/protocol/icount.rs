use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.icount", name = "ICount")]
pub trait ICount {
    #[hara_method(value = "count", arity = 1, whole_wasm)]
    fn count(&self) -> usize;
}
