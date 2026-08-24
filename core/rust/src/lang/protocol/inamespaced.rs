use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.inamespaced", name = "INamespaced")]
pub trait INamespaced {
    #[hara_method(value = "name", arity = 1)]
    fn get_name(&self) -> &str;
    #[hara_method(value = "namespace", arity = 1)]
    fn get_namespace(&self) -> Option<&str>;

    fn path_string(&self) -> String {
        match self.get_namespace() {
            Some(namespace) => format!("{namespace}/{}", self.get_name()),
            None => self.get_name().to_owned(),
        }
    }
}
