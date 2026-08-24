use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.ideps", name = "IDeps")]
pub trait IDeps<K, E> {
    type Entries: Iterator<Item = K>;
    type Keys: Iterator<Item = K>;

    #[hara_method(value = "dep-get", arity = 2)]
    fn dep_get(&self, key: &K) -> Option<E>;
    #[hara_method(value = "dep-entries", arity = 2)]
    fn dep_entries(&self, key: &K) -> Self::Entries;
    #[hara_method(value = "dep-keys", arity = 1)]
    fn dep_keys(&self) -> Self::Keys;
}

#[cfg(test)]
mod tests {
    use super::IDeps;

    struct Fixture;

    impl IDeps<&'static str, &'static str> for Fixture {
        type Entries = std::vec::IntoIter<&'static str>;
        type Keys = std::array::IntoIter<&'static str, 2>;

        fn dep_get(&self, key: &&'static str) -> Option<&'static str> {
            (*key == "a").then_some("A")
        }

        fn dep_entries(&self, key: &&'static str) -> Self::Entries {
            if *key == "a" {
                vec!["b"].into_iter()
            } else {
                vec![].into_iter()
            }
        }

        fn dep_keys(&self) -> Self::Keys {
            ["a", "b"].into_iter()
        }
    }

    #[test]
    fn exposes_dependency_values_entries_and_keys() {
        let fixture = Fixture;
        assert_eq!(fixture.dep_get(&"a"), Some("A"));
        assert_eq!(fixture.dep_entries(&"a").collect::<Vec<_>>(), vec!["b"]);
        assert_eq!(fixture.dep_keys().collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
