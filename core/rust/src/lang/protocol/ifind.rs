use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ifind", name = "IFind")]
pub trait IFind<K> {
    type Output;

    #[hara_method(value = "find", arity = 2)]
    fn find(&self, key: &K) -> Option<Self::Output>;

    fn has(&self, key: &K) -> bool {
        self.find(key).is_some()
    }
}
