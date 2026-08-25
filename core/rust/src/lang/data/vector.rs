use std::cell::Cell;
use std::rc::Rc;

use crate::lang::hash::JavaHash;
use crate::lang::protocol::ihash::HashType;
use crate::lang::protocol::{
    IAssoc, IColl, IConj, ICount, IDisplay, IEmpty, IEquality, IHash, IMetadata, IMutable, INth,
    IObjType, IPeekFirst, IPeekLast, IPersistent, IPopLast, IPushLast, IToMutable, IToPersistent,
    ObjType,
};

const NODE_SHIFT: usize = 5;
const NODE_WIDTH: usize = 1 << NODE_SHIFT;
const NODE_MASK: usize = NODE_WIDTH - 1;

/// Token stamped on persistent (non-editable) nodes, the counterpart of
/// Java's `Node.NOEDIT`. Transient nodes carry the owning `Mutable`'s token
/// instead; see `next_token`.
const NOEDIT: u64 = 0;

thread_local! {
    static NEXT_TOKEN: Cell<u64> = Cell::new(1);
}

/// Java uses an `AtomicReference<Thread>` as the edit token; the Rust runtime
/// is single-threaded per vector (Rc-based), so a thread-local monotonic u64
/// plays the same role. Each `Mutable` session gets a fresh token.
fn next_token() -> u64 {
    NEXT_TOKEN.with(|next| {
        let token = next.get();
        next.set(token + 1);
        token
    })
}

fn tail_offset(size: usize) -> usize {
    if size < NODE_WIDTH {
        0
    } else {
        ((size - 1) >> NODE_SHIFT) << NODE_SHIFT
    }
}

#[derive(Debug, Clone)]
enum Node<E> {
    Branch(Branch<E>),
    Leaf(Leaf<E>),
}

#[derive(Debug, Clone)]
struct Branch<E> {
    token: u64,
    children: Vec<Option<Rc<Node<E>>>>,
}

#[derive(Debug, Clone)]
struct Leaf<E> {
    token: u64,
    values: Vec<E>,
}

impl<E: Clone> Node<E> {
    fn empty_branch() -> Rc<Self> {
        Rc::new(Self::Branch(Branch {
            token: NOEDIT,
            children: vec![None; NODE_WIDTH],
        }))
    }

    fn editable_empty_branch(token: u64) -> Rc<Self> {
        Rc::new(Self::Branch(Branch {
            token,
            children: vec![None; NODE_WIDTH],
        }))
    }

    fn token(&self) -> u64 {
        match self {
            Node::Branch(branch) => branch.token,
            Node::Leaf(leaf) => leaf.token,
        }
    }

    fn with_token(&self, token: u64) -> Self {
        let mut node = self.clone();
        match &mut node {
            Node::Branch(branch) => branch.token = token,
            Node::Leaf(leaf) => leaf.token = token,
        }
        node
    }
}

fn children_mut<'a, E>(node: &'a mut Node<E>) -> &'a mut Vec<Option<Rc<Node<E>>>> {
    let Node::Branch(branch) = node else {
        unreachable!("editable vector path must be a branch")
    };
    &mut branch.children
}

/// Java `S.editableNode`: hand back the same node when it already belongs to
/// this transient (token match) and is uniquely owned, otherwise path-copy it
/// and stamp the copy with the transient's token.
fn ensure_editable<E: Clone>(node: Rc<Node<E>>, token: u64) -> Rc<Node<E>> {
    if node.token() == token && Rc::strong_count(&node) == 1 {
        node
    } else {
        Rc::new(node.with_token(token))
    }
}

fn new_path<E: Clone>(token: u64, level: usize, node: Rc<Node<E>>) -> Rc<Node<E>> {
    if level == 0 {
        return node;
    }
    let mut children = vec![None; NODE_WIDTH];
    children[0] = Some(new_path(token, level - NODE_SHIFT, node));
    Rc::new(Node::Branch(Branch { token, children }))
}

/// Persistent `S.pushTail` (editable=false): unconditional path-copy, new
/// nodes carry NOEDIT like Java's persistent nodes.
fn push_tail<E: Clone>(
    parent: &Rc<Node<E>>,
    level: usize,
    size: usize,
    tail: Rc<Node<E>>,
) -> Rc<Node<E>> {
    let Node::Branch(branch) = parent.as_ref() else {
        unreachable!("vector tree parent must be a branch")
    };
    let mut children = branch.children.clone();
    let index = ((size - 1) >> level) & NODE_MASK;
    children[index] = Some(if level == NODE_SHIFT {
        tail
    } else if let Some(child) = &children[index] {
        push_tail(child, level - NODE_SHIFT, size, tail)
    } else {
        new_path(NOEDIT, level - NODE_SHIFT, tail)
    });
    Rc::new(Node::Branch(Branch {
        token: NOEDIT,
        children,
    }))
}

/// Transient `S.pushTail` (editable=true): mutate in place when the parent is
/// uniquely owned by this transient, else path-copy with the transient token.
fn push_tail_editable<E: Clone>(
    token: u64,
    parent: Rc<Node<E>>,
    level: usize,
    size: usize,
    tail: Rc<Node<E>>,
) -> Rc<Node<E>> {
    let mut parent = ensure_editable(parent, token);
    let index = ((size - 1) >> level) & NODE_MASK;
    let slot = &mut children_mut(Rc::get_mut(&mut parent).expect("editable vector node"))[index];
    let child = if level == NODE_SHIFT {
        tail
    } else {
        match slot.take() {
            Some(existing) => push_tail_editable(token, existing, level - NODE_SHIFT, size, tail),
            None => new_path(token, level - NODE_SHIFT, tail),
        }
    };
    *slot = Some(child);
    parent
}

/// Persistent `S.assoc`: copy-on-write down the path.
fn assoc_node<E: Clone>(node: &Rc<Node<E>>, level: usize, index: usize, value: E) -> Rc<Node<E>> {
    if level == 0 {
        let Node::Leaf(leaf) = node.as_ref() else {
            unreachable!("vector terminal node must be a leaf")
        };
        let mut values = leaf.values.clone();
        values[index & NODE_MASK] = value;
        return Rc::new(Node::Leaf(Leaf {
            token: NOEDIT,
            values,
        }));
    }
    let Node::Branch(branch) = node.as_ref() else {
        unreachable!("vector path must contain branches")
    };
    let mut children = branch.children.clone();
    let child_index = (index >> level) & NODE_MASK;
    children[child_index] = Some(assoc_node(
        children[child_index]
            .as_ref()
            .expect("existing vector path"),
        level - NODE_SHIFT,
        index,
        value,
    ));
    Rc::new(Node::Branch(Branch {
        token: NOEDIT,
        children,
    }))
}

/// Transient assoc: descend with write-back, mutating nodes that belong to
/// this transient in place and path-copying shared/persistent ones.
///
/// DEVIATION from Java: `S.getNodeArrayFor(editable=true)` (Vector.java:34-50)
/// wraps non-matching children with `editableNode` but never writes the copy
/// back into the parent, so a transient assoc through a not-yet-editable path
/// would silently lose the write. The port links copies back like Clojure's
/// `editableNodeFor`.
fn assoc_editable<E: Clone>(
    token: u64,
    node: Rc<Node<E>>,
    level: usize,
    index: usize,
    value: E,
) -> Rc<Node<E>> {
    let mut node = ensure_editable(node, token);
    let inner = Rc::get_mut(&mut node).expect("editable vector node");
    if level == 0 {
        let Node::Leaf(leaf) = inner else {
            unreachable!("vector terminal node must be a leaf")
        };
        leaf.values[index & NODE_MASK] = value;
        return node;
    }
    let slot = &mut children_mut(inner)[(index >> level) & NODE_MASK];
    let child = slot.take().expect("existing vector path");
    *slot = Some(assoc_editable(
        token,
        child,
        level - NODE_SHIFT,
        index,
        value,
    ));
    node
}

/// Persistent `S.popTail`.
///
/// DEVIATION from Java (known bug, deliberately not replicated): Java's
/// `S.popTail` (Vector.java:104-120) mutates `node.array[subidx]` in place
/// even when `editable == false`, corrupting every persistent vector that
/// shares those nodes. The port is unconditional copy-on-write.
fn pop_tail<E: Clone>(node: &Rc<Node<E>>, level: usize, size: usize) -> Option<Rc<Node<E>>> {
    let Node::Branch(branch) = node.as_ref() else {
        unreachable!("vector path must contain branches")
    };
    let index = ((size - 2) >> level) & NODE_MASK;
    if level > NODE_SHIFT {
        let child = pop_tail(
            branch.children[index]
                .as_ref()
                .expect("existing vector path"),
            level - NODE_SHIFT,
            size,
        );
        if child.is_none() && index == 0 {
            return None;
        }
        let mut children = branch.children.clone();
        children[index] = child;
        Some(Rc::new(Node::Branch(Branch {
            token: NOEDIT,
            children,
        })))
    } else if index == 0 {
        None
    } else {
        let mut children = branch.children.clone();
        children[index] = None;
        Some(Rc::new(Node::Branch(Branch {
            token: NOEDIT,
            children,
        })))
    }
}

/// Transient `S.popTail` (editable=true): mutate in place when the node
/// belongs to this transient, else copy with the transient token.
fn pop_tail_editable<E: Clone>(
    token: u64,
    node: Rc<Node<E>>,
    level: usize,
    size: usize,
) -> Option<Rc<Node<E>>> {
    let index = ((size - 2) >> level) & NODE_MASK;
    if level == NODE_SHIFT && index == 0 {
        return None;
    }
    let mut node = ensure_editable(node, token);
    let slot = &mut children_mut(Rc::get_mut(&mut node).expect("editable vector node"))[index];
    if level > NODE_SHIFT {
        let child = pop_tail_editable(
            token,
            slot.take().expect("existing vector path"),
            level - NODE_SHIFT,
            size,
        );
        if child.is_none() && index == 0 {
            return None;
        }
        *slot = child;
    } else {
        *slot = None;
    }
    Some(node)
}

#[derive(Debug, Clone)]
pub struct Standard<E> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    size: usize,
    shift: usize,
    root: Rc<Node<E>>,
    tail: Rc<Vec<E>>,
}

impl<E: Clone> Default for Standard<E> {
    fn default() -> Self {
        Self {
            metadata: None,
            size: 0,
            shift: NODE_SHIFT,
            root: Node::empty_branch(),
            tail: Rc::new(Vec::new()),
        }
    }
}

impl<E: Clone> Standard<E> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `Standard.into`: bulk-build through the transient and freeze.
    pub fn from_iter(values: impl IntoIterator<Item = E>) -> Self {
        let mut mutable = Mutable::from_iter(values);
        mutable.to_persistent()
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    fn tail_offset(&self) -> usize {
        tail_offset(self.size)
    }

    pub fn get(&self, index: usize) -> Option<&E> {
        self.array_for(index)?.get(index & NODE_MASK)
    }

    pub fn assoc_value(&self, index: usize, value: E) -> Option<Self> {
        if index == self.size {
            return Some(self.push_last(value));
        }
        if index >= self.size {
            return None;
        }
        if index >= self.tail_offset() {
            let mut tail = (*self.tail).clone();
            tail[index & NODE_MASK] = value;
            return Some(Self {
                tail: Rc::new(tail),
                ..self.clone()
            });
        }
        Some(Self {
            root: assoc_node(&self.root, self.shift, index, value),
            ..self.clone()
        })
    }

    pub fn assoc_value_owned(mut self, index: usize, value: E) -> Option<Self> {
        if index == self.size {
            return Some(self.push_last_owned(value));
        }
        if index >= self.size {
            return None;
        }
        if index >= self.tail_offset() {
            Rc::make_mut(&mut self.tail)[index & NODE_MASK] = value;
        } else {
            self.root = assoc_editable(NOEDIT, self.root, self.shift, index, value);
        }
        Some(self)
    }

    pub fn push_last(&self, value: E) -> Self {
        if self.size - self.tail_offset() < NODE_WIDTH {
            let mut tail = (*self.tail).clone();
            tail.push(value);
            return Self {
                size: self.size + 1,
                tail: Rc::new(tail),
                ..self.clone()
            };
        }

        let tail_node = Rc::new(Node::Leaf(Leaf {
            token: NOEDIT,
            values: (*self.tail).clone(),
        }));
        let overflow = (self.size >> NODE_SHIFT) > (1usize << self.shift);
        let (root, shift) = if overflow {
            let mut children = vec![None; NODE_WIDTH];
            children[0] = Some(self.root.clone());
            children[1] = Some(new_path(NOEDIT, self.shift, tail_node));
            (
                Rc::new(Node::Branch(Branch {
                    token: NOEDIT,
                    children,
                })),
                self.shift + NODE_SHIFT,
            )
        } else {
            (
                push_tail(&self.root, self.shift, self.size, tail_node),
                self.shift,
            )
        };

        Self {
            metadata: self.metadata.clone(),
            size: self.size + 1,
            shift,
            root,
            tail: Rc::new(vec![value]),
        }
    }

    pub fn push_last_owned(mut self, value: E) -> Self {
        if self.size - self.tail_offset() < NODE_WIDTH {
            Rc::make_mut(&mut self.tail).push(value);
            self.size += 1;
            return self;
        }

        let tail_node = Rc::new(Node::Leaf(Leaf {
            token: NOEDIT,
            values: std::mem::take(Rc::make_mut(&mut self.tail)),
        }));
        self.tail = Rc::new(vec![value]);
        if (self.size >> NODE_SHIFT) > (1usize << self.shift) {
            let mut children = vec![None; NODE_WIDTH];
            children[0] = Some(self.root);
            children[1] = Some(new_path(NOEDIT, self.shift, tail_node));
            self.root = Rc::new(Node::Branch(Branch {
                token: NOEDIT,
                children,
            }));
            self.shift += NODE_SHIFT;
        } else {
            self.root = push_tail_editable(NOEDIT, self.root, self.shift, self.size, tail_node);
        }
        self.size += 1;
        self
    }

    pub fn pop_last_value(&self) -> Option<Self> {
        if self.size == 0 {
            return None;
        }
        if self.size == 1 {
            return Some(Self {
                metadata: self.metadata.clone(),
                ..Self::new()
            });
        }
        if self.size - self.tail_offset() > 1 {
            let mut tail = (*self.tail).clone();
            tail.pop();
            return Some(Self {
                size: self.size - 1,
                tail: Rc::new(tail),
                ..self.clone()
            });
        }

        let new_tail = self
            .array_for(self.size - 2)
            .expect("previous vector leaf")
            .clone();
        let mut root =
            pop_tail(&self.root, self.shift, self.size).unwrap_or_else(Node::empty_branch);
        let mut shift = self.shift;
        if shift > NODE_SHIFT {
            let collapse =
                matches!(root.as_ref(), Node::Branch(branch) if branch.children[1].is_none());
            if collapse {
                let Node::Branch(branch) = root.as_ref() else {
                    unreachable!("collapsed vector root must be a branch")
                };
                root = branch.children[0].clone().expect("collapsed vector root");
                shift -= NODE_SHIFT;
            }
        }
        Some(Self {
            metadata: self.metadata.clone(),
            size: self.size - 1,
            shift,
            root,
            tail: Rc::new(new_tail),
        })
    }

    fn array_for(&self, index: usize) -> Option<&Vec<E>> {
        if index >= self.size {
            return None;
        }
        if index >= self.tail_offset() {
            return Some(&self.tail);
        }
        let mut node = self.root.as_ref();
        let mut level = self.shift;
        while level > 0 {
            let Node::Branch(branch) = node else {
                return None;
            };
            node = branch.children[(index >> level) & NODE_MASK].as_deref()?;
            level -= NODE_SHIFT;
        }
        match node {
            Node::Leaf(leaf) => Some(&leaf.values),
            Node::Branch(_) => None,
        }
    }

    pub fn iter(&self) -> Iter<'_, E> {
        Iter::new(self, 0, self.size)
    }

    /// Java `Base.rangedIterator`, exposed for `SubView`.
    fn ranged_iter(&self, start: usize, end: usize) -> Iter<'_, E> {
        Iter::new(self, start, end)
    }

    /// DEVIATION from Java bounds: Java rejects `end > size - 1`, which makes
    /// it impossible to view the last element (`subview(0, size)` throws).
    /// The port accepts the Rust-idiomatic `start <= end <= size`.
    pub fn subview(&self, start: usize, end: usize) -> Option<SubView<E>> {
        if start > end || end > self.size {
            return None;
        }
        Some(SubView {
            vector: self.clone(),
            start,
            end,
        })
    }

    #[cfg(test)]
    fn shares_root_with(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.root, &other.root)
    }
}

/// Java `Base.rangedIterator`: walks the tree one 32-element chunk at a time
/// instead of re-descending per element.
pub struct Iter<'a, E> {
    vector: &'a Standard<E>,
    index: usize,
    end: usize,
    base: usize,
    chunk: Option<&'a [E]>,
}

impl<'a, E: Clone> Iter<'a, E> {
    fn new(vector: &'a Standard<E>, start: usize, end: usize) -> Self {
        let chunk = if start < vector.size {
            vector.array_for(start).map(|values| values.as_slice())
        } else {
            None
        };
        Self {
            vector,
            index: start,
            end,
            base: start - (start % NODE_WIDTH),
            chunk,
        }
    }
}

impl<'a, E: Clone> Iterator for Iter<'a, E> {
    type Item = &'a E;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.end {
            return None;
        }
        if self.index - self.base == NODE_WIDTH {
            self.chunk = self
                .vector
                .array_for(self.index)
                .map(|values| values.as_slice());
            self.base += NODE_WIDTH;
        }
        let value = &self.chunk?[self.index & NODE_MASK];
        self.index += 1;
        Some(value)
    }
}

impl<E: Clone> FromIterator<E> for Standard<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        Self::from_iter(iter)
    }
}

impl<E: Clone> From<Vec<E>> for Standard<E> {
    fn from(values: Vec<E>) -> Self {
        Self::from_iter(values)
    }
}

impl<E: Clone> IntoIterator for Standard<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}

impl<E: Clone> std::ops::Index<usize> for Standard<E> {
    type Output = E;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("vector index out of bounds")
    }
}

impl<E: Clone + PartialEq> PartialEq for Standard<E> {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.iter().eq(other.iter())
    }
}

impl<E: Clone + PartialEq> IEquality for Standard<E> {
    fn equality(&self, other: &Self) -> bool {
        self == other
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

impl<E: Clone> IAssoc<usize, E> for Standard<E> {
    type Output = Self;
    fn assoc(&self, index: usize, value: E) -> Self {
        self.assoc_value(index, value)
            .expect("vector index out of bounds")
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
impl<E: Clone> IPushLast<E> for Standard<E> {
    type Output = Self;
    fn push_last(&self, value: E) -> Self {
        Standard::push_last(self, value)
    }
}

impl<E: Clone> IPopLast for Standard<E> {
    type Output = Self;
    fn pop_last(&self) -> Self {
        self.pop_last_value().expect("cannot pop empty vector")
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
        Self {
            metadata: self.metadata.clone(),
            ..Self::new()
        }
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

impl<E: Clone> IToMutable for Standard<E> {
    type Mutable = Mutable<E>;

    fn to_mutable(&self) -> Self::Mutable {
        Mutable::from_standard(self)
    }
}

impl<E: Clone + std::fmt::Debug> IDisplay for Standard<E> {
    fn display(&self) -> String {
        format!(
            "[{}]",
            self.iter()
                .map(|value| format!("{value:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

impl<E: Clone + std::hash::Hash + JavaHash> IHash for Standard<E> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java IVectorType extends ISequential: ordered composition,
        // "::SEQUENTIAL" seed (see lang::hash).
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

/// Transient vector. Unlike Java's 32-slot scratch tail with null padding,
/// the Rust tail is a `Vec<E>` whose length is always exactly
/// `size - tailoff(size)`; the "scratch" behaviour comes from in-place
/// `push`/`pop` on that Vec. All tree nodes touched by the transient are
/// stamped with its `token` and mutated in place only while uniquely owned
/// (`Rc::strong_count == 1`), matching Java's edit-token discipline.
#[derive(Debug, Clone)]
pub struct Mutable<E> {
    editable: Rc<Cell<bool>>,
    token: u64,
    size: usize,
    shift: usize,
    root: Rc<Node<E>>,
    tail: Vec<E>,
    metadata: Option<Rc<crate::lang::data::Metadata>>,
}

impl<E: Clone> Mutable<E> {
    pub fn new() -> Self {
        let token = next_token();
        Self {
            editable: Rc::new(Cell::new(true)),
            token,
            size: 0,
            shift: NODE_SHIFT,
            root: Node::editable_empty_branch(token),
            tail: Vec::new(),
            metadata: None,
        }
    }

    pub fn from_iter(values: impl IntoIterator<Item = E>) -> Self {
        let mut mutable = Self::new();
        for value in values {
            mutable.push_last(value);
        }
        mutable
    }

    /// Java `new Mutable(Base)`: editable copy of the root, copy of the tail.
    fn from_standard(vector: &Standard<E>) -> Self {
        let token = next_token();
        Self {
            editable: Rc::new(Cell::new(true)),
            token,
            size: vector.size,
            shift: vector.shift,
            root: Rc::new(vector.root.with_token(token)),
            tail: (*vector.tail).clone(),
            metadata: vector.metadata.clone(),
        }
    }

    fn check_editable(&self) {
        assert!(
            self.editable.get(),
            "mutable vector used after to_persistent"
        );
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
        self.nth_ref(index)
    }

    fn nth_ref(&self, index: usize) -> Option<&E> {
        if index >= self.size {
            return None;
        }
        if index >= tail_offset(self.size) {
            return self.tail.get(index & NODE_MASK);
        }
        self.leaf_values(index)?.get(index & NODE_MASK)
    }

    /// Read the tree leaf holding `index` (index must be below the tail).
    fn leaf_values(&self, index: usize) -> Option<&Vec<E>> {
        let mut node = self.root.as_ref();
        let mut level = self.shift;
        while level > 0 {
            let Node::Branch(branch) = node else {
                return None;
            };
            node = branch.children[(index >> level) & NODE_MASK].as_deref()?;
            level -= NODE_SHIFT;
        }
        match node {
            Node::Leaf(leaf) => Some(&leaf.values),
            Node::Branch(_) => None,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.check_editable();
        (0..self.size).map(move |index| self.nth_ref(index).expect("vector index in range"))
    }

    pub fn push_last(&mut self, value: E) -> &mut Self {
        self.check_editable();
        // room in the tail?
        if self.size - tail_offset(self.size) < NODE_WIDTH {
            self.tail.push(value);
            self.size += 1;
            return self;
        }
        // full tail, wrap it as a tree node
        let tail_node = Rc::new(Node::Leaf(Leaf {
            token: self.token,
            values: std::mem::take(&mut self.tail),
        }));
        self.tail = vec![value];
        let mut new_shift = self.shift;
        // overflow root?
        let new_root = if (self.size >> NODE_SHIFT) > (1usize << self.shift) {
            let mut children = vec![None; NODE_WIDTH];
            children[0] = Some(std::mem::replace(&mut self.root, Node::empty_branch()));
            children[1] = Some(new_path(self.token, self.shift, tail_node));
            new_shift += NODE_SHIFT;
            Rc::new(Node::Branch(Branch {
                token: self.token,
                children,
            }))
        } else {
            let old_root = std::mem::replace(&mut self.root, Node::empty_branch());
            push_tail_editable(self.token, old_root, self.shift, self.size, tail_node)
        };
        self.root = new_root;
        self.shift = new_shift;
        self.size += 1;
        self
    }

    pub fn assoc(&mut self, index: usize, value: E) -> &mut Self {
        self.check_editable();
        if index == self.size {
            return self.push_last(value);
        }
        assert!(index < self.size, "vector index out of bounds");
        if index >= tail_offset(self.size) {
            self.tail[index & NODE_MASK] = value;
            return self;
        }
        let old_root = std::mem::replace(&mut self.root, Node::empty_branch());
        self.root = assoc_editable(self.token, old_root, self.shift, index, value);
        self
    }

    pub fn pop_last(&mut self) -> Option<E> {
        self.check_editable();
        if self.size == 0 {
            return None;
        }
        // The last element always lives in the tail.
        let popped = self.tail.pop().expect("non-empty vector has a tail");
        if self.size == 1 {
            self.size = 0;
            return Some(popped);
        }
        if ((self.size - 1) & NODE_MASK) > 0 {
            self.size -= 1;
            return Some(popped);
        }
        // tail boundary: pull the last tree leaf down as the new tail
        let new_tail = self
            .leaf_values(self.size - 2)
            .expect("previous vector leaf")
            .clone();
        let old_root = std::mem::replace(&mut self.root, Node::empty_branch());
        let mut new_root = pop_tail_editable(self.token, old_root, self.shift, self.size)
            .unwrap_or_else(|| Node::editable_empty_branch(self.token));
        let mut new_shift = self.shift;
        if new_shift > NODE_SHIFT {
            let collapse =
                matches!(new_root.as_ref(), Node::Branch(branch) if branch.children[1].is_none());
            if collapse {
                let child = children_mut(Rc::get_mut(&mut new_root).expect("editable vector root"))
                    [0]
                .take()
                .expect("collapsed vector root");
                new_root = ensure_editable(child, self.token);
                new_shift -= NODE_SHIFT;
            }
        }
        self.root = new_root;
        self.shift = new_shift;
        self.size -= 1;
        self.tail = new_tail;
        Some(popped)
    }

    pub fn empty(&mut self) -> &mut Self {
        self.check_editable();
        self.size = 0;
        self.shift = NODE_SHIFT;
        self.root = Node::editable_empty_branch(self.token);
        self.tail = Vec::new();
        self
    }
}

impl<E: Clone> Default for Mutable<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Clone> FromIterator<E> for Mutable<E> {
    fn from_iter<T: IntoIterator<Item = E>>(iter: T) -> Self {
        Self::from_iter(iter)
    }
}

impl<E: Clone> IMutable for Mutable<E> {}

impl<E: Clone> IToPersistent for Mutable<E> {
    type Persistent = Standard<E>;

    fn to_persistent(&mut self) -> Self::Persistent {
        self.check_editable();
        // Java `_root.edit.set(null)`: invalidate the transient session.
        self.editable.set(false);
        // Java `S.trimTail` trims the 32-slot scratch tail to
        // `size - tailoff(size)`. The Rust tail Vec is already exact-length,
        // so only spare capacity is released.
        self.tail.shrink_to_fit();
        Standard {
            metadata: self.metadata.clone(),
            size: self.size,
            shift: self.shift,
            root: self.root.clone(),
            tail: Rc::new(std::mem::take(&mut self.tail)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubView<E> {
    vector: Standard<E>,
    start: usize,
    end: usize,
}

impl<E: Clone> SubView<E> {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<&E> {
        if index >= self.len() {
            None
        } else {
            self.vector.get(self.start + index)
        }
    }

    /// Java `SubView.iterator`: ranged iterator over the backing vector.
    pub fn iter(&self) -> Iter<'_, E> {
        self.vector.ranged_iter(self.start, self.end)
    }

    /// Java `SubView.pushLast`: assoc at `_end` on the backing vector
    /// (write-through when `_end < v.count`, append when equal) and extend.
    pub fn push_last(&self, value: E) -> Self {
        Self {
            vector: self
                .vector
                .assoc_value(self.end, value)
                .expect("subview push within bounds"),
            start: self.start,
            end: self.end + 1,
        }
    }

    /// Java `SubView.popLast` returns the view unchanged when empty; the port
    /// surfaces that as `None`.
    pub fn pop_last_value(&self) -> Option<Self> {
        if self.end == self.start {
            return None;
        }
        Some(Self {
            vector: self.vector.clone(),
            start: self.start,
            end: self.end - 1,
        })
    }

    /// Java `SubView.assoc`: write-through into the backing vector; assoc at
    /// `len()` extends the view like `push_last`.
    pub fn assoc_value(&self, index: usize, value: E) -> Option<Self> {
        if index > self.len() {
            return None;
        }
        if index == self.len() {
            return Some(self.push_last(value));
        }
        Some(Self {
            vector: self.vector.assoc_value(self.start + index, value)?,
            start: self.start,
            end: self.end,
        })
    }

    /// DEVIATION from Java bounds: same off-by-one as `Standard::subview`;
    /// the port accepts `start <= end <= len()`.
    pub fn subview(&self, start: usize, end: usize) -> Option<Self> {
        if start > end || end > self.len() {
            return None;
        }
        Some(Self {
            vector: self.vector.clone(),
            start: self.start + start,
            end: self.start + end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Mutable, Standard};
    use crate::lang::protocol::{IAssoc, IToMutable, IToPersistent};

    fn contents(vector: &Standard<i64>) -> Vec<i64> {
        vector.iter().copied().collect()
    }

    #[test]
    fn preserves_values_across_java_tree_boundaries() {
        let vector = (0..1057).collect::<Standard<_>>();
        let appended = vector.push_last(1057);
        let updated = appended.assoc(32, -1);

        assert_eq!(vector.len(), 1057);
        assert_eq!(vector[32], 32);
        assert_eq!(appended[1057], 1057);
        assert_eq!(updated[32], -1);
        assert_eq!(updated[1057], 1057);
    }

    #[test]
    fn tail_updates_share_the_tree() {
        let vector = (0..33).collect::<Standard<_>>();
        let appended = vector.push_last(33);

        assert!(vector.shares_root_with(&appended));
    }

    #[test]
    fn push_across_32_1024_32768_boundaries() {
        // Persistent pushes across the first root-growth boundaries.
        let mut vector = Standard::new();
        for value in 0..2100i64 {
            vector = vector.push_last(value);
            assert_eq!(vector.len() as i64, value + 1);
            assert_eq!(vector.get(value as usize), Some(&value));
        }
        for value in 0..2100i64 {
            assert_eq!(vector.get(value as usize), Some(&value));
        }

        // Bulk build across the 32768 (second root-level) boundary.
        let big = Standard::from_iter(0..33000i64);
        assert_eq!(big.len(), 33000);
        for value in 0..33000i64 {
            assert_eq!(big.get(value as usize), Some(&value));
        }
        assert_eq!(contents(&big), (0..33000).collect::<Vec<_>>());
        // Pushing past a full 32768-element tree grows a new root level.
        let grown = (0..32768i64).fold(Standard::new(), |v, i| v.push_last(i));
        assert_eq!(grown.len(), 32768);
        let grown = grown.push_last(32768);
        assert_eq!(grown.len(), 32769);
        assert_eq!(grown.get(32768), Some(&32768));
        assert_eq!(grown.get(0), Some(&0));
    }

    #[test]
    fn pop_to_empty() {
        let mut vector = Standard::from_iter(0..2000i64);
        for expected in (0..2000i64).rev() {
            assert_eq!(vector.len() as i64, expected + 1);
            assert_eq!(vector.get(expected as usize), Some(&expected));
            vector = vector.pop_last_value().expect("pop non-empty vector");
        }
        assert!(vector.is_empty());
        assert_eq!(vector.pop_last_value(), None);
        // Reuse after popping to empty.
        let refilled = (0..40).fold(vector, |v, i| v.push_last(i));
        assert_eq!(contents(&refilled), (0..40).collect::<Vec<_>>());
    }

    #[test]
    fn assoc_at_all_tree_levels() {
        let vector = Standard::from_iter(0..40000i64);
        for index in [0usize, 1, 31, 32, 33, 1000, 1024, 1056, 32767, 32768, 39999] {
            let updated = vector.assoc_value(index, -(index as i64)).unwrap();
            assert_eq!(updated.get(index), Some(&(-(index as i64))));
            assert_eq!(
                vector.get(index),
                Some(&(index as i64)),
                "persistent source mutated"
            );
            assert_eq!(updated.len(), vector.len());
        }
        // assoc at len appends; past len is out of bounds.
        let appended = vector.assoc_value(40000, -1).unwrap();
        assert_eq!(appended.len(), 40001);
        assert_eq!(appended.get(40000), Some(&-1));
        assert!(vector.assoc_value(40001, -1).is_none());
    }

    #[test]
    fn subview_semantics() {
        let vector = Standard::from_iter(0..100i64);
        let view = vector.subview(10, 50).unwrap();
        assert_eq!(view.len(), 40);
        assert_eq!(view.get(0), Some(&10));
        assert_eq!(view.get(39), Some(&49));
        assert_eq!(view.get(40), None);
        assert_eq!(
            view.iter().copied().collect::<Vec<_>>(),
            (10..50).collect::<Vec<_>>()
        );

        // push through the view writes through to index `end` of the backing
        // vector and extends the view; the original vector is untouched.
        let pushed = view.push_last(1000);
        assert_eq!(pushed.len(), 41);
        assert_eq!(pushed.get(40), Some(&1000));
        assert_eq!(vector.get(50), Some(&50));

        // pop shrinks the view only.
        let popped = view.pop_last_value().unwrap();
        assert_eq!(popped.len(), 39);
        assert_eq!(popped.get(38), Some(&48));
        assert_eq!(view.len(), 40);

        // assoc writes through; assoc at len extends like push_last.
        let updated = view.assoc_value(0, -1).unwrap();
        assert_eq!(updated.get(0), Some(&-1));
        assert_eq!(updated.len(), 40);
        assert_eq!(vector.get(10), Some(&10));
        let extended = view.assoc_value(40, 777).unwrap();
        assert_eq!(extended.len(), 41);
        assert_eq!(extended.get(40), Some(&777));
        assert!(view.assoc_value(41, 0).is_none());

        // nested subview composes offsets.
        let nested = view.subview(5, 10).unwrap();
        assert_eq!(nested.len(), 5);
        assert_eq!(nested.get(0), Some(&15));
        assert_eq!(
            nested.iter().copied().collect::<Vec<_>>(),
            (15..20).collect::<Vec<_>>()
        );

        // empty view: pop yields None.
        let empty = vector.subview(5, 5).unwrap();
        assert!(empty.is_empty());
        assert!(empty.pop_last_value().is_none());
        // out-of-range views rejected.
        assert!(vector.subview(0, 101).is_none());
        assert!(vector.subview(6, 5).is_none());
    }

    #[test]
    fn transient_round_trip_matches_persistent() {
        // Bulk build through the transient.
        let mut transient = Mutable::from_iter(0..5000i64);
        let frozen = transient.to_persistent();
        let persistent = Standard::from_iter(0..5000i64);
        assert_eq!(contents(&frozen), (0..5000).collect::<Vec<_>>());
        assert_eq!(frozen, persistent);

        // Thaw, mutate at every level, push and pop across boundaries, freeze.
        let mut model: Vec<i64> = (0..5000).collect();
        let mut mutable = frozen.to_mutable();
        for index in [0usize, 31, 32, 1024, 4095, 4999] {
            mutable.assoc(index, -(index as i64));
            model[index] = -(index as i64);
        }
        for value in 5000..6000i64 {
            mutable.push_last(value);
            model.push(value);
        }
        for _ in 0..1500 {
            assert_eq!(mutable.pop_last(), model.pop());
        }
        let refrozen = mutable.to_persistent();
        assert_eq!(contents(&refrozen), model);
    }

    #[test]
    fn transient_freeze_trims_tail() {
        // Pop into a partial tail, freeze, then keep using the persistent
        // vector: the tail must be right-sized (no stale scratch slots).
        let mut mutable = Mutable::from_iter(0..100i64);
        for _ in 0..40 {
            let _ = mutable.pop_last();
        }
        let frozen = mutable.to_persistent();
        assert_eq!(frozen.len(), 60);
        assert_eq!(contents(&frozen), (0..60).collect::<Vec<_>>());
        let appended = frozen.push_last(100);
        assert_eq!(
            contents(&appended),
            (0..60).chain(std::iter::once(100)).collect::<Vec<_>>()
        );
        // Freeze on exact tail boundaries too.
        let mut boundary = Mutable::from_iter(0..64i64);
        let frozen = boundary.to_persistent();
        assert_eq!(frozen.len(), 64);
        let appended = frozen.push_last(64);
        assert_eq!(appended.len(), 65);
        assert_eq!(appended.get(64), Some(&64));
    }

    #[test]
    fn mutable_vector_matches_java_update_surface() {
        let mut vector = Mutable::from_iter([1, 2, 3]);
        assert_eq!(vector.len(), 3);
        assert_eq!(vector.get(1), Some(&2));
        vector.assoc(1, 5).assoc(3, 4);
        assert_eq!(vector.iter().copied().collect::<Vec<_>>(), vec![1, 5, 3, 4]);
        assert_eq!(vector.pop_last(), Some(4));
        vector.empty();
        assert!(vector.is_empty());
        assert_eq!(vector.pop_last(), None);
        // empty() resets to a usable transient.
        vector.push_last(9);
        assert_eq!(vector.iter().copied().collect::<Vec<_>>(), vec![9]);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn mutable_vector_rejects_assoc_past_count() {
        let mut vector = Mutable::from_iter([1, 2, 3]);
        vector.assoc(4, 5);
    }

    #[test]
    #[should_panic(expected = "mutable vector used after to_persistent")]
    fn mutable_vector_is_invalid_after_persisting() {
        let vector = (0..4).collect::<Standard<_>>();
        let mut mutable = vector.to_mutable();
        let _persistent = mutable.to_persistent();
        mutable.push_last(5);
    }

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn fuzz_matches_vec_model() {
        let mut rng = Lcg(0x9E3779B97F4A7C15);
        let mut model: Vec<i64> = Vec::new();
        let mut persistent: Standard<i64> = Standard::new();
        let mut transient: Mutable<i64> = Mutable::new();
        for step in 0..6000 {
            match rng.below(10) {
                0..=4 => {
                    let value = rng.next() as i64;
                    model.push(value);
                    persistent = persistent.push_last(value);
                    transient.push_last(value);
                }
                5..=6 => {
                    if !model.is_empty() {
                        model.pop();
                        persistent = persistent.pop_last_value().unwrap();
                        let _ = transient.pop_last();
                    }
                }
                _ => {
                    if !model.is_empty() {
                        let index = rng.below(model.len());
                        let value = rng.next() as i64;
                        model[index] = value;
                        persistent = persistent.assoc_value(index, value).unwrap();
                        transient.assoc(index, value);
                    }
                }
            }
            if step % 97 == 0 {
                assert_eq!(
                    persistent.iter().copied().collect::<Vec<_>>(),
                    model,
                    "persistent diverged at step {step}"
                );
                assert_eq!(
                    transient.iter().copied().collect::<Vec<_>>(),
                    model,
                    "transient diverged at step {step}"
                );
            }
            if step % 501 == 0 {
                // Freeze the transient and thaw a fresh one from the result.
                let frozen = transient.to_persistent();
                assert_eq!(frozen.iter().copied().collect::<Vec<_>>(), model);
                assert_eq!(frozen, persistent);
                transient = frozen.to_mutable();
                persistent = frozen;
            }
        }
    }
}
