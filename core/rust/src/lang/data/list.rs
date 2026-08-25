//! Persistent chunked list, ported from `hara.lang.data.List` (Java).
//!
//! `Standard` is a singly-linked chain of fixed 32-slot chunks. Each chunk
//! holds a live window `offset .. offset + count` into its block.
//!
//! Chunk block representation: Java uses a shared `Object[32]` with an
//! offset/count window; `popFirst` creates a new chunk that SHARES the block
//! (window shifted by one), while `pushFirst` into an open window CLONES the
//! block before writing. The Rust port mirrors this exactly with
//! `Rc<[Option<E>; 32]>`: the fixed array is a faithful, safe analogue of the
//! Java block (same memory shape, `None` where Java has `null`), `Rc` gives
//! the structural sharing, and clone-on-write preserves persistence. No
//! unsafe, no new crates, wasm32-compatible.
//!
//! `Mutable` is a power-of-2 ring buffer (`elements`, `mask`, `size`,
//! `offset`) with doubling grow and linearize-on-resize, guarded by an
//! editable flag: any use after `to_persistent` panics.

use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IAssoc, IColl, IConj, ICons, ICount, IDisplay, IEmpty, IEquality, IHash, IMetadata,
    IMutable, INth, IObjType, IPeekFirst, IPeekLast, IPersistent, IPopFirst, IPopLast, IPushFirst,
    IPushLast, IToMutable, IToPersistent, ObjType,
};
use std::cell::Cell;
use std::rc::Rc;

const CHUNK_SIZE: usize = 32;
const DEFAULT_CAPACITY: usize = 4;

/// Fixed 32-slot chunk block; `None` slots are outside the live window.
type Block<E> = [Option<E>; CHUNK_SIZE];

fn empty_block<E>() -> Block<E> {
    std::array::from_fn(|_| None)
}

#[derive(Debug, Clone)]
struct Chunk<E> {
    array: Rc<Block<E>>,
    offset: usize,
    count: usize,
    next: Option<Rc<Chunk<E>>>,
}

#[derive(Debug, Clone)]
pub struct Standard<E> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    head: Option<Rc<Chunk<E>>>,
    size: usize,
}

impl<E> Default for Standard<E> {
    fn default() -> Self {
        Self {
            metadata: None,
            head: None,
            size: 0,
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

    pub fn push_first(&self, value: E) -> Self {
        let head = match &self.head {
            // Open window: clone the block, write at offset - 1.
            Some(head) if head.offset > 0 => {
                let mut array = (*head.array).clone();
                array[head.offset - 1] = Some(value);
                Rc::new(Chunk {
                    array: Rc::new(array),
                    offset: head.offset - 1,
                    count: head.count + 1,
                    next: head.next.clone(),
                })
            }
            // No open window: fresh block, value at the last slot.
            _ => {
                let mut array = empty_block();
                array[CHUNK_SIZE - 1] = Some(value);
                Rc::new(Chunk {
                    array: Rc::new(array),
                    offset: CHUNK_SIZE - 1,
                    count: 1,
                    next: self.head.clone(),
                })
            }
        };
        Self {
            metadata: self.metadata.clone(),
            head: Some(head),
            size: self.size + 1,
        }
    }

    pub fn push_last(&self, value: E) -> Self {
        let mut mutable = self.to_mutable();
        mutable.push_last(value);
        mutable.to_persistent()
    }

    pub fn pop_first_value(&self) -> Self {
        if self.size == 0 {
            return self.clone();
        }
        let head = self.head.as_ref().expect("non-empty list has a head chunk");
        let head = if head.count > 1 {
            // Window shift: the new head chunk SHARES the block (no copy).
            Rc::new(Chunk {
                array: head.array.clone(),
                offset: head.offset + 1,
                count: head.count - 1,
                next: head.next.clone(),
            })
        } else {
            return Self {
                metadata: self.metadata.clone(),
                head: head.next.clone(),
                size: self.size - 1,
            };
        };
        Self {
            metadata: self.metadata.clone(),
            head: Some(head),
            size: self.size - 1,
        }
    }

    pub fn pop_last_value(&self) -> Self {
        let mut mutable = self.to_mutable();
        mutable.pop_last();
        mutable.to_persistent()
    }

    pub fn get(&self, index: usize) -> Option<&E> {
        if index >= self.size {
            return None;
        }
        let mut idx = index;
        let mut chunk = self.head.as_deref();
        while let Some(current) = chunk {
            if idx < current.count {
                return current.array[current.offset + idx].as_ref();
            }
            idx -= current.count;
            chunk = current.next.as_deref();
        }
        None
    }

    pub fn iter(&self) -> Iter<'_, E> {
        Iter {
            chunk: self.head.as_deref(),
            index: 0,
        }
    }
}

pub struct Iter<'a, E> {
    chunk: Option<&'a Chunk<E>>,
    index: usize,
}
impl<'a, E> Iterator for Iter<'a, E> {
    type Item = &'a E;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let chunk = self.chunk?;
            if self.index < chunk.count {
                let value = chunk.array[chunk.offset + self.index].as_ref();
                self.index += 1;
                return value;
            }
            self.chunk = chunk.next.as_deref();
            self.index = 0;
        }
    }
}

impl<E: Clone> FromIterator<E> for Standard<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        // Java `Standard.into`: fill a Mutable with pushLast, then freeze.
        let mut mutable = iter.into_iter().collect::<Mutable<E>>();
        mutable.to_persistent()
    }
}
impl<E: Clone> From<Vec<E>> for Standard<E> {
    fn from(values: Vec<E>) -> Self {
        values.into_iter().collect()
    }
}
impl<E: Clone> IntoIterator for Standard<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<'a, E: Clone> IntoIterator for &'a Standard<E> {
    type Item = &'a E;
    type IntoIter = Iter<'a, E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<E: Clone> std::ops::Index<usize> for Standard<E> {
    type Output = E;
    fn index(&self, index: usize) -> &E {
        self.get(index).expect("list index out of bounds")
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
        self.get(0).cloned()
    }
}
impl<E: Clone> IPeekLast<E> for Standard<E> {
    fn peek_last(&self) -> Option<E> {
        self.size
            .checked_sub(1)
            .and_then(|index| self.get(index))
            .cloned()
    }
}
impl<E: Clone> IPushFirst<E> for Standard<E> {
    type Output = Self;
    fn push_first(&self, value: E) -> Self {
        Standard::push_first(self, value)
    }
}
impl<E: Clone> IPushLast<E> for Standard<E> {
    type Output = Self;
    fn push_last(&self, value: E) -> Self {
        Standard::push_last(self, value)
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
impl<E: Clone> ICons<E> for Standard<E> {
    type Output = Self;
    fn cons(&self, value: E) -> Self {
        self.push_first(value)
    }
}
impl<E: Clone> IConj<E> for Standard<E> {
    type Output = Self;
    fn conj(&self, value: E) -> Self {
        self.push_last(value)
    }
}
impl<E: Clone> IEmpty for Standard<E> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.metadata.clone())
    }
}
impl<E: Clone> IAssoc<usize, E> for Standard<E> {
    type Output = Self;
    fn assoc(&self, index: usize, value: E) -> Self {
        let mut mutable = self.to_mutable();
        mutable.assoc(index, value);
        mutable.to_persistent()
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

impl<E: Clone + PartialEq> IEquality for Standard<E> {
    fn equality(&self, other: &Self) -> bool {
        self == other
    }
}
impl<E: Clone + std::fmt::Debug> IDisplay for Standard<E> {
    fn display(&self) -> String {
        format!(
            "({})",
            self.iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<E: Clone + std::hash::Hash + JavaHash> IHash for Standard<E> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java List extends IVectorType → ISequential: ordered
        // composition, "::SEQUENTIAL" seed (see lang::hash).
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
        "("
    }
    fn end_string(&self) -> &'static str {
        ")"
    }
}

/// Next power of two, minimum 1 (Java `max(1, 1 << log2Ceil(capacity))`).
fn ring_capacity(capacity: usize) -> usize {
    capacity.max(1).next_power_of_two()
}

#[derive(Clone)]
pub struct Mutable<E> {
    editable: Rc<Cell<bool>>,
    elements: Vec<Option<E>>,
    mask: usize,
    size: usize,
    offset: usize,
    metadata: Option<Rc<crate::lang::data::Metadata>>,
}

impl<E> Mutable<E> {
    pub fn new() -> Self {
        let capacity = ring_capacity(DEFAULT_CAPACITY);
        Self {
            editable: Rc::new(Cell::new(true)),
            elements: std::iter::repeat_with(|| None).take(capacity).collect(),
            mask: capacity - 1,
            size: 0,
            offset: 0,
            metadata: None,
        }
    }

    fn from_standard(list: &Standard<E>) -> Self
    where
        E: Clone,
    {
        let capacity = ring_capacity(list.size + 1);
        let mut elements: Vec<Option<E>> = std::iter::repeat_with(|| None).take(capacity).collect();
        // Flatten chunks into the ring buffer, in order, at offset 0.
        let mut i = 0;
        let mut chunk = list.head.as_deref();
        while let Some(current) = chunk {
            for j in 0..current.count {
                elements[i] = current.array[current.offset + j].clone();
                i += 1;
            }
            chunk = current.next.as_deref();
        }
        Self {
            editable: Rc::new(Cell::new(true)),
            elements,
            mask: capacity - 1,
            size: list.size,
            offset: 0,
            metadata: list.metadata.clone(),
        }
    }

    fn check_editable(&self) {
        assert!(self.editable.get(), "mutable list used after to_persistent");
    }

    // Doubling grow with linearize-on-resize.
    fn resize(&mut self, new_capacity: usize) {
        let mut elements: Vec<Option<E>> =
            std::iter::repeat_with(|| None).take(new_capacity).collect();
        for i in 0..self.size {
            elements[i] = self.elements[(self.offset + i) & self.mask].take();
        }
        self.elements = elements;
        self.offset = 0;
        self.mask = new_capacity - 1;
    }

    pub fn len(&self) -> usize {
        self.check_editable();
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> Option<&E> {
        self.check_editable();
        if index >= self.size {
            return None;
        }
        self.elements[(self.offset + index) & self.mask].as_ref()
    }
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.check_editable();
        (0..self.size).map(move |i| {
            self.elements[(self.offset + i) & self.mask]
                .as_ref()
                .expect("live ring slot")
        })
    }
    pub fn push_first(&mut self, value: E) -> &mut Self {
        self.check_editable();
        if self.size == self.elements.len() {
            self.resize(self.size << 1);
        }
        self.offset = self.offset.wrapping_sub(1) & self.mask;
        self.elements[self.offset] = Some(value);
        self.size += 1;
        self
    }
    pub fn push_last(&mut self, value: E) -> &mut Self {
        self.check_editable();
        if self.size == self.elements.len() {
            self.resize(self.size << 1);
        }
        self.elements[(self.offset + self.size) & self.mask] = Some(value);
        self.size += 1;
        self
    }
    pub fn pop_first(&mut self) -> Option<E> {
        self.check_editable();
        if self.size == 0 {
            return None;
        }
        let value = self.elements[self.offset].take();
        self.offset = (self.offset + 1) & self.mask;
        self.size -= 1;
        value
    }
    pub fn pop_last(&mut self) -> Option<E> {
        self.check_editable();
        if self.size == 0 {
            return None;
        }
        let value = self.elements[(self.offset + self.size - 1) & self.mask].take();
        self.size -= 1;
        value
    }
    pub fn assoc(&mut self, index: usize, value: E) -> Option<E> {
        self.check_editable();
        if index == self.size {
            self.push_last(value);
            return None;
        }
        assert!(index < self.size, "list index out of bounds");
        self.elements[(self.offset + index) & self.mask].replace(value)
    }
    pub fn empty(&mut self) -> &mut Self {
        self.check_editable();
        for slot in &mut self.elements {
            *slot = None;
        }
        self.size = 0;
        self.offset = 0;
        self
    }
}

impl<E> Default for Mutable<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: PartialEq> PartialEq for Mutable<E> {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
            && self.metadata == other.metadata
            && (0..self.size).all(|i| {
                self.elements[(self.offset + i) & self.mask]
                    == other.elements[(other.offset + i) & other.mask]
            })
    }
}

impl<E: std::fmt::Debug> std::fmt::Debug for Mutable<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(
                (0..self.size)
                    .filter_map(|i| self.elements[(self.offset + i) & self.mask].as_ref()),
            )
            .finish()
    }
}

impl<E> FromIterator<E> for Mutable<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        let mut mutable = Self::new();
        for value in iter {
            mutable.push_last(value);
        }
        mutable
    }
}
impl<E> IMutable for Mutable<E> {}
impl<E: Clone> IToPersistent for Mutable<E> {
    type Persistent = Standard<E>;
    fn to_persistent(&mut self) -> Self::Persistent {
        self.check_editable();
        self.editable.set(false);
        // Convert the ring buffer to a chunked list, cutting chunks from the
        // END: the remainder (size % CHUNK_SIZE) is cut first, so every chunk
        // is full except possibly the last (tail) one.
        let mut next: Option<Rc<Chunk<E>>> = None;
        let mut remaining = self.size;
        while remaining > 0 {
            let mut count = remaining % CHUNK_SIZE;
            if count == 0 {
                count = CHUNK_SIZE;
            }
            let start = remaining - count;
            let array: Block<E> = std::array::from_fn(|i| {
                if i < count {
                    self.elements[(self.offset + start + i) & self.mask].clone()
                } else {
                    None
                }
            });
            next = Some(Rc::new(Chunk {
                array: Rc::new(array),
                offset: 0,
                count,
                next,
            }));
            remaining -= count;
        }
        Standard {
            metadata: self.metadata.clone(),
            head: next,
            size: self.size,
        }
    }
}
impl<E: Clone> IToMutable for Standard<E> {
    type Mutable = Mutable<E>;
    fn to_mutable(&self) -> Self::Mutable {
        Mutable::from_standard(self)
    }
}

#[cfg(test)]
mod tests {
    use super::{Mutable, Standard};
    use crate::lang::protocol::{IAssoc, IEmpty, IMetadata, INth, IToMutable, IToPersistent};
    use std::collections::VecDeque;
    use std::rc::Rc;

    #[test]
    fn chunk_boundaries_and_persistence() {
        let list = (0..65).collect::<Standard<_>>();
        let prefixed = list.push_first(-1);
        let appended = list.push_last(65);
        assert_eq!(list.len(), 65);
        assert_eq!(list[0], 0);
        assert_eq!(prefixed[0], -1);
        assert_eq!(appended[65], 65);
    }

    #[test]
    fn push_first_pop_first_across_chunk_boundaries() {
        for &n in &[31usize, 32, 33, 64, 65] {
            let mut list = Standard::new();
            for i in 0..n {
                list = list.push_first(i);
            }
            assert_eq!(list.len(), n);
            // push-first consing order: last pushed comes first
            let expected = (0..n).rev().collect::<Vec<_>>();
            assert_eq!(list.iter().copied().collect::<Vec<_>>(), expected);
            let mut popped = list.clone();
            let mut model: VecDeque<_> = expected.iter().copied().collect();
            while let Some(front) = model.pop_front() {
                assert_eq!(popped.get(0), Some(&front));
                popped = popped.pop_first_value();
            }
            assert!(popped.is_empty());
            assert_eq!(popped.pop_first_value().len(), 0);
        }
    }

    #[test]
    fn pop_first_shares_chunk_array_with_original() {
        let list = Standard::from((0..40).collect::<Vec<_>>());
        let popped = list.pop_first_value();
        // Aliasing: the popped list's head chunk shares the original's block.
        assert!(Rc::ptr_eq(
            &list.head.as_ref().unwrap().array,
            &popped.head.as_ref().unwrap().array
        ));
        // Both iterate correctly through the shared block.
        assert_eq!(
            list.iter().copied().collect::<Vec<_>>(),
            (0..40).collect::<Vec<_>>()
        );
        assert_eq!(
            popped.iter().copied().collect::<Vec<_>>(),
            (1..40).collect::<Vec<_>>()
        );
        // Popping through a whole chunk (count hits 1) drops to the next.
        let mut rest = list.clone();
        for _ in 0..32 {
            rest = rest.pop_first_value();
        }
        assert_eq!(
            rest.iter().copied().collect::<Vec<_>>(),
            (32..40).collect::<Vec<_>>()
        );
    }

    #[test]
    fn push_first_into_open_window_clones_chunk_array() {
        // push_first-only head has offset > 0, so the next push clones.
        let list = Standard::new().push_first(3).push_first(2).push_first(1);
        let pushed = list.push_first(0);
        assert!(!Rc::ptr_eq(
            &list.head.as_ref().unwrap().array,
            &pushed.head.as_ref().unwrap().array
        ));
        assert_eq!(pushed.iter().copied().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert_eq!(list.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn to_persistent_cuts_full_chunks_from_the_end() {
        let list = Standard::from((0..65).collect::<Vec<_>>());
        // Java Mutable.toPersistent: remaining % 32 is cut FIRST from the
        // logical end, so the TAIL chunk is partial; all chunks ahead of it
        // are full (offset 0). 65 = 32 + 32 + 1.
        let head = list.head.as_ref().unwrap();
        assert_eq!((head.offset, head.count), (0, 32));
        let second = head.next.as_ref().unwrap();
        assert_eq!((second.offset, second.count), (0, 32));
        let tail = second.next.as_ref().unwrap();
        assert_eq!((tail.offset, tail.count), (0, 1));
        // Exact multiple of the chunk size: tail is full too.
        let full = Standard::from((0..64).collect::<Vec<_>>());
        assert_eq!(full.head.as_ref().unwrap().count, 32);
    }

    #[test]
    fn iteration_order_is_front_to_back() {
        let list = Standard::new().push_first(3).push_first(2).push_first(1);
        assert_eq!(list.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(list.clone().into_iter().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(Standard::<i32>::new().iter().next().is_none());
    }

    #[test]
    fn nth_walks_chunks() {
        let list = Standard::from((0..100).collect::<Vec<_>>());
        for i in 0..100 {
            assert_eq!(list.get(i), Some(&i));
            assert_eq!(list.nth(i), Some(&i));
        }
        assert_eq!(list.get(100), None);
    }

    #[test]
    fn mutable_ring_buffer_wraparound_matches_vecdeque() {
        let mut mutable = Mutable::new();
        let mut model = VecDeque::new();
        // Interleave pushes and pops to force offset wraparound and resize.
        for i in 0..200i32 {
            mutable.push_last(i);
            model.push_back(i);
            if i % 3 == 0 {
                assert_eq!(mutable.pop_first(), model.pop_front());
            }
            if i % 5 == 0 {
                mutable.push_first(-i);
                model.push_front(-i);
            }
            if i % 7 == 0 {
                assert_eq!(mutable.pop_last(), model.pop_back());
            }
            assert_eq!(mutable.len(), model.len());
            assert_eq!(
                mutable.iter().copied().collect::<Vec<_>>(),
                model.iter().copied().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn fuzz_persistent_and_mutable_against_vecdeque() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let mut list = Standard::new();
        let mut model: VecDeque<usize> = VecDeque::new();
        for _ in 0..2000 {
            match next() % 4 {
                0 => {
                    let v = next();
                    list = list.push_first(v);
                    model.push_front(v);
                }
                1 => {
                    let v = next();
                    list = list.push_last(v);
                    model.push_back(v);
                }
                2 => {
                    list = list.pop_first_value();
                    model.pop_front();
                }
                _ => {
                    list = list.pop_last_value();
                    model.pop_back();
                }
            }
            assert_eq!(list.len(), model.len());
        }
        assert_eq!(
            list.iter().copied().collect::<Vec<_>>(),
            model.iter().copied().collect::<Vec<_>>()
        );
        // Mutable phase: continue the same op stream through the transient.
        let mut mutable = list.to_mutable();
        for _ in 0..2000 {
            match next() % 4 {
                0 => {
                    let v = next();
                    mutable.push_first(v);
                    model.push_front(v);
                }
                1 => {
                    let v = next();
                    mutable.push_last(v);
                    model.push_back(v);
                }
                2 => assert_eq!(mutable.pop_first(), model.pop_front()),
                _ => assert_eq!(mutable.pop_last(), model.pop_back()),
            }
            assert_eq!(mutable.len(), model.len());
        }
        let frozen = mutable.to_persistent();
        assert_eq!(
            frozen.iter().copied().collect::<Vec<_>>(),
            model.iter().copied().collect::<Vec<_>>()
        );
    }

    #[test]
    #[should_panic(expected = "mutable list used after to_persistent")]
    fn mutable_use_after_to_persistent_panics() {
        let mut mutable = Standard::from(vec![1, 2, 3]).to_mutable();
        let _ = mutable.to_persistent();
        mutable.push_last(4);
    }

    #[test]
    fn persistent_operations_preserve_metadata() {
        let list = Standard::from(vec![1, 2, 3])
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        let values = [
            list.push_first(0),
            list.push_last(4),
            list.pop_first_value(),
            list.pop_last_value(),
            list.assoc(1, 20),
            list.empty(),
        ];
        assert!(values
            .iter()
            .all(|value| value.meta().map(|m| m.doc().unwrap()) == Some("doc")));
    }

    #[test]
    fn mutable_round_trip_preserves_original_and_updates_edges() {
        let original = (0..65).collect::<Standard<_>>();
        let mut mutable = original.to_mutable();
        assert_eq!(mutable.assoc(32, 320), Some(32));
        mutable.push_first(-1).push_last(65);
        assert_eq!(mutable.pop_first(), Some(-1));
        assert_eq!(mutable.pop_last(), Some(65));
        let persistent = mutable.to_persistent();
        assert_eq!(persistent.get(32), Some(&320));
        assert_eq!(original.get(32), Some(&32));
    }

    #[test]
    fn assoc_at_count_appends_and_round_trip_keeps_metadata() {
        let original = Standard::from(vec![1, 2])
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        let appended = original.assoc(2, 3);
        assert_eq!(appended.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(appended.meta().and_then(|value| value.doc()), Some("doc"));

        let mut mutable = original.to_mutable();
        assert_eq!(mutable.assoc(2, 3), None);
        let persistent = mutable.to_persistent();
        assert_eq!(
            persistent.iter().copied().collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(persistent.meta().and_then(|value| value.doc()), Some("doc"));
    }

    #[test]
    #[should_panic(expected = "list index out of bounds")]
    fn persistent_assoc_rejects_index_past_count() {
        let _ = Standard::from(vec![1, 2]).assoc(3, 4);
    }
}
