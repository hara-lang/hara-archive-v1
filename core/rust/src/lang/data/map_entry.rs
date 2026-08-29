use crate::core::Value;
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, ICount, IDisplay, IEquality, IHash, IMetadata, INth, IObjType, IPair, ObjType,
};
use std::rc::Rc;

/// The immutable runtime representation of a map entry.
///
/// Compact tuples deliberately remain a separate internal representation for
/// small vectors. A `MapEntry` is the only value that implements `IPair`.
#[derive(Debug, Clone)]
pub struct MapEntry {
    key: Value,
    value: Value,
    metadata: Option<Rc<crate::lang::data::Metadata>>,
}

impl MapEntry {
    pub fn new(key: Value, value: Value) -> Self {
        Self {
            key,
            value,
            metadata: None,
        }
    }

    pub fn key(&self) -> &Value {
        &self.key
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn nth(&self, index: usize) -> Option<&Value> {
        match index {
            0 => Some(&self.key),
            1 => Some(&self.value),
            _ => None,
        }
    }

    pub fn iter(&self) -> std::array::IntoIter<&Value, 2> {
        [&self.key, &self.value].into_iter()
    }
}

impl PartialEq for MapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.value == other.value
    }
}

impl Eq for MapEntry {}

impl ICount for MapEntry {
    fn count(&self) -> usize {
        2
    }
}

impl INth<Value> for MapEntry {
    fn nth(&self, index: usize) -> Option<&Value> {
        self.nth(index)
    }
}

impl IPair<Value, Value> for MapEntry {
    fn key(&self) -> &Value {
        self.key()
    }

    fn value(&self) -> &Value {
        self.value()
    }
}

impl IMetadata for MapEntry {
    type Metadata = Rc<crate::lang::data::Metadata>;

    fn meta(&self) -> Option<&Self::Metadata> {
        self.metadata.as_ref()
    }

    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            key: self.key.clone(),
            value: self.value.clone(),
            metadata,
        }
    }
}

impl IEquality for MapEntry {
    fn equality(&self, other: &Self) -> bool {
        self == other
    }
}

impl IDisplay for MapEntry {
    fn display(&self) -> String {
        format!("[{} {}]", self.key.display(), self.value.display())
    }
}

impl IHash for MapEntry {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        crate::lang::hash::compose_ordered(
            "SEQUENTIAL",
            self.iter().map(|value| value.java_hash(hash_type)),
        ) as u64
    }
}

impl IObjType for MapEntry {
    fn obj_type(&self) -> ObjType {
        ObjType::MapEntry
    }

    fn hash_seed(&self) -> String {
        "::SEQUENTIAL".into()
    }
}
