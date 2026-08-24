use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.iwatch", name = "IWatch")]
pub trait IWatch<V: Clone> {
    type Key: Clone + Eq;
    type WatchEntry;

    #[hara_method(value = "watch-add", arity = 3)]
    fn add_watch(&self, key: Self::Key, watch: impl Fn(&Self::WatchEntry) + Send + Sync + 'static);
    #[hara_method(value = "watch-remove", arity = 2)]
    fn remove_watch(&self, key: &Self::Key);
    #[hara_method(value = "watch-list", arity = 1)]
    fn list_watches(&self) -> Vec<Self::WatchEntry> {
        Vec::new()
    }

    fn notify_watches(&self, old_value: V, new_value: V);
}
