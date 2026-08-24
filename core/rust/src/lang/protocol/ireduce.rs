use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ireduce", name = "IReduce")]
pub trait IReduce<A, F> {
    type Error;

    #[hara_method(value = "reduce", arity = -1, variadic = true, min_arity = 2, max_arity = 3)]
    fn reduce(&self, function: F, initial: Option<A>) -> Result<A, Self::Error>;
}
