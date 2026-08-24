use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iapplicable", name = "IApplicable")]
pub trait IApplicable<R, A> {
    type Output;

    #[hara_method(value = "apply-in", arity = 3)]
    fn apply_in(&self, runtime: &mut R, arguments: A) -> Self::Output;
    #[hara_method(value = "apply-default", arity = 1)]
    fn apply_default(&mut self) -> &mut R;
    #[hara_method(value = "transform-in", arity = 3)]
    fn transform_in(&self, runtime: &R, arguments: A) -> A;
    #[hara_method(value = "transform-out", arity = 4)]
    fn transform_out(&self, runtime: &R, arguments: A, value: Self::Output) -> Self::Output;
}
