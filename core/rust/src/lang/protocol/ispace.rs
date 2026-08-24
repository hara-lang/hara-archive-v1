use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ispace", name = "ISpace")]
pub trait ISpace<C, K, O> {
    type Runtime;

    #[hara_method(value = "context-set", arity = 4)]
    fn context_set(&mut self, context: C, key: K, options: O);
    #[hara_method(value = "context-unset", arity = 2)]
    fn context_unset(&mut self, context: &C);
    #[hara_method(value = "context-list", arity = 1)]
    fn context_list(&self) -> Vec<C>;
    #[hara_method(value = "context-get", arity = 2)]
    fn context_get(&self, context: &C) -> Option<O>;
    #[hara_method(value = "rt-active", arity = 1)]
    fn runtime_active(&self) -> Vec<Self::Runtime>;
    #[hara_method(value = "rt-get", arity = 2)]
    fn runtime_get(&self, context: &C) -> Option<Self::Runtime>;
    #[hara_method(value = "rt-start", arity = 2)]
    fn runtime_start(&mut self, context: C) -> Self::Runtime;
    #[hara_method(value = "rt-started?", arity = 2)]
    fn runtime_started(&self, context: &C) -> bool;
    #[hara_method(value = "rt-stopped?", arity = 2)]
    fn runtime_stopped(&self, context: &C) -> bool;
    #[hara_method(value = "rt-stop", arity = 2)]
    fn runtime_stop(&mut self, context: &C);
}
