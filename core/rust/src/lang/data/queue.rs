use crate::lang::data::{List, Vector};
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IColl, IConj, ICount, IDisplay, IEmpty, IEquality, IHash, IMetadata, IMutable, INth,
    IObjType, IPeekFirst, IPeekLast, IPersistent, IPopFirst, IPopLast, IPushFirst, IPushLast,
    IToMutable, IToPersistent, ObjType,
};
use std::rc::Rc;

pub const MAX_LENGTH: usize = 1024;

#[derive(Debug, Clone)]
pub struct Standard<E> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    size: usize,
    offset: usize,
    head: Vector<E>,
    tail: Vector<E>,
    buffer: List<Vector<E>>,
}

impl<E: Clone> Default for Standard<E> {
    fn default() -> Self {
        Self {
            metadata: None,
            size: 0,
            offset: 0,
            head: Vector::new(),
            tail: Vector::new(),
            buffer: List::new(),
        }
    }
}
impl<E: Clone> Standard<E> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    pub fn peek_first(&self) -> Option<&E> {
        if self.size == 0 {
            None
        } else {
            self.head.get(self.offset)
        }
    }
    pub fn peek_last(&self) -> Option<&E> {
        if self.size == 0 {
            None
        } else if !self.tail.is_empty() {
            self.tail.get(self.tail.len() - 1)
        } else if !self.buffer.is_empty() {
            self.buffer
                .get(self.buffer.len() - 1)
                .and_then(|v| v.get(v.len() - 1))
        } else {
            self.head.get(self.head.len() - 1)
        }
    }
    pub fn get(&self, index: usize) -> Option<&E> {
        if index >= self.size {
            return None;
        }
        let head_remaining = self.head.len().saturating_sub(self.offset);
        if index < head_remaining {
            return self.head.get(self.offset + index);
        }
        let after = index - head_remaining;
        let chunk = after / MAX_LENGTH;
        let column = after % MAX_LENGTH;
        if chunk < self.buffer.len() {
            self.buffer.get(chunk).and_then(|v| v.get(column))
        } else {
            self.tail.get(column)
        }
    }
    pub fn push_last(&self, value: E) -> Self {
        let space = MAX_LENGTH - self.offset;
        if self.size < space {
            return Self {
                size: self.size + 1,
                head: self.head.push_last(value),
                ..self.clone()
            };
        }
        let tail = self.tail.push_last(value);
        if tail.len() < MAX_LENGTH {
            Self {
                size: self.size + 1,
                tail,
                ..self.clone()
            }
        } else {
            Self {
                size: self.size + 1,
                tail: Vector::new(),
                buffer: self.buffer.push_last(tail),
                ..self.clone()
            }
        }
    }
    pub fn push_first(&self, value: E) -> Self {
        std::iter::once(value)
            .chain(self.iter().cloned())
            .collect::<Self>()
            .with_meta(self.metadata.clone())
    }
    pub fn pop_first_value(&self) -> Self {
        if self.size == 0 {
            return self.clone();
        }
        let offset = self.offset + 1;
        if offset < MAX_LENGTH {
            return Self {
                size: self.size - 1,
                offset,
                ..self.clone()
            };
        }
        if self.buffer.is_empty() {
            Self {
                metadata: self.metadata.clone(),
                size: self.size - 1,
                offset: 0,
                head: self.tail.clone(),
                tail: Vector::new(),
                buffer: self.buffer.clone(),
            }
        } else {
            Self {
                size: self.size - 1,
                offset: 0,
                head: self.buffer[0].clone(),
                buffer: self.buffer.pop_first_value(),
                ..self.clone()
            }
        }
    }
    pub fn pop_last_value(&self) -> Self {
        if self.size == 0 {
            return self.clone();
        }
        if !self.tail.is_empty() {
            Self {
                size: self.size - 1,
                tail: self.tail.pop_last_value().expect("nonempty tail"),
                ..self.clone()
            }
        } else if self.buffer.is_empty() {
            Self {
                size: self.size - 1,
                head: self.head.pop_last_value().unwrap_or_else(Vector::new),
                ..self.clone()
            }
        } else {
            Self {
                size: self.size - 1,
                tail: self.buffer[self.buffer.len() - 1].clone(),
                buffer: self.buffer.pop_last_value(),
                ..self.clone()
            }
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        // Java Queue.Base.iterator (fixed): drop the first _offset head
        // elements, then every buffer segment in order, then the tail.
        self.head
            .iter()
            .skip(self.offset)
            .chain(self.buffer.iter().flat_map(|segment| segment.iter()))
            .chain(self.tail.iter())
    }
}
impl<E: Clone> FromIterator<E> for Standard<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        iter.into_iter().fold(Self::new(), |q, v| q.push_last(v))
    }
}
impl<E: Clone + PartialEq> PartialEq for Standard<E> {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.iter().eq(other.iter())
    }
}
impl<E: Clone> ICount for Standard<E> {
    fn count(&self) -> usize {
        self.size
    }
}
impl<E: Clone> INth<E> for Standard<E> {
    fn nth(&self, index: usize) -> Option<&E> {
        self.get(index)
    }
}
impl<E: Clone> IPeekFirst<E> for Standard<E> {
    fn peek_first(&self) -> Option<E> {
        Standard::peek_first(self).cloned()
    }
}
impl<E: Clone> IPeekLast<E> for Standard<E> {
    fn peek_last(&self) -> Option<E> {
        Standard::peek_last(self).cloned()
    }
}
impl<E: Clone> IPushLast<E> for Standard<E> {
    type Output = Self;
    fn push_last(&self, value: E) -> Self {
        Standard::push_last(self, value)
    }
}
impl<E: Clone> IPushFirst<E> for Standard<E> {
    type Output = Self;
    fn push_first(&self, value: E) -> Self {
        Standard::push_first(self, value)
    }
}
impl<E: Clone> IConj<E> for Standard<E> {
    type Output = Self;
    fn conj(&self, value: E) -> Self {
        self.push_last(value)
    }
}
impl<E: Clone> IPopFirst for Standard<E> {
    type Output = Self;
    fn pop_first(&self) -> Self {
        self.pop_first_value()
    }
}
impl<E: Clone> IPopLast for Standard<E> {
    type Output = Self;
    fn pop_last(&self) -> Self {
        self.pop_last_value()
    }
}
impl<E: Clone> IEmpty for Standard<E> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.metadata.clone())
    }
}
impl<E: Clone> IMetadata for Standard<E> {
    type Metadata = Rc<crate::lang::data::Metadata>;
    fn meta(&self) -> Option<&Self::Metadata> {
        self.metadata.as_ref()
    }
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            metadata,
            ..self.clone()
        }
    }
}
impl<E: Clone> IPersistent for Standard<E> {}

impl<E: Clone> IntoIterator for Standard<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<E: Clone + PartialEq> IEquality for Standard<E> {
    fn equality(&self, other: &Self) -> bool {
        self == other
    }
}
impl<E: Clone + std::fmt::Debug> IDisplay for Standard<E> {
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
impl<E: Clone + std::hash::Hash + JavaHash> IHash for Standard<E> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java Queue extends ISequentialLookupType → ISequential:
        // ordered composition, "::SEQUENTIAL" seed (see lang::hash).
        crate::lang::hash::compose_ordered(
            "SEQUENTIAL",
            self.iter().map(|value| value.java_hash(hash_type)),
        ) as u64
    }
}
impl<E: Clone + std::fmt::Debug> IObjType for Standard<E> {
    fn obj_type(&self) -> ObjType {
        ObjType::Sequential
    }
}
impl<E> IColl<E> for Standard<E>
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mutable<E: Clone> {
    value: Standard<E>,
}
impl<E: Clone> Mutable<E> {
    pub fn new() -> Self {
        Self {
            value: Standard::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.value.len()
    }
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }
    pub fn get(&self, index: usize) -> Option<&E> {
        self.value.get(index)
    }
    pub fn peek_first(&self) -> Option<&E> {
        self.value.peek_first()
    }
    pub fn peek_last(&self) -> Option<&E> {
        self.value.peek_last()
    }
    pub fn push_last(&mut self, value: E) -> &mut Self {
        self.value = self.value.push_last(value);
        self
    }
    pub fn push_first(&mut self, value: E) -> &mut Self {
        self.value = self.value.push_first(value);
        self
    }
    pub fn pop_first(&mut self) -> &mut Self {
        self.value = self.value.pop_first_value();
        self
    }
    pub fn pop_last(&mut self) -> &mut Self {
        self.value = self.value.pop_last_value();
        self
    }
    pub fn empty(&mut self) -> &mut Self {
        self.value = Standard::new();
        self
    }
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.value.iter()
    }
}
impl<E: Clone> FromIterator<E> for Mutable<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        Self {
            value: iter.into_iter().collect(),
        }
    }
}
impl<E: Clone> IMutable for Mutable<E> {}
impl<E: Clone> IToPersistent for Mutable<E> {
    type Persistent = Standard<E>;
    fn to_persistent(&mut self) -> Self::Persistent {
        self.value.clone()
    }
}
impl<E: Clone> IToMutable for Standard<E> {
    type Mutable = Mutable<E>;
    fn to_mutable(&self) -> Self::Mutable {
        Mutable {
            value: self.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Standard, MAX_LENGTH};
    #[test]
    fn crosses_head_buffer_tail_boundaries() {
        let q = (0..(MAX_LENGTH * 2 + 9)).collect::<Standard<_>>();
        assert_eq!(q.get(0), Some(&0));
        assert_eq!(q.get(MAX_LENGTH), Some(&MAX_LENGTH));
        assert_eq!(q.peek_last(), Some(&(MAX_LENGTH * 2 + 8)));
        let p = q.pop_first_value();
        assert_eq!(p.peek_first(), Some(&1));
        assert_eq!(q.peek_first(), Some(&0));
    }
    #[test]
    fn pop_last_preserves_previous() {
        let q = (0..1030).collect::<Standard<_>>();
        let p = q.pop_last_value();
        assert_eq!(q.peek_last(), Some(&1029));
        assert_eq!(p.peek_last(), Some(&1028));
    }
    #[test]
    fn push_first_preserves_order_and_previous_value() {
        let queue = (1..=3).collect::<Standard<_>>();
        let pushed = queue.push_first(0);
        assert_eq!(queue.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(pushed.iter().copied().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    }
    #[test]
    fn persistent_operations_preserve_metadata() {
        use crate::lang::protocol::{IEmpty, IMetadata};
        let queue = Standard::from_iter(0..(MAX_LENGTH + 2))
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        let values = [
            queue.push_last(9999),
            queue.pop_first_value(),
            queue.pop_last_value(),
            queue.empty(),
        ];
        assert!(values
            .iter()
            .all(|value| value.meta().map(|m| m.doc().unwrap()) == Some("doc")));
    }

    #[test]
    fn mutable_round_trip_crosses_chunk_boundaries() {
        use crate::lang::protocol::{IToMutable, IToPersistent};
        let original = (0..(MAX_LENGTH + 3)).collect::<Standard<_>>();
        let mut mutable = original.to_mutable();
        mutable.pop_first().pop_last().push_last(9999);
        assert_eq!(mutable.peek_first(), Some(&1));
        assert_eq!(mutable.peek_last(), Some(&9999));
        let persistent = mutable.to_persistent();
        assert_eq!(persistent.peek_first(), Some(&1));
        assert_eq!(persistent.peek_last(), Some(&9999));
        assert_eq!(original.peek_first(), Some(&0));
    }

    #[test]
    fn pop_first_promotes_tail_to_head_across_segment() {
        // 1030 pushes: head fills to 1024, the remaining 6 go to the tail.
        let queue = (0..1030).collect::<Standard<_>>();
        assert_eq!(queue.head.len(), MAX_LENGTH);
        assert_eq!(queue.tail.len(), 6);
        // 1024 pops push offset to 1024: head is replaced by the former
        // (short) tail and offset resets (Java popFirst, buffer empty).
        let queue = (0..MAX_LENGTH).fold(queue, |q, _| q.pop_first_value());
        assert_eq!(queue.offset, 0);
        assert_eq!(queue.head.len(), 6);
        assert!(queue.tail.is_empty());
        assert_eq!(
            queue.iter().copied().collect::<Vec<_>>(),
            (1024..1030).collect::<Vec<_>>()
        );
        // iteration and nth respect the offset into the promoted short head
        let queue = queue.pop_first_value();
        assert_eq!(queue.offset, 1);
        assert_eq!(
            queue.iter().copied().collect::<Vec<_>>(),
            vec![1025, 1026, 1027, 1028, 1029]
        );
        assert_eq!(queue.get(0), Some(&1025));
        assert_eq!(queue.get(4), Some(&1029));
        assert_eq!(queue.get(5), None);
    }

    #[test]
    fn nth_probes_across_head_buffer_and_tail() {
        // 2500 pushes, 100 pops: offset 100 into head, one buffer segment,
        // 452-element tail (Java nth segment computation).
        let queue = (0..2500).collect::<Standard<_>>();
        let queue = (0..100).fold(queue, |q, _| q.pop_first_value());
        assert_eq!(queue.offset, 100);
        assert_eq!(queue.buffer.len(), 1);
        assert_eq!(queue.len(), 2400);
        for (index, value) in [
            (0, 100),
            (923, 1023),
            (924, 1024),
            (1947, 2047),
            (1948, 2048),
            (2399, 2499),
        ] {
            assert_eq!(queue.get(index), Some(&value));
        }
        assert_eq!(queue.iter().next(), Some(&100));
        assert_eq!(queue.iter().last(), Some(&2499));
        assert_eq!(queue.iter().count(), 2400);
    }

    #[test]
    fn pop_last_promotes_last_buffer_segment_to_tail() {
        // Java popLast quirk, pinned for parity: when the tail is empty and
        // the buffer is not, the last buffer segment becomes the tail
        // WITHOUT dropping an element — only _size decrements, so the
        // segment iterator still yields the retained last element.
        // Build the quirk state directly: head + 1 buffer segment + empty tail.
        let queue = (0..(2 * MAX_LENGTH + 1)).collect::<Standard<_>>();
        let queue = queue.pop_last_value(); // tail 1 -> empty
        assert!(queue.tail.is_empty());
        assert_eq!(queue.buffer.len(), 1);
        let queue = queue.pop_last_value(); // promotes buffer segment, no drop
        assert!(queue.buffer.is_empty());
        assert_eq!(queue.tail.len(), MAX_LENGTH);
        assert_eq!(queue.len(), 2 * MAX_LENGTH - 1);
        assert_eq!(queue.peek_last(), Some(&(2 * MAX_LENGTH - 1)));
        // segment iterator (Java parity) still sees the retained element
        assert_eq!(queue.iter().count(), 2 * MAX_LENGTH);
        assert_eq!(queue.iter().last(), Some(&(2 * MAX_LENGTH - 1)));
    }
}
