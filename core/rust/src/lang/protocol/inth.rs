use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.inth", name = "INth")]
pub trait INth<E> {
    #[hara_method(value = "nth", arity = 2, whole_wasm)]
    fn nth(&self, index: usize) -> Option<&E>;
}
