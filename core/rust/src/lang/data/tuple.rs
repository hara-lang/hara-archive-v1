use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IColl, IConj, ICons, ICount, IDisplay, IEmpty, IEquality, IHash, IMetadata, INth,
    IObjType, IPeekFirst, IPeekLast, IPersistent, IPopFirst, IPopLast, IPushFirst, IPushLast,
    ObjType,
};
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum Tuple<E> {
    Tup0,
    Tup1([E; 1]),
    Tup2([E; 2]),
    Tup3([E; 3]),
    Tup4([E; 4]),
    Tup5([E; 5]),
    Tup6([E; 6]),
    Tup7([E; 7]),
    Tup8([E; 8]),
    WithMeta(Rc<crate::lang::data::Metadata>, Box<Tuple<E>>),
}

impl<E: Clone + PartialEq> PartialEq for Tuple<E> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<E: Clone + Eq> Eq for Tuple<E> {}
impl<E: Clone + std::hash::Hash> std::hash::Hash for Tuple<E> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.iter()
            .for_each(|value| std::hash::Hash::hash(value, state));
    }
}

impl<E: Clone> Tuple<E> {
    pub fn len(&self) -> usize {
        match self {
            Self::Tup0 => 0,
            Self::Tup1(_) => 1,
            Self::Tup2(_) => 2,
            Self::Tup3(_) => 3,
            Self::Tup4(_) => 4,
            Self::Tup5(_) => 5,
            Self::Tup6(_) => 6,
            Self::Tup7(_) => 7,
            Self::Tup8(_) => 8,
            Self::WithMeta(_, tuple) => tuple.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> Option<&E> {
        match self {
            Self::Tup0 => None,
            Self::Tup1(v) => v.get(index),
            Self::Tup2(v) => v.get(index),
            Self::Tup3(v) => v.get(index),
            Self::Tup4(v) => v.get(index),
            Self::Tup5(v) => v.get(index),
            Self::Tup6(v) => v.get(index),
            Self::Tup7(v) => v.get(index),
            Self::Tup8(v) => v.get(index),
            Self::WithMeta(_, tuple) => tuple.get(index),
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        (0..self.len()).map(|i| self.get(i).expect("valid tuple index"))
    }
    pub fn from_values(values: Vec<E>) -> Result<Self, String> {
        Ok(match values.as_slice() {
            [] => Self::Tup0,
            [a] => Self::Tup1([a.clone()]),
            [a, b] => Self::Tup2([a.clone(), b.clone()]),
            [a, b, c] => Self::Tup3([a.clone(), b.clone(), c.clone()]),
            [a, b, c, d] => Self::Tup4([a.clone(), b.clone(), c.clone(), d.clone()]),
            [a, b, c, d, e] => Self::Tup5([a.clone(), b.clone(), c.clone(), d.clone(), e.clone()]),
            [a, b, c, d, e, f] => Self::Tup6([
                a.clone(),
                b.clone(),
                c.clone(),
                d.clone(),
                e.clone(),
                f.clone(),
            ]),
            [a, b, c, d, e, f, g] => Self::Tup7([
                a.clone(),
                b.clone(),
                c.clone(),
                d.clone(),
                e.clone(),
                f.clone(),
                g.clone(),
            ]),
            [a, b, c, d, e, f, g, h] => Self::Tup8([
                a.clone(),
                b.clone(),
                c.clone(),
                d.clone(),
                e.clone(),
                f.clone(),
                g.clone(),
                h.clone(),
            ]),
            _ => return Err("tuple arity cannot exceed 8".into()),
        })
    }
    pub fn push_first(&self, value: E) -> Result<Self, String> {
        Self::from_values(std::iter::once(value).chain(self.iter().cloned()).collect())
            .map(|tuple| tuple.with_meta(self.meta().cloned()))
    }
    pub fn push_last(&self, value: E) -> Result<Self, String> {
        Self::from_values(self.iter().cloned().chain(std::iter::once(value)).collect())
            .map(|tuple| tuple.with_meta(self.meta().cloned()))
    }
    pub fn pop_first(&self) -> Self {
        Self::from_values(self.iter().skip(1).cloned().collect())
            .expect("smaller tuple")
            .with_meta(self.meta().cloned())
    }
    pub fn pop_last(&self) -> Self {
        Self::from_values(
            self.iter()
                .take(self.len().saturating_sub(1))
                .cloned()
                .collect(),
        )
        .expect("smaller tuple")
        .with_meta(self.meta().cloned())
    }
    pub fn peek_first(&self) -> Option<&E> {
        self.get(0)
    }
    pub fn peek_last(&self) -> Option<&E> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }
}
impl<E: Clone> ICount for Tuple<E> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<E: Clone> INth<E> for Tuple<E> {
    fn nth(&self, index: usize) -> Option<&E> {
        self.get(index)
    }
}
impl<E: Clone> IPeekFirst<E> for Tuple<E> {
    fn peek_first(&self) -> Option<E> {
        Tuple::peek_first(self).cloned()
    }
}
impl<E: Clone> IPeekLast<E> for Tuple<E> {
    fn peek_last(&self) -> Option<E> {
        Tuple::peek_last(self).cloned()
    }
}
impl<E: Clone> IPushFirst<E> for Tuple<E> {
    type Output = Self;
    fn push_first(&self, value: E) -> Self {
        Tuple::push_first(self, value).expect("tuple arity cannot exceed 8")
    }
}
impl<E: Clone> IPushLast<E> for Tuple<E> {
    type Output = Self;
    fn push_last(&self, value: E) -> Self {
        Tuple::push_last(self, value).expect("tuple arity cannot exceed 8")
    }
}
impl<E: Clone> IPopFirst for Tuple<E> {
    type Output = Self;
    fn pop_first(&self) -> Self {
        Tuple::pop_first(self)
    }
}
impl<E: Clone> IPopLast for Tuple<E> {
    type Output = Self;
    fn pop_last(&self) -> Self {
        Tuple::pop_last(self)
    }
}
impl<E: Clone> ICons<E> for Tuple<E> {
    type Output = Self;
    fn cons(&self, value: E) -> Self {
        IPushFirst::push_first(self, value)
    }
}
impl<E: Clone> IConj<E> for Tuple<E> {
    type Output = Self;
    fn conj(&self, value: E) -> Self {
        IPushLast::push_last(self, value)
    }
}
impl<E: Clone> IPersistent for Tuple<E> {}
impl<E: Clone> IMetadata for Tuple<E> {
    type Metadata = Rc<crate::lang::data::Metadata>;
    fn meta(&self) -> Option<&Self::Metadata> {
        match self {
            Self::WithMeta(metadata, _) => Some(metadata),
            _ => None,
        }
    }
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        let base = match self {
            Self::WithMeta(_, tuple) => tuple.as_ref(),
            _ => self,
        };
        metadata.map_or_else(
            || base.clone(),
            |metadata| Self::WithMeta(metadata, Box::new(base.clone())),
        )
    }
}
impl<E: Clone> IEmpty for Tuple<E> {
    type Output = Self;
    fn empty(&self) -> Self::Output {
        Self::Tup0.with_meta(self.meta().cloned())
    }
}
impl<E: Clone> Default for Tuple<E> {
    fn default() -> Self {
        Self::Tup0
    }
}

impl<E: Clone> IntoIterator for Tuple<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<E: Clone + PartialEq> IEquality for Tuple<E> {
    fn equality(&self, other: &Self) -> bool {
        self == other
    }
}
impl<E: Clone + std::fmt::Debug> IDisplay for Tuple<E> {
    fn display(&self) -> String {
        format!(
            "[{}]",
            self.iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<E: Clone + std::hash::Hash + JavaHash> IHash for Tuple<E> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java Tuple.TupN implements ISequential: ordered composition,
        // "::SEQUENTIAL" seed — NOT "::TUPLE" (see lang::hash). MapEntry
        // values use this same ordered two-value composition.
        crate::lang::hash::compose_ordered(
            "SEQUENTIAL",
            self.iter().map(|value| value.java_hash(hash_type)),
        ) as u64
    }
}
impl<E: Clone + std::fmt::Debug> IObjType for Tuple<E> {
    fn obj_type(&self) -> ObjType {
        ObjType::Sequential
    }
}
impl<E> IColl<E> for Tuple<E>
where
    E: Clone + PartialEq + std::fmt::Debug + std::hash::Hash + JavaHash,
{
    fn start_string(&self) -> &'static str {
        "["
    }
    fn end_string(&self) -> &'static str {
        "]"
    }
}

#[cfg(test)]
mod tests {
    use super::Tuple;
    use crate::lang::protocol::{IEmpty, IMetadata};

    #[test]
    fn covers_all_java_arities_and_linear_operations() {
        let mut t = Tuple::Tup0;
        for i in 0..8 {
            t = t.push_last(i).unwrap();
            assert_eq!(t.len(), i + 1)
        }
        assert!(t.push_last(8).is_err());
        assert_eq!(t.peek_first(), Some(&0));
        assert_eq!(t.peek_last(), Some(&7));
        assert_eq!(t.pop_first().peek_first(), Some(&1));
        assert_eq!(t.pop_last().peek_last(), Some(&6));
    }

    #[test]
    fn preserves_metadata_across_persistent_operations() {
        let tuple =
            Tuple::Tup2([1, 2]).with_meta(Some(crate::lang::data::Metadata::document("doc")));

        for result in [
            tuple.push_first(0).unwrap(),
            tuple.push_last(3).unwrap(),
            tuple.pop_first(),
            tuple.pop_last(),
            tuple.empty(),
        ] {
            assert_eq!(result.meta().map(|m| m.doc().unwrap()), Some("doc"));
        }
        assert_eq!(tuple.push_first(0).unwrap().len(), 3);
        assert_eq!(tuple.empty().len(), 0);
    }
}
