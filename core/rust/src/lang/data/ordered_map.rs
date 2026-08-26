use std::cell::Cell;
use std::hash::Hash;

use crate::lang::data::{Map, Vector};
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IAssoc, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash,
    ILookup, IMetadata, IMutable, INth, IObjType, IPersistent, IToMutable, IToPersistent, MetaType,
    ObjType,
};

const COMPACT_MINIMUM: usize = 32;

#[derive(Debug, Clone)]
pub struct Standard<K, V> {
    lookup: Map<K, (usize, V)>,
    order: Vector<Option<(K, V)>>,
}

impl<K: Clone + Eq + Hash, V: Clone> Default for Standard<K, V> {
    fn default() -> Self {
        Self {
            lookup: Map::new(),
            order: Vector::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, V: Clone> Standard<K, V> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.lookup.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lookup.is_empty()
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        self.lookup.get(key).map(|(_, value)| value)
    }
    pub fn find_entry(&self, key: &K) -> Option<(&K, &V)> {
        self.lookup
            .find_entry(key)
            .map(|(stored, (_, value))| (stored, value))
    }
    pub fn iter(&self) -> impl Iterator<Item = &(K, V)> {
        self.order.iter().filter_map(Option::as_ref)
    }
    pub fn assoc_value(&self, key: K, value: V) -> Self {
        match self.lookup.get(&key) {
            None => {
                let index = self.order.len();
                Self {
                    lookup: self.lookup.assoc_value(key.clone(), (index, value.clone())),
                    order: self.order.push_last(Some((key, value))),
                }
            }
            Some((index, _)) => Self {
                lookup: self
                    .lookup
                    .assoc_value(key.clone(), (*index, value.clone())),
                order: self
                    .order
                    .assoc_value(*index, Some((key, value)))
                    .expect("ordered-map index"),
            },
        }
    }
    /// Associates into a consumed ordered map, reusing the CHAMP lookup path
    /// when it is uniquely owned while retaining persistent alias semantics.
    pub fn assoc_value_owned(mut self, key: K, value: V) -> Self {
        match self.lookup.get(&key).map(|(index, _)| *index) {
            None => {
                let index = self.order.len();
                self.lookup = self
                    .lookup
                    .assoc_value_owned(key.clone(), (index, value.clone()));
                self.order = self.order.push_last_owned(Some((key, value)));
            }
            Some(index) => {
                self.lookup = self
                    .lookup
                    .assoc_value_owned(key.clone(), (index, value.clone()));
                self.order = self
                    .order
                    .assoc_value_owned(index, Some((key, value)))
                    .expect("ordered-map index");
            }
        }
        self
    }
    pub fn dissoc_value(&self, key: &K) -> Self {
        let Some((index, _)) = self.lookup.get(key) else {
            return self.clone();
        };
        let map = Self {
            lookup: self.lookup.dissoc_value(key),
            order: self
                .order
                .assoc_value(*index, None)
                .expect("ordered-map index"),
        };
        map.compact_if_sparse()
    }
    fn compact_if_sparse(self) -> Self {
        if self.order.len() <= COMPACT_MINIMUM || self.order.len() <= 2 * self.lookup.len() {
            return self;
        }
        self.iter()
            .cloned()
            .collect::<Self>()
            .with_meta(self.lookup.meta().cloned())
    }
}

impl<K: Clone + Eq + Hash, V: Clone> FromIterator<(K, V)> for Standard<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::new(), |map, (key, value)| map.assoc_value(key, value))
    }
}
impl<K: Clone + Eq + Hash, V: Clone + PartialEq> PartialEq for Standard<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(key, value)| other.get(key) == Some(value))
    }
}
impl<K: Clone + Eq + Hash, V: Clone> ICount for Standard<K, V> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IFind<K> for Standard<K, V> {
    type Output = (K, V);
    fn find(&self, key: &K) -> Option<Self::Output> {
        self.find_entry(key).map(|(k, v)| (k.clone(), v.clone()))
    }
}
impl<K: Clone + Eq + Hash, V: Clone> ILookup<K, V> for Standard<K, V> {
    type Keys = std::vec::IntoIter<K>;
    type Values = std::vec::IntoIter<V>;
    fn keys(&self) -> Self::Keys {
        self.iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>()
            .into_iter()
    }
    fn vals(&self) -> Self::Values {
        self.iter()
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>()
            .into_iter()
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IAssoc<K, V> for Standard<K, V> {
    type Output = Self;
    fn assoc(&self, k: K, v: V) -> Self {
        self.assoc_value(k, v)
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IDissoc<K> for Standard<K, V> {
    type Output = Self;
    fn dissoc(&self, k: &K) -> Self {
        self.dissoc_value(k)
    }
}
impl<K: Clone + Eq + Hash, V: Clone> INth<(K, V)> for Standard<K, V> {
    fn nth(&self, i: usize) -> Option<&(K, V)> {
        self.iter().nth(i)
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IEmpty for Standard<K, V> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.lookup.meta().cloned())
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IMetadata for Standard<K, V> {
    type Metadata = std::rc::Rc<crate::lang::data::Metadata>;
    fn meta(&self) -> Option<&Self::Metadata> {
        self.lookup.meta()
    }
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            lookup: self.lookup.with_meta(metadata.clone()),
            order: self.order.with_meta(metadata),
        }
    }

    fn metatype(&self) -> MetaType {
        MetaType::Map
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IPersistent for Standard<K, V> {}
impl<K: Clone + Eq + Hash, V: Clone> IntoIterator for Standard<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IConj<(K, V)> for Standard<K, V> {
    type Output = Self;
    fn conj(&self, (k, v): (K, V)) -> Self {
        self.assoc_value(k, v)
    }
}
impl<K: Clone + Eq + Hash, V: Clone + PartialEq> IEquality for Standard<K, V> {
    fn equality(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}
impl<K: Clone + Eq + Hash + std::fmt::Debug, V: Clone + std::fmt::Debug> IDisplay
    for Standard<K, V>
{
    fn display(&self) -> String {
        format!(
            "{{{}}}",
            self.iter()
                .map(|(k, v)| format!("{k:?} {v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<K: Clone + Eq + Hash + JavaHash, V: Clone + Hash + JavaHash> IHash for Standard<K, V> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Same unordered entry composition as the hash map:
        // "::MAP" seed, sum of ordered-entry hashes (see lang::hash).
        crate::lang::hash::compose_unordered(
            "MAP",
            self.iter().map(|(k, v)| {
                crate::lang::hash::compose_entry(k.java_hash(hash_type), v.java_hash(hash_type))
            }),
        ) as u64
    }
}
impl<K: Clone + Eq + Hash + std::fmt::Debug, V: Clone + std::fmt::Debug> IObjType
    for Standard<K, V>
{
    fn obj_type(&self) -> ObjType {
        ObjType::Map
    }
}
impl<K, V> IColl<(K, V)> for Standard<K, V>
where
    K: Clone + Eq + Hash + JavaHash + std::fmt::Debug,
    V: Clone + PartialEq + Hash + JavaHash + std::fmt::Debug,
{
    fn start_string(&self) -> &'static str {
        "{"
    }
    fn end_string(&self) -> &'static str {
        "}"
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IToMutable for Standard<K, V> {
    type Mutable = Mutable<K, V>;
    fn to_mutable(&self) -> Self::Mutable {
        Mutable {
            editable: Cell::new(true),
            map: self.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mutable<K, V> {
    editable: Cell<bool>,
    map: Standard<K, V>,
}
impl<K: Clone + Eq + Hash, V: Clone> Mutable<K, V> {
    fn check(&self) {
        assert!(
            self.editable.get(),
            "mutable ordered map used after to_persistent"
        );
    }
    pub fn assoc(&mut self, key: K, value: V) -> &mut Self {
        self.check();
        self.map = self.map.assoc_value(key, value);
        self
    }
    pub fn dissoc(&mut self, key: &K) -> &mut Self {
        self.check();
        self.map = self.map.dissoc_value(key);
        self
    }
}
impl<K: Clone + Eq + Hash, V: Clone> std::ops::Deref for Mutable<K, V> {
    type Target = Standard<K, V>;
    fn deref(&self) -> &Self::Target {
        self.check();
        &self.map
    }
}
impl<K, V> IMutable for Mutable<K, V> {}
impl<K: Clone + Eq + Hash, V: Clone> IToPersistent for Mutable<K, V> {
    type Persistent = Standard<K, V>;
    fn to_persistent(&mut self) -> Self::Persistent {
        self.check();
        self.editable.set(false);
        self.map.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Standard;
    #[test]
    fn replacement_keeps_position_and_deletion_compacts() {
        let map = Standard::new()
            .assoc_value("a", 1)
            .assoc_value("b", 2)
            .assoc_value("a", 3);
        assert_eq!(
            map.iter().cloned().collect::<Vec<_>>(),
            vec![("a", 3), ("b", 2)]
        );
        use crate::lang::protocol::IMetadata;
        let original = (0..80)
            .map(|n| (n, n))
            .collect::<Standard<_, _>>()
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        let reduced = (0..60).fold(original.clone(), |map, key| map.dissoc_value(&key));
        assert_eq!(original.len(), 80);
        assert_eq!(reduced.meta().map(|m| m.doc().unwrap()), Some("doc"));
        assert_eq!(
            reduced.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            (60..80).collect::<Vec<_>>()
        );
    }

    #[test]
    fn compaction_triggers_at_java_threshold_and_renumbers() {
        // Java: compact on dissoc when order.count() > 32 &&
        // order.count() > 2 * lookup.count(), rebuilding both structures
        // and renumbering lookup indices 0..count-1.
        let mut map = (0..33).map(|n| (n, n)).collect::<Standard<_, _>>();
        for key in 0..16 {
            map = map.dissoc_value(&key);
        }
        // order 33, lookup 17: 33 > 32 but 33 <= 2 * 17, no compaction
        assert_eq!(map.order.len(), 33);
        map = map.dissoc_value(&16);
        // order 33, lookup 16: 33 > 32 and 33 > 2 * 16, compact + renumber
        assert_eq!(map.order.len(), 16);
        assert_eq!(
            map.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            (17..33).collect::<Vec<_>>()
        );
        // renumbered: overwriting the first live key writes slot 0
        let overwritten = map.assoc_value(17, 999);
        assert_eq!(overwritten.iter().next(), Some(&(17, 999)));
        // order at exactly 32 never compacts
        let mut small = (0..32).map(|n| (n, n)).collect::<Standard<_, _>>();
        for key in 0..31 {
            small = small.dissoc_value(&key);
        }
        assert_eq!(small.order.len(), 32);
        assert_eq!(small.len(), 1);
    }

    #[test]
    fn dissoc_then_reassoc_appends_at_end() {
        use crate::lang::protocol::{IFind, INth};
        let map = Standard::new()
            .assoc_value("a", 1)
            .assoc_value("b", 2)
            .assoc_value("c", 3)
            .dissoc_value(&"b")
            .assoc_value("b", 9);
        assert_eq!(
            map.iter().cloned().collect::<Vec<_>>(),
            vec![("a", 1), ("c", 3), ("b", 9)]
        );
        assert_eq!(map.nth(1), Some(&("c", 3)));
        assert_eq!(map.find(&"b"), Some(("b", 9)));
        assert_eq!(map.find(&"zz"), None);
    }
}
