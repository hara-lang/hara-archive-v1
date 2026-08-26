use std::cell::Cell;
use std::hash::Hash;

use crate::lang::data::{Map, Vector};
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash, IMetadata,
    IMutable, INth, IObjType, IPersistent, IToMutable, IToPersistent, ObjType,
};

const COMPACT_MINIMUM: usize = 32;

#[derive(Debug, Clone)]
pub struct Standard<E> {
    lookup: Map<E, usize>,
    order: Vector<Option<E>>,
}
impl<E: Clone + Eq + Hash> Default for Standard<E> {
    fn default() -> Self {
        Self {
            lookup: Map::new(),
            order: Vector::new(),
        }
    }
}
impl<E: Clone + Eq + Hash> Standard<E> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.lookup.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lookup.is_empty()
    }
    pub fn get(&self, value: &E) -> Option<&E> {
        self.lookup.find_entry(value).map(|(stored, _)| stored)
    }
    pub fn contains(&self, value: &E) -> bool {
        self.get(value).is_some()
    }
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.order.iter().filter_map(Option::as_ref)
    }
    pub fn conj_value(&self, value: E) -> Self {
        if self.lookup.get(&value).is_some() {
            return self.clone();
        }
        let index = self.order.len();
        Self {
            lookup: self.lookup.assoc_value(value.clone(), index),
            order: self.order.push_last(Some(value)),
        }
    }
    pub fn dissoc_value(&self, value: &E) -> Self {
        let Some(index) = self.lookup.get(value) else {
            return self.clone();
        };
        let set = Self {
            lookup: self.lookup.dissoc_value(value),
            order: self
                .order
                .assoc_value(*index, None)
                .expect("ordered-set index"),
        };
        set.compact_if_sparse()
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
impl<E: Clone + Eq + Hash> FromIterator<E> for Standard<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::new(), |set, value| set.conj_value(value))
    }
}
impl<E: Clone + Eq + Hash> From<Vec<E>> for Standard<E> {
    fn from(values: Vec<E>) -> Self {
        values.into_iter().collect()
    }
}
impl<E: Clone + Eq + Hash> PartialEq for Standard<E> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|v| other.contains(v))
    }
}
impl<E: Clone + Eq + Hash> ICount for Standard<E> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<E: Clone + Eq + Hash> IFind<E> for Standard<E> {
    type Output = E;
    fn find(&self, key: &E) -> Option<E> {
        self.get(key).cloned()
    }
}
impl<E: Clone + Eq + Hash> IConj<E> for Standard<E> {
    type Output = Self;
    fn conj(&self, value: E) -> Self {
        self.conj_value(value)
    }
}
impl<E: Clone + Eq + Hash> IDissoc<E> for Standard<E> {
    type Output = Self;
    fn dissoc(&self, key: &E) -> Self {
        self.dissoc_value(key)
    }
}
impl<E: Clone + Eq + Hash> INth<E> for Standard<E> {
    fn nth(&self, index: usize) -> Option<&E> {
        self.iter().nth(index)
    }
}
impl<E: Clone + Eq + Hash> IEmpty for Standard<E> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.lookup.meta().cloned())
    }
}
impl<E: Clone + Eq + Hash> IMetadata for Standard<E> {
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
}
impl<E: Clone + Eq + Hash> IPersistent for Standard<E> {}
impl<E: Clone + Eq + Hash> IntoIterator for Standard<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<E: Clone + Eq + Hash> IEquality for Standard<E> {
    fn equality(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|v| other.get(v).is_some())
    }
}
impl<E: Clone + Eq + Hash + std::fmt::Debug> IDisplay for Standard<E> {
    fn display(&self) -> String {
        format!(
            "#{{{}}}",
            self.iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<E: Clone + Eq + Hash + JavaHash> IHash for Standard<E> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java ISetType uses unordered item hashing: order-insensitive sum,
        // "::SET" seed (see lang::hash).
        crate::lang::hash::compose_unordered("SET", self.iter().map(|v| v.java_hash(hash_type)))
            as u64
    }
}
impl<E: Clone + Eq + Hash + std::fmt::Debug> IObjType for Standard<E> {
    fn obj_type(&self) -> ObjType {
        ObjType::Set
    }
}
impl<E> IColl<E> for Standard<E>
where
    E: Clone + Eq + Hash + JavaHash + std::fmt::Debug,
{
    fn start_string(&self) -> &'static str {
        "#{"
    }
    fn end_string(&self) -> &'static str {
        "}"
    }
}
impl<E: Clone + Eq + Hash> IToMutable for Standard<E> {
    type Mutable = Mutable<E>;
    fn to_mutable(&self) -> Self::Mutable {
        Mutable {
            editable: Cell::new(true),
            set: self.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mutable<E> {
    editable: Cell<bool>,
    set: Standard<E>,
}
impl<E: Clone + Eq + Hash> Mutable<E> {
    fn check(&self) {
        assert!(
            self.editable.get(),
            "mutable ordered set used after to_persistent"
        );
    }
    pub fn conj(&mut self, value: E) -> &mut Self {
        self.check();
        self.set = self.set.conj_value(value);
        self
    }
    pub fn dissoc(&mut self, value: &E) -> &mut Self {
        self.check();
        self.set = self.set.dissoc_value(value);
        self
    }
}
impl<E: Clone + Eq + Hash> std::ops::Deref for Mutable<E> {
    type Target = Standard<E>;
    fn deref(&self) -> &Self::Target {
        self.check();
        &self.set
    }
}
impl<E> IMutable for Mutable<E> {}
impl<E: Clone + Eq + Hash> IToPersistent for Mutable<E> {
    type Persistent = Standard<E>;
    fn to_persistent(&mut self) -> Self::Persistent {
        self.check();
        self.editable.set(false);
        self.set.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Standard;
    #[test]
    fn compaction_and_empty_preserve_metadata() {
        use crate::lang::protocol::{IEmpty, IMetadata};
        let original = (0..80)
            .collect::<Standard<_>>()
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        let reduced = (0..60).fold(original, |set, value| set.dissoc_value(&value));
        assert_eq!(reduced.meta().map(|m| m.doc().unwrap()), Some("doc"));
        assert_eq!(
            reduced.empty().meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
    }

    #[test]
    fn is_unique_ordered_and_persistent() {
        let original = [3, 1, 3, 2].into_iter().collect::<Standard<_>>();
        let reduced = original.dissoc_value(&1);
        assert_eq!(original.iter().copied().collect::<Vec<_>>(), vec![3, 1, 2]);
        assert_eq!(reduced.iter().copied().collect::<Vec<_>>(), vec![3, 2]);
    }

    #[test]
    fn compaction_triggers_at_java_threshold_and_renumbers() {
        // Java: compact on dissoc when order.count() > 32 &&
        // order.count() > 2 * lookup.count(), rebuilding both structures
        // and renumbering lookup indices 0..count-1.
        let mut set = (0..33).collect::<Standard<_>>();
        for value in 0..16 {
            set = set.dissoc_value(&value);
        }
        // order 33, lookup 17: 33 > 32 but 33 <= 2 * 17, no compaction
        assert_eq!(set.order.len(), 33);
        set = set.dissoc_value(&16);
        // order 33, lookup 16: 33 > 32 and 33 > 2 * 16, compact + renumber
        assert_eq!(set.order.len(), 16);
        assert_eq!(
            set.iter().copied().collect::<Vec<_>>(),
            (17..33).collect::<Vec<_>>()
        );
        // order at exactly 32 never compacts
        let mut small = (0..32).collect::<Standard<_>>();
        for value in 0..31 {
            small = small.dissoc_value(&value);
        }
        assert_eq!(small.order.len(), 32);
        assert_eq!(small.len(), 1);
    }

    #[test]
    fn dissoc_then_reconj_appends_at_end() {
        use crate::lang::protocol::{IFind, INth};
        let set = [1, 2, 3]
            .into_iter()
            .collect::<Standard<_>>()
            .dissoc_value(&2)
            .conj_value(2);
        assert_eq!(set.iter().copied().collect::<Vec<_>>(), vec![1, 3, 2]);
        assert_eq!(set.nth(1), Some(&3));
        assert_eq!(set.find(&2), Some(2));
        assert_eq!(set.find(&9), None);
        // duplicate conj keeps original position
        let dup = set.conj_value(1);
        assert_eq!(dup.iter().copied().collect::<Vec<_>>(), vec![1, 3, 2]);
    }
}
