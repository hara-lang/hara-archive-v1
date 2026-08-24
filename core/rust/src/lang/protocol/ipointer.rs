use hara_protocol_macros::hara_protocol;

/// Identifies the evaluator context that owns pointer resolution.
///
/// Pointer descriptor fields are accessed through the ordinary collection
/// protocols; they are deliberately not duplicated here.
#[hara_protocol(namespace = "std.protocol.ipointer", name = "IPointer")]
pub trait IPointer<C> {
    #[hara_method(value = "ptr-context", arity = 1)]
    fn pointer_context(&self) -> C;
}
