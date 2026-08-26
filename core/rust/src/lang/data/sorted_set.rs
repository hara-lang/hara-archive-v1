use std::cell::Cell;

use crate::lang::data::SortedMap;
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash, IMetadata,
    IMutable, INth, IObjType, IPersistent, IToMutable, IToPersistent, ObjType,
};

#[derive(Debug, Clone)]
pub struct Standard<E> {
    lookup: SortedMap<E, E>,
}
impl<E> Default for Standard<E> {
    fn default() -> Self {
        Self {
            lookup: SortedMap::default(),
        }
    }
}
impl<E: Clone + Ord> Standard<E> {
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
        self.lookup.get(value)
    }
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.lookup.iter().map(|(key, _)| key)
    }
    pub fn conj_value(&self, value: E) -> Self {
        Self {
            lookup: self.lookup.assoc_value(value.clone(), value),
        }
    }
    pub fn dissoc_value(&self, value: &E) -> Self {
        Self {
            lookup: self.lookup.dissoc_value(value),
        }
    }
    pub fn index_of(&self, value: &E) -> Option<usize> {
        self.lookup.index_of_key(value)
    }
    pub fn inclusive_floor_index(&self, value: &E) -> Option<usize> {
        self.lookup.inclusive_floor_index(value)
    }
    pub fn ceil_index(&self, value: &E) -> Option<usize> {
        self.lookup.ceil_index(value)
    }
    pub fn slice(&self, min: &E, max: &E) -> Self {
        Self {
            lookup: self.lookup.slice(min, max),
        }
    }
}
impl<E: Clone + Ord> FromIterator<E> for Standard<E> {
    fn from_iter<T: IntoIterator<Item = E>>(it: T) -> Self {
        it.into_iter().fold(Self::new(), |s, v| s.conj_value(v))
    }
}
impl<E: Clone + Ord> ICount for Standard<E> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<E: Clone + Ord> IFind<E> for Standard<E> {
    type Output = E;
    fn find(&self, k: &E) -> Option<E> {
        self.get(k).cloned()
    }
}
impl<E: Clone + Ord> IConj<E> for Standard<E> {
    type Output = Self;
    fn conj(&self, v: E) -> Self {
        self.conj_value(v)
    }
}
impl<E: Clone + Ord> IDissoc<E> for Standard<E> {
    type Output = Self;
    fn dissoc(&self, k: &E) -> Self {
        self.dissoc_value(k)
    }
}
impl<E: Clone + Ord> INth<E> for Standard<E> {
    fn nth(&self, i: usize) -> Option<&E> {
        self.iter().nth(i)
    }
}
impl<E: Clone + Ord> IEmpty for Standard<E> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self {
            lookup: self.lookup.empty(),
        }
    }
}
impl<E: Clone + Ord> IMetadata for Standard<E> {
    type Metadata = std::rc::Rc<crate::lang::data::Metadata>;
    fn meta(&self) -> Option<&Self::Metadata> {
        self.lookup.meta()
    }
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            lookup: self.lookup.with_meta(metadata),
        }
    }
}
impl<E: Clone + Ord> IPersistent for Standard<E> {}
impl<E: Clone + Ord> IntoIterator for Standard<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<E: Clone + Ord> IEquality for Standard<E> {
    fn equality(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<E: Clone + Ord + std::fmt::Debug> IDisplay for Standard<E> {
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
impl<E: Clone + Ord + std::hash::Hash + JavaHash> IHash for Standard<E> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java ISetType uses unordered item hashing: order-insensitive sum,
        // "::SET" seed (see lang::hash).
        crate::lang::hash::compose_unordered("SET", self.iter().map(|v| v.java_hash(hash_type)))
            as u64
    }
}
impl<E: Clone + Ord + std::fmt::Debug> IObjType for Standard<E> {
    fn obj_type(&self) -> ObjType {
        ObjType::Set
    }
}
impl<E> IColl<E> for Standard<E>
where
    E: Clone + Ord + std::hash::Hash + JavaHash + std::fmt::Debug,
{
    fn start_string(&self) -> &'static str {
        "#{"
    }
    fn end_string(&self) -> &'static str {
        "}"
    }
}
impl<E: Clone + Ord> IToMutable for Standard<E> {
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
impl<E: Clone + Ord> Mutable<E> {
    fn check(&self) {
        assert!(
            self.editable.get(),
            "mutable sorted set used after to_persistent"
        )
    }
    pub fn conj(&mut self, v: E) -> &mut Self {
        self.check();
        self.set = self.set.conj_value(v);
        self
    }
    pub fn dissoc(&mut self, v: &E) -> &mut Self {
        self.check();
        self.set = self.set.dissoc_value(v);
        self
    }
}
impl<E: Clone + Ord> std::ops::Deref for Mutable<E> {
    type Target = Standard<E>;
    fn deref(&self) -> &Self::Target {
        self.check();
        &self.set
    }
}
impl<E> IMutable for Mutable<E> {}
impl<E: Clone + Ord> IToPersistent for Mutable<E> {
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
    fn updates_slices_empty_and_mutable_preserve_metadata() {
        use crate::lang::protocol::{IEmpty, IMetadata, IToMutable, IToPersistent};
        let set = [1, 2, 3]
            .into_iter()
            .collect::<Standard<_>>()
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        for value in [
            set.conj_value(4),
            set.dissoc_value(&1),
            set.slice(&1, &2),
            set.empty(),
        ] {
            assert_eq!(value.meta().map(|m| m.doc().unwrap()), Some("doc"));
        }
        let mut mutable = set.to_mutable();
        mutable.conj(4);
        assert_eq!(
            mutable.to_persistent().meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
    }

    #[test]
    fn deduplicates_and_sorts() {
        let set = [4, 1, 3, 1, 2].into_iter().collect::<Standard<_>>();
        assert_eq!(set.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert_eq!(set.index_of(&3), Some(2));
        assert_eq!(
            set.slice(&2, &3).iter().copied().collect::<Vec<_>>(),
            vec![2, 3]
        );
    }
}
