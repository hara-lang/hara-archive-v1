use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

thread_local! {
    static CHAMP_PLACEMENT_HASHING: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn with_champ_placement_hash<T>(f: impl FnOnce() -> T) -> T {
    CHAMP_PLACEMENT_HASHING.with(|active| {
        let previous = active.replace(true);
        let result = f();
        active.set(previous);
        result
    })
}

pub(crate) fn champ_placement_hashing() -> bool {
    CHAMP_PLACEMENT_HASHING.with(Cell::get)
}

use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IAssoc, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash,
    ILookup, IMetadata, IMutable, IObjType, IPersistent, IToMutable, IToPersistent, MetaType,
    ObjType,
};

const SHIFT: usize = 5;
const MASK: u64 = 0x1f;

// CHAMP port of `java/src/main/java/hara/lang/data/Map.java`.
//
// Layout parity with the Java `DataNode`: a node's `slots` vec is split into
// a data region `[0, data_arity)` holding one `Slot::Entry` per key/value pair
// (Java flattens pairs into two array slots; one slot per pair here keeps the
// same ordering) and a node region `[data_arity, len)` holding sub-nodes.
// Entries ascend by bitpos from the front; sub-nodes ascend by bitpos from
// the END, so `slots[data_arity]` is the highest-bitpos sub-node and
// `slots[len - 1]` the lowest. Iteration emits the data region first, then
// descends sub-nodes from `slots[data_arity]` forward — Java `NodeIter`
// visits its node slots counting down from nodeArity, which is the same
// descending-bitpos order.
#[derive(Debug, Clone)]
enum Slot<K, V> {
    Entry { hash: u64, key: K, value: V },
    Node(Rc<Node<K, V>>),
}

#[derive(Debug, Clone)]
struct DataNode<K, V> {
    edit: Cell<u64>,
    datamap: u32,
    nodemap: u32,
    slots: Vec<Slot<K, V>>,
}

#[derive(Debug, Clone)]
struct CollisionNode<K, V> {
    edit: Cell<u64>,
    hash: u64,
    entries: Vec<(K, V)>,
}

#[derive(Debug, Clone)]
enum Node<K, V> {
    Data(DataNode<K, V>),
    Collision(CollisionNode<K, V>),
}

impl<K, V> Node<K, V> {
    fn empty() -> Rc<Self> {
        Rc::new(Self::Data(DataNode {
            edit: Cell::new(0),
            datamap: 0,
            nodemap: 0,
            slots: Vec::new(),
        }))
    }
    fn set_edit(&self, token: u64) {
        match self {
            Node::Data(d) => d.edit.set(token),
            Node::Collision(c) => c.edit.set(token),
        }
    }
    fn is_single(&self) -> bool {
        match self {
            Node::Data(d) => d.nodemap == 0 && d.datamap.count_ones() == 1,
            Node::Collision(c) => c.entries.len() == 1,
        }
    }
}

/// Hash used for CHAMP placement, matching Java's `(int) G.hashRapid(key)` —
/// the low 32 bits of the Java-parity value hash.
///
/// `core::Value`'s `std::hash::Hash` impl writes exactly its `stable_hash()`
/// (the Java-parity RAPID hash) as a single `write_u64`, so we capture that
/// write with a probe hasher and take its low 32 bits. Keys of any other
/// type fall back to the previous `DefaultHasher`-over-`Hash` behaviour
/// (identical writes, identical result — no Java parity exists for them).
fn key_hash<K: Hash>(key: &K) -> u64 {
    #[derive(Default)]
    struct Probe {
        captured: Option<u64>,
        fallback: std::collections::hash_map::DefaultHasher,
    }
    impl Hasher for Probe {
        fn finish(&self) -> u64 {
            self.captured.unwrap_or_else(|| self.fallback.finish())
        }
        fn write(&mut self, bytes: &[u8]) {
            self.fallback.write(bytes);
        }
        fn write_u64(&mut self, value: u64) {
            if self.captured.is_none() {
                self.captured = Some(value);
            }
        }
    }
    let mut probe = Probe::default();
    with_champ_placement_hash(|| key.hash(&mut probe));
    let hash = probe.finish();
    if probe.captured.is_some() {
        (hash as u32) as u64
    } else {
        hash
    }
}
fn mask(hash: u64, shift: usize) -> usize {
    ((hash >> shift) & MASK) as usize
}
fn bit(hash: u64, shift: usize) -> u32 {
    1u32 << mask(hash, shift)
}
fn index(bitmap: u32, bit: u32) -> usize {
    (bitmap & (bit - 1)).count_ones() as usize
}
/// Node-region slot index for `bit`: sub-nodes ascend by bitpos from the end,
/// so the slot for `bit` sits at `data_arity + (node_arity - 1 - p)` where
/// `p` counts node bits below `bit`.
fn node_slot<K, V>(d: &DataNode<K, V>, bit: u32) -> usize {
    d.datamap.count_ones() as usize + (d.nodemap.count_ones() as usize - 1 - index(d.nodemap, bit))
}

/// Clone-on-write unifier. Every node-level operation prepares its level
/// through `ensure` before doing surgery. With `Some(token)` (transient) the
/// node is edited in place when uniquely owned and cloned when shared; with
/// `None` (persistent) the node is always cloned, leaving the source tree
/// untouched.
fn ensure<'a, K: Clone, V: Clone>(
    node: &'a mut Rc<Node<K, V>>,
    edit: Option<u64>,
) -> &'a mut Node<K, V> {
    match edit {
        Some(token) => {
            let n = Rc::make_mut(node);
            n.set_edit(token);
            n
        }
        None => {
            let fresh = (**node).clone();
            fresh.set_edit(0);
            *node = Rc::new(fresh);
            Rc::get_mut(node).expect("freshly cloned node is uniquely owned")
        }
    }
}

/// Java `mergeTwoKeyValuePairs`: build the sub-tree holding two entries that
/// currently collide at `shift`. Past shift 32 equal hashes become a
/// collision node (Java `BranchNode`); equal masks recurse wrapped in a
/// single-node DataNode; differing masks produce a two-entry DataNode ordered
/// by mask.
fn merge_two<K: Clone, V: Clone>(
    edit: Option<u64>,
    shift: usize,
    a_hash: u64,
    a_key: K,
    a_val: V,
    b_hash: u64,
    b_key: K,
    b_val: V,
) -> Rc<Node<K, V>> {
    let e = edit.unwrap_or(0);
    if shift > 32 && a_hash == b_hash {
        return Rc::new(Node::Collision(CollisionNode {
            edit: Cell::new(e),
            hash: a_hash,
            entries: vec![(a_key, a_val), (b_key, b_val)],
        }));
    }
    let abit = bit(a_hash, shift);
    let bbit = bit(b_hash, shift);
    if abit == bbit {
        return Rc::new(Node::Data(DataNode {
            edit: Cell::new(e),
            datamap: 0,
            nodemap: abit,
            slots: vec![Slot::Node(merge_two(
                edit,
                shift + SHIFT,
                a_hash,
                a_key,
                a_val,
                b_hash,
                b_key,
                b_val,
            ))],
        }));
    }
    let (first, second) = if mask(a_hash, shift) < mask(b_hash, shift) {
        (
            Slot::Entry {
                hash: a_hash,
                key: a_key,
                value: a_val,
            },
            Slot::Entry {
                hash: b_hash,
                key: b_key,
                value: b_val,
            },
        )
    } else {
        (
            Slot::Entry {
                hash: b_hash,
                key: b_key,
                value: b_val,
            },
            Slot::Entry {
                hash: a_hash,
                key: a_key,
                value: a_val,
            },
        )
    };
    Rc::new(Node::Data(DataNode {
        edit: Cell::new(e),
        datamap: abit | bbit,
        nodemap: 0,
        slots: vec![first, second],
    }))
}

/// Defensive merge of an existing (collision) node with a new entry whose
/// hash differs — unreachable for 32-bit placement past shift 32, kept for
/// fallback-hash keys. Data region first, node region last.
fn merge_node<K: Clone, V: Clone>(
    edit: Option<u64>,
    shift: usize,
    node_hash: u64,
    node: Rc<Node<K, V>>,
    hash: u64,
    key: K,
    value: V,
) -> Rc<Node<K, V>> {
    let e = edit.unwrap_or(0);
    let abit = bit(node_hash, shift);
    let bbit = bit(hash, shift);
    if abit == bbit {
        return Rc::new(Node::Data(DataNode {
            edit: Cell::new(e),
            datamap: 0,
            nodemap: abit,
            slots: vec![Slot::Node(merge_node(
                edit,
                shift + SHIFT,
                node_hash,
                node,
                hash,
                key,
                value,
            ))],
        }));
    }
    Rc::new(Node::Data(DataNode {
        edit: Cell::new(e),
        datamap: bbit,
        nodemap: abit,
        slots: vec![Slot::Entry { hash, key, value }, Slot::Node(node)],
    }))
}

/// Java `DataNode.assoc` / `BranchNode.assoc`. Returns whether a pair was
/// added (false on value replacement). `node` is updated in place through
/// the `Rc`; `ensure` at each level provides persistent vs transient
/// behaviour.
fn assoc_node<K: Clone + Eq, V: Clone>(
    node: &mut Rc<Node<K, V>>,
    edit: Option<u64>,
    shift: usize,
    hash: u64,
    key: K,
    value: V,
) -> bool {
    enum Act<K, V> {
        CollisionReplace(usize),
        CollisionPush,
        CollisionMerge,
        DataReplace(usize),
        DataMerge { i: usize, b: u32, old: (u64, K, V) },
        DataRecurse(usize),
        DataInsert { i: usize, b: u32 },
    }
    let act = match node.as_ref() {
        Node::Collision(c) => {
            if c.hash == hash {
                match c.entries.iter().position(|(k, _)| k == &key) {
                    Some(i) => Act::CollisionReplace(i),
                    None => Act::CollisionPush,
                }
            } else {
                Act::CollisionMerge
            }
        }
        Node::Data(d) => {
            let b = bit(hash, shift);
            if d.datamap & b != 0 {
                let i = index(d.datamap, b);
                match &d.slots[i] {
                    Slot::Entry {
                        hash: old_hash,
                        key: old_key,
                        value: old_value,
                    } => {
                        if old_key == &key {
                            Act::DataReplace(i)
                        } else {
                            Act::DataMerge {
                                i,
                                b,
                                old: (*old_hash, old_key.clone(), old_value.clone()),
                            }
                        }
                    }
                    Slot::Node(_) => unreachable!("data region holds entries only"),
                }
            } else if d.nodemap & b != 0 {
                Act::DataRecurse(node_slot(d, b))
            } else {
                Act::DataInsert {
                    i: index(d.datamap, b),
                    b,
                }
            }
        }
    };
    match act {
        Act::CollisionReplace(i) => {
            if let Node::Collision(c) = ensure(node, edit) {
                c.entries[i].1 = value;
            }
            false
        }
        Act::CollisionPush => {
            if let Node::Collision(c) = ensure(node, edit) {
                c.entries.push((key, value));
            }
            true
        }
        Act::CollisionMerge => {
            let node_hash = match node.as_ref() {
                Node::Collision(c) => c.hash,
                _ => unreachable!(),
            };
            let old = std::mem::replace(node, Node::empty());
            *node = merge_node(edit, shift, node_hash, old, hash, key, value);
            true
        }
        Act::DataReplace(i) => {
            if let Node::Data(d) = ensure(node, edit) {
                d.slots[i] = Slot::Entry { hash, key, value };
            }
            false
        }
        Act::DataMerge { i, b, old } => {
            let (old_hash, old_key, old_value) = old;
            let merged = merge_two(
                edit,
                shift + SHIFT,
                old_hash,
                old_key,
                old_value,
                hash,
                key,
                value,
            );
            if let Node::Data(d) = ensure(node, edit) {
                // copyAndMigrateToNode: drop the data slot, move the bit to
                // nodemap, insert the sub-node into the node region.
                d.slots.remove(i);
                d.datamap ^= b;
                d.nodemap |= b;
                let p = index(d.nodemap, b);
                let pos =
                    d.datamap.count_ones() as usize + (d.nodemap.count_ones() as usize - 1 - p);
                d.slots.insert(pos, Slot::Node(merged));
            }
            true
        }
        Act::DataRecurse(i) => {
            let n = ensure(node, edit);
            match n {
                Node::Data(d) => match &mut d.slots[i] {
                    Slot::Node(child) => assoc_node(child, edit, shift + SHIFT, hash, key, value),
                    Slot::Entry { .. } => unreachable!("node region holds nodes only"),
                },
                _ => unreachable!(),
            }
        }
        Act::DataInsert { i, b } => {
            if let Node::Data(d) = ensure(node, edit) {
                d.slots.insert(i, Slot::Entry { hash, key, value });
                d.datamap |= b;
            }
            true
        }
    }
}

fn find_node<'a, K: Eq, V>(
    node: &'a Node<K, V>,
    shift: usize,
    hash: u64,
    key: &K,
) -> Option<(&'a K, &'a V)> {
    match node {
        Node::Collision(c) if c.hash == hash => c
            .entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(k, v)| (k, v)),
        Node::Collision(_) => None,
        Node::Data(d) => {
            let b = bit(hash, shift);
            if d.datamap & b != 0 {
                match &d.slots[index(d.datamap, b)] {
                    Slot::Entry {
                        key: k, value: v, ..
                    } if k == key => Some((k, v)),
                    _ => None,
                }
            } else if d.nodemap & b != 0 {
                match &d.slots[node_slot(d, b)] {
                    Slot::Node(child) => find_node(child, shift + SHIFT, hash, key),
                    Slot::Entry { .. } => None,
                }
            } else {
                None
            }
        }
    }
}

/// Java `DataNode.without` / `BranchNode.without` for a key known to be
/// present — callers pre-check with `find_node`, so every level along the
/// path changes and `ensure` is always safe. Exact-size semantics differ
/// deliberately from two Java bugs this does not replicate: Java's
/// `BranchNode.persistentAssoc` inflates the node pair count on value
/// replacement, and its `BranchNode.without` two-element case threads the
/// `removed_leaf` counter through `EMPTY.assoc`, double-counting the removal.
fn without_present<K: Clone + Eq, V: Clone>(
    node: &mut Rc<Node<K, V>>,
    edit: Option<u64>,
    shift: usize,
    hash: u64,
    key: &K,
) {
    enum Act {
        DataRemove { i: usize, b: u32 },
        DataCollapse { keep: usize, new_datamap: u32 },
        Recurse { i: usize, b: u32 },
        CollisionRemove(usize),
        CollisionToData,
        CollisionToEmpty,
    }
    let act = match node.as_ref() {
        Node::Collision(c) => {
            debug_assert_eq!(c.hash, hash);
            match c.entries.len() {
                1 => Act::CollisionToEmpty,
                2 => Act::CollisionToData,
                _ => Act::CollisionRemove(
                    c.entries
                        .iter()
                        .position(|(k, _)| k == key)
                        .expect("key is present"),
                ),
            }
        }
        Node::Data(d) => {
            let b = bit(hash, shift);
            if d.datamap & b != 0 {
                let i = index(d.datamap, b);
                if d.datamap.count_ones() == 2 && d.nodemap == 0 {
                    let keep = if i == 0 { 1 } else { 0 };
                    // Java re-bases the removed key's hash at level 0. That is
                    // correct, not a bug: every key routed into a sub-node at
                    // shift s shares the removed key's masks at shifts
                    // 0..s-5, so both keys have the same root bitpos.
                    let new_datamap = if shift == 0 {
                        d.datamap ^ b
                    } else {
                        bit(hash, 0)
                    };
                    Act::DataCollapse { keep, new_datamap }
                } else {
                    Act::DataRemove { i, b }
                }
            } else {
                debug_assert!(d.nodemap & b != 0, "key is present below");
                Act::Recurse {
                    i: node_slot(d, b),
                    b,
                }
            }
        }
    };
    match act {
        Act::DataRemove { i, b } => {
            if let Node::Data(d) = ensure(node, edit) {
                d.slots.remove(i);
                d.datamap ^= b;
            }
        }
        Act::DataCollapse { keep, new_datamap } => {
            let kept = match node.as_ref() {
                Node::Data(d) => d.slots[keep].clone(),
                _ => unreachable!(),
            };
            *node = Rc::new(Node::Data(DataNode {
                edit: Cell::new(edit.unwrap_or(0)),
                datamap: new_datamap,
                nodemap: 0,
                slots: vec![kept],
            }));
        }
        Act::Recurse { i, b } => {
            let n = ensure(node, edit);
            let d = match n {
                Node::Data(d) => d,
                _ => unreachable!(),
            };
            let child_single = match &mut d.slots[i] {
                Slot::Node(child) => {
                    without_present(child, edit, shift + SHIFT, hash, key);
                    child.is_single()
                }
                Slot::Entry { .. } => unreachable!("node region holds nodes only"),
            };
            if !child_single {
                return;
            }
            if d.datamap == 0 && d.nodemap.count_ones() == 1 {
                // Only child left: collapse this node away entirely.
                let child = match &d.slots[i] {
                    Slot::Node(c) => c.clone(),
                    _ => unreachable!(),
                };
                *node = child;
                return;
            }
            // copyAndMigrateToInline: lift the collapsed child's pair into
            // this node's data region.
            let child = match &d.slots[i] {
                Slot::Node(c) => c.clone(),
                _ => unreachable!(),
            };
            let (ehash, ekey, evalue) = match child.as_ref() {
                Node::Data(cd) => match &cd.slots[0] {
                    Slot::Entry { hash, key, value } => (*hash, key.clone(), value.clone()),
                    Slot::Node(_) => unreachable!("single-pair node holds an entry"),
                },
                Node::Collision(cc) => {
                    let (k, v) = cc.entries[0].clone();
                    (cc.hash, k, v)
                }
            };
            let p = index(d.nodemap, b);
            let node_pos =
                d.datamap.count_ones() as usize + (d.nodemap.count_ones() as usize - 1 - p);
            d.slots.remove(node_pos);
            d.nodemap ^= b;
            d.datamap |= b;
            let data_pos = index(d.datamap, b);
            d.slots.insert(
                data_pos,
                Slot::Entry {
                    hash: ehash,
                    key: ekey,
                    value: evalue,
                },
            );
        }
        Act::CollisionRemove(i) => {
            if let Node::Collision(c) = ensure(node, edit) {
                c.entries.remove(i);
            }
        }
        Act::CollisionToData => {
            // Java: `EMPTY.assoc(edit, 0, hash, remaining)` — a root-level
            // single-pair DataNode keyed by the shared collision hash.
            let (k, v) = match node.as_ref() {
                Node::Collision(c) => c
                    .entries
                    .iter()
                    .find(|(k, _)| k != key)
                    .cloned()
                    .expect("other entry is present"),
                _ => unreachable!(),
            };
            *node = Rc::new(Node::Data(DataNode {
                edit: Cell::new(edit.unwrap_or(0)),
                datamap: bit(hash, 0),
                nodemap: 0,
                slots: vec![Slot::Entry {
                    hash,
                    key: k,
                    value: v,
                }],
            }));
        }
        Act::CollisionToEmpty => {
            *node = Rc::new(Node::Data(DataNode {
                edit: Cell::new(edit.unwrap_or(0)),
                datamap: 0,
                nodemap: 0,
                slots: Vec::new(),
            }));
        }
    }
}

fn collect<'a, K, V>(node: &'a Node<K, V>, out: &mut Vec<(&'a K, &'a V)>) {
    match node {
        Node::Collision(c) => out.extend(c.entries.iter().map(|(k, v)| (k, v))),
        Node::Data(d) => {
            let data_arity = d.datamap.count_ones() as usize;
            for slot in &d.slots[..data_arity] {
                match slot {
                    Slot::Entry { key, value, .. } => out.push((key, value)),
                    Slot::Node(_) => unreachable!("data region holds entries only"),
                }
            }
            // Java NodeIter descends sub-nodes from the highest bitpos down;
            // that is the node region from `slots[data_arity]` forward.
            for slot in &d.slots[data_arity..] {
                match slot {
                    Slot::Node(child) => collect(child, out),
                    Slot::Entry { .. } => unreachable!("node region holds nodes only"),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Standard<K, V> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    root: Rc<Node<K, V>>,
    size: usize,
}
impl<K, V> Default for Standard<K, V> {
    fn default() -> Self {
        Self {
            metadata: None,
            root: Node::empty(),
            size: 0,
        }
    }
}
impl<K: Clone + Eq + Hash, V: Clone> Standard<K, V> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        find_node(&self.root, 0, key_hash(key), key).map(|(_, v)| v)
    }
    pub fn find_entry(&self, key: &K) -> Option<(&K, &V)> {
        find_node(&self.root, 0, key_hash(key), key)
    }
    pub fn assoc_value(&self, key: K, value: V) -> Self {
        let mut root = self.root.clone();
        let added = assoc_node(&mut root, None, 0, key_hash(&key), key, value);
        Self {
            metadata: self.metadata.clone(),
            root,
            size: self.size + usize::from(added),
        }
    }
    /// Associates into a consumed map using clone-on-write nodes. This keeps
    /// persistent aliases immutable while allowing uniquely owned paths to be
    /// updated without first cloning every node on the path.
    pub fn assoc_value_owned(mut self, key: K, value: V) -> Self {
        let added = assoc_node(&mut self.root, Some(0), 0, key_hash(&key), key, value);
        self.size += usize::from(added);
        self
    }
    pub fn dissoc_value(&self, key: &K) -> Self {
        let hash = key_hash(key);
        if find_node(&self.root, 0, hash, key).is_none() {
            return self.clone();
        }
        let mut root = self.root.clone();
        without_present(&mut root, None, 0, hash, key);
        Self {
            metadata: self.metadata.clone(),
            root,
            size: self.size - 1,
        }
    }
    pub fn iter(&self) -> std::vec::IntoIter<(&K, &V)> {
        self.entries().into_iter()
    }
    pub fn entries(&self) -> Vec<(&K, &V)> {
        let mut out = Vec::with_capacity(self.size);
        collect(&self.root, &mut out);
        out
    }
    pub fn shares_root_with(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.root, &other.root)
    }
}
impl<K: Clone + Eq + Hash, V: Clone> FromIterator<(K, V)> for Standard<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::new(), |map, (k, v)| map.assoc_value(k, v))
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IntoIterator for Standard<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries()
            .into_iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>()
            .into_iter()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + PartialEq> PartialEq for Standard<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.entries().iter().all(|(k, v)| other.get(k) == Some(*v))
    }
}
impl<K: Clone + Eq + Hash, V: Clone> ICount for Standard<K, V> {
    fn count(&self) -> usize {
        self.size
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IAssoc<K, V> for Standard<K, V> {
    type Output = Self;
    fn assoc(&self, key: K, value: V) -> Self {
        self.assoc_value(key, value)
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IDissoc<K> for Standard<K, V> {
    type Output = Self;
    fn dissoc(&self, key: &K) -> Self {
        self.dissoc_value(key)
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
        self.entries()
            .into_iter()
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>()
            .into_iter()
    }
    fn vals(&self) -> Self::Values {
        self.entries()
            .into_iter()
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>()
            .into_iter()
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IEmpty for Standard<K, V> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.metadata.clone())
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IMetadata for Standard<K, V> {
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

    fn metatype(&self) -> MetaType {
        MetaType::Map
    }
}
impl<K: Clone + Eq + Hash, V: Clone> IPersistent for Standard<K, V> {}
impl<K: Clone + Eq + Hash, V: Clone> IConj<(K, V)> for Standard<K, V> {
    type Output = Self;
    fn conj(&self, (key, value): (K, V)) -> Self {
        self.assoc_value(key, value)
    }
}
impl<K: Clone + Eq + Hash, V: Clone + PartialEq> IEquality for Standard<K, V> {
    fn equality(&self, other: &Self) -> bool {
        self == other
    }
}
impl<K: Clone + Eq + Hash + std::fmt::Debug, V: Clone + std::fmt::Debug> IDisplay
    for Standard<K, V>
{
    fn display(&self) -> String {
        format!(
            "{{{}}}",
            self.entries()
                .iter()
                .map(|(k, v)| format!("{k:?} {v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<K: Clone + Eq + Hash + JavaHash, V: Clone + Hash + JavaHash> IHash for Standard<K, V> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java IMapType → IUnOrderedType over entries: order-insensitive sum,
        // "::MAP" seed. Entries iterate as MapEntry values in Java, so each
        // entry hashes as an ordered 2-tuple ("::SEQUENTIAL"
        // seed): (seed * 31 + hk) * 31 + hv. See lang::hash.
        crate::lang::hash::compose_unordered(
            "MAP",
            self.entries().iter().map(|(k, v)| {
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
            token: fresh_edit(),
            standard: self.clone(),
        }
    }
}

thread_local! {
    static NEXT_EDIT: Cell<u64> = const { Cell::new(1) };
}
fn fresh_edit() -> u64 {
    NEXT_EDIT.with(|c| {
        let token = c.get();
        c.set(token + 1);
        token
    })
}

#[derive(Debug, Clone)]
pub struct Mutable<K, V> {
    editable: Cell<bool>,
    token: u64,
    standard: Standard<K, V>,
}
impl<K: Clone + Eq + Hash, V: Clone> Mutable<K, V> {
    fn check(&self) {
        assert!(self.editable.get(), "mutable map used after to_persistent")
    }
    pub fn assoc(&mut self, key: K, value: V) -> &mut Self {
        self.check();
        let hash = key_hash(&key);
        // Take the root out first: otherwise this struct's own handle keeps
        // the Rc shared and in-place editing never triggers.
        let mut root = std::mem::replace(&mut self.standard.root, Node::empty());
        let added = assoc_node(&mut root, Some(self.token), 0, hash, key, value);
        self.standard.root = root;
        self.standard.size += usize::from(added);
        self
    }
    pub fn dissoc(&mut self, key: &K) -> &mut Self {
        self.check();
        let hash = key_hash(key);
        if find_node(&self.standard.root, 0, hash, key).is_some() {
            let mut root = std::mem::replace(&mut self.standard.root, Node::empty());
            without_present(&mut root, Some(self.token), 0, hash, key);
            self.standard.root = root;
            self.standard.size -= 1;
        }
        self
    }
}
impl<K: Clone + Eq + Hash, V: Clone> std::ops::Deref for Mutable<K, V> {
    type Target = Standard<K, V>;
    fn deref(&self) -> &Self::Target {
        self.check();
        &self.standard
    }
}
impl<K, V> IMutable for Mutable<K, V> {}
impl<K: Clone + Eq + Hash, V: Clone> IToPersistent for Mutable<K, V> {
    type Persistent = Standard<K, V>;
    fn to_persistent(&mut self) -> Self::Persistent {
        self.check();
        self.editable.set(false);
        self.standard.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Standard;
    use crate::core::Value;
    use crate::lang::protocol::{IEmpty, IMetadata, IToMutable, IToPersistent};
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Collision(i32);
    impl Hash for Collision {
        fn hash<H: Hasher>(&self, state: &mut H) {
            0.hash(state)
        }
    }

    /// Key whose `Hash` writes its inner value as a single `write_u64`, so
    /// `key_hash`'s probe captures it and placement uses its low 32 bits
    /// exactly — controlled CHAMP placement without touching the fallback.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
    struct Key(u64);
    impl Hash for Key {
        fn hash<H: Hasher>(&self, state: &mut H) {
            state.write_u64(self.0);
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    #[test]
    fn persistent_operations_and_mutable_round_trip_preserve_metadata() {
        let map = Standard::new()
            .assoc_value("a", 1)
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        assert_eq!(
            map.assoc_value("b", 2).meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(
            map.dissoc_value(&"a").meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(map.empty().meta().map(|m| m.doc().unwrap()), Some("doc"));
        let mut mutable = map.to_mutable();
        mutable.assoc("b", 2);
        assert_eq!(
            mutable.to_persistent().meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
    }

    #[test]
    fn assoc_collision_removal_and_persistence() {
        let empty = Standard::new();
        let a = empty.assoc_value(Collision(1), 10);
        let b = a.assoc_value(Collision(2), 20);
        let c = b.dissoc_value(&Collision(1));
        assert_eq!(a.get(&Collision(1)), Some(&10));
        assert_eq!(b.get(&Collision(2)), Some(&20));
        assert_eq!(c.get(&Collision(1)), None);
        assert_eq!(c.get(&Collision(2)), Some(&20));
        assert!(empty.shares_root_with(&empty.dissoc_value(&Collision(9))));
    }

    #[test]
    fn assoc_get_overwrite_and_dissoc_basics() {
        let mut map = Standard::new();
        for i in 0..100u64 {
            map = map.assoc_value(i, i * 10);
        }
        assert_eq!(map.len(), 100);
        for i in 0..100u64 {
            assert_eq!(map.get(&i), Some(&(i * 10)));
        }
        // Overwrite keeps size.
        let overwritten = map.assoc_value(42, 999);
        assert_eq!(overwritten.len(), 100);
        assert_eq!(overwritten.get(&42), Some(&999));
        assert_eq!(map.get(&42), Some(&420));
        // Dissoc down to empty.
        for i in 0..100u64 {
            map = map.dissoc_value(&i);
        }
        assert!(map.is_empty());
        assert_eq!(map.get(&0), None);
    }

    #[test]
    fn captured_hash_collision_converts_back_to_single_pair() {
        // Both keys capture low-32 == 1: identical placement hash, so they
        // land in a collision node past shift 32.
        let a = Key(1);
        let b = Key(0x1_0000_0001);
        let map = Standard::new().assoc_value(a, 10).assoc_value(b, 20);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&a), Some(&10));
        assert_eq!(map.get(&b), Some(&20));
        // Overwrite inside a collision node keeps the size exact.
        let overwritten = map.assoc_value(a, 99);
        assert_eq!(overwritten.len(), 2);
        assert_eq!(overwritten.get(&a), Some(&99));
        assert_eq!(map.get(&a), Some(&10));
        // Removing one of two converts the node to a single-pair DataNode.
        let one = map.dissoc_value(&a);
        assert_eq!(one.len(), 1);
        assert_eq!(one.get(&a), None);
        assert_eq!(one.get(&b), Some(&20));
        // The pair still iterates.
        let entries: Vec<_> = one.entries().into_iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(entries, vec![(b, 20)]);
    }

    /// Churn a map against a `HashMap` model, twice (DefaultHasher fallback
    /// keys and probe-captured keys over a small space to force collisions
    /// and shared-mask sub-nodes), then require deterministic iteration.
    fn churn_build<K: Clone + Eq + Hash + Ord + std::fmt::Debug>(
        seed: u64,
        mk: impl Fn(u64) -> K,
    ) -> (Standard<K, u64>, HashMap<K, u64>) {
        let mut rng = Rng(seed);
        let mut map = Standard::new();
        let mut model = HashMap::new();
        for _ in 0..300 {
            let key = mk(rng.next() % 24);
            if rng.next() % 3 == 0 {
                map = map.dissoc_value(&key);
                model.remove(&key);
            } else {
                let value = rng.next();
                map = map.assoc_value(key.clone(), value);
                model.insert(key, value);
            }
        }
        (map, model)
    }

    fn churn_case<K: Clone + Eq + Hash + Ord + std::fmt::Debug>(mk: impl Fn(u64) -> K + Copy) {
        let (map, model) = churn_build(0x9e37_79b9_7f4a_7c15, mk);
        assert_eq!(map.len(), model.len());
        for (k, v) in &model {
            assert_eq!(map.get(k), Some(v));
        }
        let mut got: Vec<(K, u64)> = map
            .entries()
            .into_iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let mut want: Vec<(K, u64)> = model.iter().map(|(k, v)| (k.clone(), *v)).collect();
        got.sort();
        want.sort();
        assert_eq!(got, want);
        // Iteration determinism: an identical build yields identical order.
        let (again, _) = churn_build(0x9e37_79b9_7f4a_7c15, mk);
        let got_again: Vec<(K, u64)> = again
            .entries()
            .into_iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let got_unsorted: Vec<(K, u64)> = map
            .entries()
            .into_iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        assert_eq!(got_unsorted, got_again);
    }

    #[test]
    fn churn_matches_hashmap_model() {
        churn_case(|i| i as i64);
        churn_case(Key);
    }

    #[test]
    fn transient_bulk_assoc_matches_persistent_build() {
        let mut rng = Rng(42);
        let mut mutable = Standard::new().to_mutable();
        let mut persistent = Standard::new();
        for _ in 0..1000 {
            let key = rng.next() % 500;
            let value = rng.next();
            mutable.assoc(key, value);
            persistent = persistent.assoc_value(key, value);
        }
        let frozen = mutable.to_persistent();
        assert_eq!(frozen.len(), persistent.len());
        // Content AND iteration order must match the persistent build.
        let transient_entries: Vec<_> = frozen
            .entries()
            .into_iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        let persistent_entries: Vec<_> = persistent
            .entries()
            .into_iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        assert_eq!(transient_entries, persistent_entries);
        // Persistent source remains intact after transient edits.
        let mut check = Standard::new();
        let mut rng = Rng(42);
        let mut expected_len = 0;
        let mut seen = HashMap::new();
        for _ in 0..1000 {
            let key = rng.next() % 500;
            let value = rng.next();
            if seen.insert(key, value).is_none() {
                expected_len += 1;
            }
            check = check.assoc_value(key, value);
        }
        assert_eq!(check.len(), expected_len);
    }

    #[test]
    fn transient_assoc_dissoc_cycles_stay_correct() {
        let mut m = Standard::new().to_mutable();
        for round in 0..50u64 {
            for i in 0..20u64 {
                m.assoc(round * 20 + i, i);
            }
            for i in 0..20u64 {
                m.dissoc(&(round * 20 + i));
            }
            assert_eq!(m.len(), 0);
        }
        for i in 0..100u64 {
            m.assoc(i, i * 2);
        }
        for i in (0..100u64).step_by(2) {
            m.dissoc(&i);
        }
        assert_eq!(m.len(), 50);
        let frozen = m.to_persistent();
        assert_eq!(frozen.len(), 50);
        for i in (1..100u64).step_by(2) {
            assert_eq!(frozen.get(&i), Some(&(i * 2)));
        }
        // The frozen map is persistent: further dissoc does not alias it.
        let smaller = frozen.dissoc_value(&1);
        assert_eq!(smaller.len(), 49);
        assert_eq!(frozen.len(), 50);
        assert_eq!(frozen.get(&1), Some(&2));
    }

    #[test]
    fn integer_churn_matches_java_champ_order() {
        let mut map = Standard::new();
        for i in 0..30i64 {
            map = map.assoc_value(Value::Number(i), i);
        }
        for i in (0..30i64).step_by(3) {
            map = map.dissoc_value(&Value::Number(i));
        }
        let entries: Vec<_> = map
            .entries()
            .into_iter()
            .map(|(k, _)| match k {
                Value::Number(value) => *value,
                other => panic!("unexpected key: {other:?}"),
            })
            .collect();
        assert_eq!(
            entries,
            vec![29, 28, 26, 25, 23, 22, 20, 19, 17, 16, 14, 13, 11, 10, 8, 7, 5, 4, 2, 1]
        );
    }

    #[test]
    #[should_panic(expected = "mutable map used after to_persistent")]
    fn use_after_to_persistent_panics() {
        let mut m = Standard::new().to_mutable();
        m.assoc(1u64, 1u64);
        let _ = m.to_persistent();
        m.assoc(2u64, 2u64);
    }
}
