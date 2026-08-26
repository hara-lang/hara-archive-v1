use std::cell::Cell;
use std::hash::Hash;

use crate::lang::data::Map;
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash, IMetadata,
    IMutable, IObjType, IPersistent, IToMutable, IToPersistent, ObjType,
};

#[derive(Debug, Clone)]
pub struct Standard<E> {
    lookup: Map<E, E>,
}
impl<E> Default for Standard<E> {
    fn default() -> Self {
        Self {
            lookup: Map::default(),
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
        self.lookup.get(value)
    }
    pub fn contains(&self, value: &E) -> bool {
        self.get(value).is_some()
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
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.lookup.entries().into_iter().map(|(key, _)| key)
    }
}
impl<E: Clone + Eq + Hash> FromIterator<E> for Standard<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::new(), |set, value| set.conj_value(value))
    }
}
impl<E: Clone + Eq + Hash> IntoIterator for Standard<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<E: Clone + Eq + Hash> From<Vec<E>> for Standard<E> {
    fn from(values: Vec<E>) -> Self {
        values.into_iter().collect()
    }
}
impl<E: Clone + Eq + Hash> PartialEq for Standard<E> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|v| other.get(v).is_some())
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
impl<E: Clone + Eq + Hash> IEmpty for Standard<E> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self {
            lookup: self.lookup.empty(),
        }
    }
}
impl<E: Clone + Eq + Hash> IMetadata for Standard<E> {
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
impl<E: Clone + Eq + Hash> IPersistent for Standard<E> {}
impl<E: Clone + Eq + Hash> IEquality for Standard<E> {
    fn equality(&self, other: &Self) -> bool {
        self == other
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
    fn to_mutable(&self) -> Mutable<E> {
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
        assert!(self.editable.get(), "mutable set used after to_persistent")
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
    fn to_persistent(&mut self) -> Standard<E> {
        self.check();
        self.editable.set(false);
        self.set.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Standard;
    #[test]
    fn persistent_operations_preserve_metadata() {
        use crate::lang::protocol::{IEmpty, IMetadata};
        let set = Standard::from(vec![1, 2])
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        for value in [set.conj_value(3), set.dissoc_value(&1), set.empty()] {
            assert_eq!(value.meta().map(|m| m.doc().unwrap()), Some("doc"));
        }
    }

    #[test]
    fn is_map_backed_unique_and_persistent() {
        let a = [1, 2, 2].into_iter().collect::<Standard<_>>();
        let b = a.conj_value(3);
        let c = b.dissoc_value(&2);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 3);
        assert!(c.get(&2).is_none());
        assert!(a.get(&2).is_some());
    }
}
