use std::cell::RefCell;
use std::rc::Rc;

use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, ICons, ICount, IDisplay, IEmpty, IEquality, IHash, IMetadata, IObjType, IPeekFirst,
    IPersistent, IPopFirst, IPushFirst, ObjType,
};

#[derive(Clone)]
pub struct Seq<E> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    state: Rc<RefCell<State<E>>>,
    offset: usize,
}
struct State<E> {
    source: Option<Box<dyn Iterator<Item = E>>>,
    realized: Vec<E>,
}

impl<E: Clone + 'static> Seq<E> {
    pub fn new(source: impl Iterator<Item = E> + 'static) -> Self {
        Self {
            metadata: None,
            state: Rc::new(RefCell::new(State {
                source: Some(Box::new(source)),
                realized: Vec::new(),
            })),
            offset: 0,
        }
    }
    fn realize(&self, index: usize) -> bool {
        let mut state = self.state.borrow_mut();
        while state.realized.len() <= index {
            let next = state.source.as_mut().and_then(Iterator::next);
            match next {
                Some(v) => state.realized.push(v),
                None => {
                    state.source = None;
                    return false;
                }
            }
        }
        true
    }
    pub fn peek_first(&self) -> Option<E> {
        self.realize(self.offset)
            .then(|| self.state.borrow().realized[self.offset].clone())
    }
    pub fn pop_first(&self) -> Self {
        Self {
            metadata: self.metadata.clone(),
            state: self.state.clone(),
            offset: self.offset + usize::from(self.peek_first().is_some()),
        }
    }
    pub fn count(&self) -> usize {
        let mut state = self.state.borrow_mut();
        while let Some(v) = state.source.as_mut().and_then(Iterator::next) {
            state.realized.push(v)
        }
        state.source = None;
        state.realized.len().saturating_sub(self.offset)
    }
    pub fn iter(&self) -> SeqIter<E> {
        SeqIter {
            seq: self.clone(),
            index: 0,
        }
    }
}
impl<E: Clone + 'static> ICount for Seq<E> {
    fn count(&self) -> usize {
        Seq::count(self)
    }
}
impl<E: Clone + 'static> IPeekFirst<E> for Seq<E> {
    fn peek_first(&self) -> Option<E> {
        Seq::peek_first(self)
    }
}
impl<E: Clone + 'static> IPushFirst<E> for Seq<E> {
    type Output = crate::lang::data::Cons<E, Self>;
    fn push_first(&self, value: E) -> Self::Output {
        crate::lang::data::Cons::new(value, self.clone()).with_meta(self.metadata.clone())
    }
}
impl<E: Clone + 'static> ICons<E> for Seq<E> {
    type Output = crate::lang::data::Cons<E, Self>;
    fn cons(&self, value: E) -> Self::Output {
        crate::lang::data::Cons::new(value, self.clone())
    }
}
impl<E: Clone + 'static> IPopFirst for Seq<E> {
    type Output = Self;
    fn pop_first(&self) -> Self::Output {
        Seq::pop_first(self)
    }
}
impl<E: Clone + 'static> IEmpty for Seq<E> {
    type Output = crate::lang::data::Tuple<E>;
    fn empty(&self) -> Self::Output {
        crate::lang::data::Tuple::Tup0.with_meta(self.metadata.clone())
    }
}
impl<E: Clone + 'static> IMetadata for Seq<E> {
    type Metadata = Rc<crate::lang::data::Metadata>;
    fn meta(&self) -> Option<&Self::Metadata> {
        self.metadata.as_ref()
    }
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            metadata,
            state: self.state.clone(),
            offset: self.offset,
        }
    }
}
impl<E: Clone + 'static> IPersistent for Seq<E> {}
impl<E: Clone + PartialEq + 'static> IEquality for Seq<E> {
    fn equality(&self, other: &Self) -> bool {
        self.clone().into_iter().eq(other.clone())
    }
}
impl<E: Clone + std::fmt::Debug + 'static> IDisplay for Seq<E> {
    fn display(&self) -> String {
        let mut values = self.iter();
        let mut displayed = values
            .by_ref()
            .take(10)
            .map(|value| format!("{value:?}"))
            .collect::<Vec<_>>();
        if values.next().is_some() {
            displayed.push("...".into());
        }
        format!("({})", displayed.join(" "))
    }
}
impl<E: Clone + std::hash::Hash + JavaHash + 'static> IHash for Seq<E> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java Seq is sequential: ordered composition, "::SEQUENTIAL" seed.
        crate::lang::hash::compose_ordered(
            "SEQUENTIAL",
            self.clone().into_iter().map(|v| v.java_hash(hash_type)),
        ) as u64
    }
}
impl<E: Clone + std::fmt::Debug + 'static> IObjType for Seq<E> {
    fn obj_type(&self) -> ObjType {
        ObjType::Sequential
    }
}
impl<E: Clone + 'static> IntoIterator for Seq<E> {
    type Item = E;
    type IntoIter = SeqIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        SeqIter {
            seq: self,
            index: 0,
        }
    }
}

pub struct SeqIter<E> {
    seq: Seq<E>,
    index: usize,
}
impl<E: Clone + 'static> Iterator for SeqIter<E> {
    type Item = E;
    fn next(&mut self) -> Option<E> {
        let index = self.seq.offset + self.index;
        if !self.seq.realize(index) {
            return None;
        }
        self.index += 1;
        Some(self.seq.state.borrow().realized[index].clone())
    }
}
impl<E> std::fmt::Debug for Seq<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seq")
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::Seq;
    use crate::lang::protocol::{ICons, IEmpty, IMetadata, IPopFirst, IPushFirst};
    use std::cell::Cell;
    use std::rc::Rc;
    #[test]
    fn realizes_once_and_shares_state() {
        let calls = Rc::new(Cell::new(0));
        let c = calls.clone();
        let seq = Seq::new((0..3).map(move |v| {
            c.set(c.get() + 1);
            v
        }));
        assert_eq!(seq.peek_first(), Some(0));
        assert_eq!(seq.peek_first(), Some(0));
        assert_eq!(calls.get(), 1);
        assert_eq!(seq.pop_first().iter().collect::<Vec<_>>(), vec![1, 2]);

        let documented = seq.with_meta(Some(crate::lang::data::Metadata::document("doc")));
        assert_eq!(
            IPopFirst::pop_first(&documented)
                .meta()
                .map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert!(documented.empty().is_empty());
        assert_eq!(
            documented.empty().meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );

        let pushed = documented.push_first(-1);
        assert_eq!(pushed.peek_first(), &-1);
        let tail = pushed.pop_first();
        assert_eq!(tail.peek_first(), Some(0));
        assert_eq!(tail.meta().map(|m| m.doc().unwrap()), Some("doc"));
        let consed = documented.cons(-1);
        assert_eq!(consed.meta(), None);
        assert_eq!(consed.pop_first().peek_first(), Some(0));
    }
}
