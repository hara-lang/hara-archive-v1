use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.icontextlifecycle",
    name = "IContextLifeCycle"
)]
pub trait IContextLifeCycle<M, P> {
    #[hara_method(value = "has-module?", arity = 2)]
    fn has_module(&self, module: &M) -> bool;
    #[hara_method(value = "setup-module", arity = 2)]
    fn setup_module(&mut self, module: M);
    #[hara_method(value = "teardown-module", arity = 2)]
    fn teardown_module(&mut self, module: &M);
    #[hara_method(value = "has-pointer?", arity = 2)]
    fn has_pointer(&self, pointer: &P) -> bool;
    #[hara_method(value = "setup-pointer", arity = 2)]
    fn setup_pointer(&mut self, pointer: P);
    #[hara_method(value = "teardown-pointer", arity = 2)]
    fn teardown_pointer(&mut self, pointer: &P);
}
