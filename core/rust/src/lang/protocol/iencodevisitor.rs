use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iencodevisitor", name = "IEncodeVisitor")]
pub trait IEncodeVisitor {
    type Value;
    type Output;

    #[hara_method(value = "visit-nil", arity = 1)]
    fn visit_nil(&mut self) -> Self::Output;
    #[hara_method(value = "visit-boolean", arity = 2)]
    fn visit_boolean(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-number", arity = 2)]
    fn visit_number(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-character", arity = 2)]
    fn visit_character(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-string", arity = 2)]
    fn visit_string(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-keyword", arity = 2)]
    fn visit_keyword(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-symbol", arity = 2)]
    fn visit_symbol(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-seq", arity = 2)]
    fn visit_seq(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-vector", arity = 2)]
    fn visit_vector(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-map", arity = 2)]
    fn visit_map(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-set", arity = 2)]
    fn visit_set(&mut self, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-tagged", arity = 3)]
    fn visit_tagged(&mut self, tag: Self::Value, value: Self::Value) -> Self::Output;
    #[hara_method(value = "visit-unknown", arity = 2)]
    fn visit_unknown(&mut self, value: Self::Value) -> Self::Output;
}
