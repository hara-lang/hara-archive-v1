//! Persistent sorted map: a weight-recorded red-black tree ported from
//! `java/src/main/java/hara/lang/data/SortedMap.java`.
//!
//! Leaf modelling: Java uses two non-null leaf sentinels (`EMPTY_NODE`,
//! colour BLACK, and `DOUBLE_EMPTY_NODE`, colour DOUBLE_BLACK; both size 0)
//! so the delete rebalancer can push a "double black" up from a removed
//! leaf. Rust has no null links, so the child link itself is the three-state
//! enum [`Link`]: `Empty`/`DoubleEmpty` are the two size-0 leaves and
//! `Full(Rc<Node>)` is an internal node. Every colour test in
//! `balance`/`rotate` is then a direct transcription of the Java code.

use std::cell::Cell;
use std::cmp::Ordering;
use std::rc::Rc;

use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IAssoc, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash,
    IIndexedKV, ILookup, IMetadata, IMutable, INth, IObjType, IPersistent, IToMutable,
    IToPersistent, MetaType, ObjType,
};

/// Java `SortedMap.Color`; DOUBLE_BLACK only exists transiently during delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Color {
    Red,
    Black,
    DoubleBlack,
}
use Color::{Black, DoubleBlack, Red};

/// Child link: the two size-0 leaf sentinels or an internal node.
#[derive(Debug, Clone)]
enum Link<K, V> {
    /// Java `Node.EMPTY_NODE` (BLACK, size 0).
    Empty,
    /// Java `Node.DOUBLE_EMPTY_NODE` (DOUBLE_BLACK, size 0).
    DoubleEmpty,
    Full(Rc<Node<K, V>>),
}

#[derive(Debug, Clone)]
pub struct Node<K, V> {
    pub key: K,
    pub value: V,
    color: Color,
    left: Link<K, V>,
    right: Link<K, V>,
    size: usize,
}

impl<K, V> Link<K, V> {
    fn color(&self) -> Color {
        match self {
            Link::Empty => Black,
            Link::DoubleEmpty => DoubleBlack,
            Link::Full(node) => node.color,
        }
    }
    fn size(&self) -> usize {
        match self {
            Link::Full(node) => node.size,
            _ => 0,
        }
    }
}

/// Java `S.node`: weight-recorded constructor, size = l.size + r.size + 1.
fn node<K, V>(color: Color, left: Link<K, V>, key: K, value: V, right: Link<K, V>) -> Link<K, V> {
    Link::Full(Rc::new(Node {
        size: left.size() + right.size() + 1,
        color,
        left,
        key,
        value,
        right,
    }))
}
fn red<K, V>(left: Link<K, V>, key: K, value: V, right: Link<K, V>) -> Link<K, V> {
    node(Red, left, key, value, right)
}
fn black<K, V>(left: Link<K, V>, key: K, value: V, right: Link<K, V>) -> Link<K, V> {
    node(Black, left, key, value, right)
}

/// Java `Node.redden`: BLACK with two BLACK children becomes RED (delete prep).
fn redden<K: Clone, V: Clone>(link: &Link<K, V>) -> Link<K, V> {
    match link {
        Link::Full(n)
            if n.color == Black && n.left.color() == Black && n.right.color() == Black =>
        {
            red(
                n.left.clone(),
                n.key.clone(),
                n.value.clone(),
                n.right.clone(),
            )
        }
        _ => link.clone(),
    }
}
/// Java `Node.blacken`: RED becomes BLACK.
fn blacken<K: Clone, V: Clone>(link: &Link<K, V>) -> Link<K, V> {
    match link {
        Link::Full(n) if n.color == Red => black(
            n.left.clone(),
            n.key.clone(),
            n.value.clone(),
            n.right.clone(),
        ),
        _ => link.clone(),
    }
}
/// Java `Node.unblacken`: DOUBLE_BLACK becomes BLACK (also maps the
/// double-empty leaf back to the plain empty leaf).
fn unblacken<K: Clone, V: Clone>(link: &Link<K, V>) -> Link<K, V> {
    match link {
        Link::DoubleEmpty => Link::Empty,
        Link::Full(n) if n.color == DoubleBlack => black(
            n.left.clone(),
            n.key.clone(),
            n.value.clone(),
            n.right.clone(),
        ),
        _ => link.clone(),
    }
}

/// Java `Node.put` with assoc's merge `(o, n) -> n`: recursive put, root
/// blackened on the way out.
fn put<K: Clone + Ord, V: Clone>(root: &Link<K, V>, key: K, value: V) -> Link<K, V> {
    blacken(&put_rec(root, key, value))
}
fn put_rec<K: Clone + Ord, V: Clone>(link: &Link<K, V>, key: K, value: V) -> Link<K, V> {
    match link {
        // empty slot -> RED node (BLACK when the slot was double-empty)
        Link::Empty => red(Link::Empty, key, value, Link::Empty),
        Link::DoubleEmpty => black(Link::Empty, key, value, Link::Empty),
        Link::Full(n) => match key.cmp(&n.key) {
            Ordering::Less => balance(node(
                n.color,
                put_rec(&n.left, key, value),
                n.key.clone(),
                n.value.clone(),
                n.right.clone(),
            )),
            Ordering::Greater => balance(node(
                n.color,
                n.left.clone(),
                n.key.clone(),
                n.value.clone(),
                put_rec(&n.right, key, value),
            )),
            Ordering::Equal => node(n.color, n.left.clone(), key, value, n.right.clone()),
        },
    }
}

/// Java `Node.remove`: redden the root, recursive remove; an unchanged size
/// means the key was absent, so keep the original (non-reddened) root.
fn remove<K: Clone + Ord, V: Clone>(root: &Link<K, V>, key: &K) -> Link<K, V> {
    let result = remove_rec(&redden(root), key);
    if result.size() == root.size() {
        root.clone()
    } else {
        result
    }
}
fn remove_rec<K: Clone + Ord, V: Clone>(link: &Link<K, V>, key: &K) -> Link<K, V> {
    let Link::Full(n) = link else {
        return link.clone();
    };
    match key.cmp(&n.key) {
        Ordering::Less => rotate(node(
            n.color,
            remove_rec(&n.left, key),
            n.key.clone(),
            n.value.clone(),
            n.right.clone(),
        )),
        Ordering::Greater => rotate(node(
            n.color,
            n.left.clone(),
            n.key.clone(),
            n.value.clone(),
            remove_rec(&n.right, key),
        )),
        Ordering::Equal => {
            if n.size == 1 {
                // leaf removal: RED leaves vanish, BLACK leaves go double-black
                if n.color == Black {
                    Link::DoubleEmpty
                } else {
                    Link::Empty
                }
            } else if n.right.size() == 0 {
                blacken(&n.left)
            } else {
                let min = leftmost(&n.right);
                rotate(node(
                    n.color,
                    n.left.clone(),
                    min.key.clone(),
                    min.value.clone(),
                    remove_min(&n.right),
                ))
            }
        }
    }
}

/// Java `S.min`: leftmost node of a non-empty subtree.
fn leftmost<K, V>(link: &Link<K, V>) -> &Rc<Node<K, V>> {
    let Link::Full(n) = link else {
        unreachable!("min of an empty subtree")
    };
    match &n.left {
        Link::Full(_) => leftmost(&n.left),
        _ => n,
    }
}

/// Java `Node.removeMin`: mirror of `remove_rec`, rotating on the way up.
fn remove_min<K: Clone, V: Clone>(link: &Link<K, V>) -> Link<K, V> {
    let Link::Full(n) = link else {
        return link.clone();
    };
    if n.left.size() == 0 {
        return match n.color {
            Red => Link::Empty,
            _ if n.right.size() == 0 => Link::DoubleEmpty,
            _ => blacken(&n.right),
        };
    }
    rotate(node(
        n.color,
        remove_min(&n.left),
        n.key.clone(),
        n.value.clone(),
        n.right.clone(),
    ))
}

/// Java `Node.balance`: BLACK -> balanceBlack, DOUBLE_BLACK ->
/// balanceDoubleBlack, RED and the leaves pass through.
fn balance<K: Clone, V: Clone>(link: Link<K, V>) -> Link<K, V> {
    let Link::Full(_) = link else {
        return link;
    };
    match link.color() {
        Black => balance_black(&link),
        DoubleBlack => balance_double_black(&link),
        Red => link,
    }
}

/// Java `Node.balanceBlack`: the four classic Okasaki red-red cases.
fn balance_black<K: Clone, V: Clone>(link: &Link<K, V>) -> Link<K, V> {
    let Link::Full(n) = link else {
        return link.clone();
    };
    if let Link::Full(l) = &n.left {
        if l.color == Red {
            // (B (R (R a x b) y c) z d) -> (R (B a x b) y (B c z d))
            if let Link::Full(ll) = &l.left {
                if ll.color == Red {
                    return red(
                        blacken(&l.left),
                        l.key.clone(),
                        l.value.clone(),
                        black(
                            l.right.clone(),
                            n.key.clone(),
                            n.value.clone(),
                            n.right.clone(),
                        ),
                    );
                }
            }
            // (B (R a x (R b y c)) z d) -> (R (B a x b) y (B c z d))
            if let Link::Full(lr) = &l.right {
                if lr.color == Red {
                    return red(
                        black(
                            l.left.clone(),
                            l.key.clone(),
                            l.value.clone(),
                            lr.left.clone(),
                        ),
                        lr.key.clone(),
                        lr.value.clone(),
                        black(
                            lr.right.clone(),
                            n.key.clone(),
                            n.value.clone(),
                            n.right.clone(),
                        ),
                    );
                }
            }
        }
    }
    if let Link::Full(r) = &n.right {
        if r.color == Red {
            // (B a x (R (R b y c) z d)) -> (R (B a x b) y (B c z d))
            if let Link::Full(rl) = &r.left {
                if rl.color == Red {
                    return red(
                        black(
                            n.left.clone(),
                            n.key.clone(),
                            n.value.clone(),
                            rl.left.clone(),
                        ),
                        rl.key.clone(),
                        rl.value.clone(),
                        black(
                            rl.right.clone(),
                            r.key.clone(),
                            r.value.clone(),
                            r.right.clone(),
                        ),
                    );
                }
            }
            // (B a x (R b y (R c z d))) -> (R (B a x b) y (B c z d))
            if let Link::Full(rr) = &r.right {
                if rr.color == Red {
                    return red(
                        black(
                            n.left.clone(),
                            n.key.clone(),
                            n.value.clone(),
                            r.left.clone(),
                        ),
                        r.key.clone(),
                        r.value.clone(),
                        blacken(&r.right),
                    );
                }
            }
        }
    }
    link.clone()
}

/// Java `Node.balanceDoubleBlack`: the two red-red cases under a double black.
fn balance_double_black<K: Clone, V: Clone>(link: &Link<K, V>) -> Link<K, V> {
    let Link::Full(n) = link else {
        return link.clone();
    };
    // (BB (R a x (R b y c)) z d) -> (B (B a x b) y (B c z d))
    if let Link::Full(l) = &n.left {
        if l.color == Red {
            if let Link::Full(lr) = &l.right {
                if lr.color == Red {
                    return black(
                        black(
                            l.left.clone(),
                            l.key.clone(),
                            l.value.clone(),
                            lr.left.clone(),
                        ),
                        lr.key.clone(),
                        lr.value.clone(),
                        black(
                            lr.right.clone(),
                            n.key.clone(),
                            n.value.clone(),
                            n.right.clone(),
                        ),
                    );
                }
            }
        }
    }
    // (BB a x (R (R b y c) z d)) -> (B (B a x b) y (B c z d))
    if let Link::Full(r) = &n.right {
        if r.color == Red {
            if let Link::Full(rl) = &r.left {
                if rl.color == Red {
                    return black(
                        black(
                            n.left.clone(),
                            n.key.clone(),
                            n.value.clone(),
                            rl.left.clone(),
                        ),
                        rl.key.clone(),
                        rl.value.clone(),
                        black(
                            rl.right.clone(),
                            r.key.clone(),
                            r.value.clone(),
                            r.right.clone(),
                        ),
                    );
                }
            }
        }
    }
    link.clone()
}

/// Java `Node.rotate`: the double-black rebalancer, six pattern cases, each
/// ending in `balance`. Called on every return path of remove/removeMin.
fn rotate<K: Clone, V: Clone>(link: Link<K, V>) -> Link<K, V> {
    let Link::Full(n) = &link else {
        return link;
    };
    match n.color {
        Red => {
            // (R (BB? a-x-b) y (B c z d)) -> (balance (B (R (-B a-x-b) y c) z d))
            if n.left.color() == DoubleBlack && n.right.color() == Black {
                let Link::Full(r) = &n.right else {
                    unreachable!("sibling of a double black must be an internal node")
                };
                return balance(black(
                    red(
                        unblacken(&n.left),
                        n.key.clone(),
                        n.value.clone(),
                        r.left.clone(),
                    ),
                    r.key.clone(),
                    r.value.clone(),
                    r.right.clone(),
                ));
            }
            // (R (B a x b) y (BB? c-z-d)) -> (balance (B a x (R b y (-B c-z-d))))
            if n.right.color() == DoubleBlack && n.left.color() == Black {
                let Link::Full(l) = &n.left else {
                    unreachable!("sibling of a double black must be an internal node")
                };
                return balance(black(
                    l.left.clone(),
                    l.key.clone(),
                    l.value.clone(),
                    red(
                        l.right.clone(),
                        n.key.clone(),
                        n.value.clone(),
                        unblacken(&n.right),
                    ),
                ));
            }
        }
        Black => {
            // (B (BB? a-x-b) y (B c z d)) -> (balance (BB (R (-B a-x-b) y c) z d))
            if n.left.color() == DoubleBlack && n.right.color() == Black {
                let Link::Full(r) = &n.right else {
                    unreachable!("sibling of a double black must be an internal node")
                };
                return balance(node(
                    DoubleBlack,
                    red(
                        unblacken(&n.left),
                        n.key.clone(),
                        n.value.clone(),
                        r.left.clone(),
                    ),
                    r.key.clone(),
                    r.value.clone(),
                    r.right.clone(),
                ));
            }
            // (B (B a x b) y (BB? c-z-d)) -> (balance (BB a x (R b y (-B c-z-d))))
            if n.left.color() == Black && n.right.color() == DoubleBlack {
                let Link::Full(l) = &n.left else {
                    unreachable!("sibling of a double black must be an internal node")
                };
                return balance(node(
                    DoubleBlack,
                    l.left.clone(),
                    l.key.clone(),
                    l.value.clone(),
                    red(
                        l.right.clone(),
                        n.key.clone(),
                        n.value.clone(),
                        unblacken(&n.right),
                    ),
                ));
            }
            // (B (BB? a-w-b) x (R (B c y d) z e))
            // -> (B (balance (B (R (-B a-w-b) x c) y d)) z e)
            if n.left.color() == DoubleBlack && n.right.color() == Red {
                let Link::Full(r) = &n.right else {
                    unreachable!("sibling of a double black must be an internal node")
                };
                if let Link::Full(rl) = &r.left {
                    if rl.color == Black {
                        return black(
                            balance(black(
                                red(
                                    unblacken(&n.left),
                                    n.key.clone(),
                                    n.value.clone(),
                                    rl.left.clone(),
                                ),
                                rl.key.clone(),
                                rl.value.clone(),
                                rl.right.clone(),
                            )),
                            r.key.clone(),
                            r.value.clone(),
                            r.right.clone(),
                        );
                    }
                }
            }
            // (B (R a w (B b x c)) y (BB? d-z-e))
            // -> (B a w (balance (B b x (R c y (-B d-z-e)))))
            if n.left.color() == Red && n.right.color() == DoubleBlack {
                let Link::Full(l) = &n.left else {
                    unreachable!("sibling of a double black must be an internal node")
                };
                if let Link::Full(lr) = &l.right {
                    if lr.color == Black {
                        return black(
                            l.left.clone(),
                            l.key.clone(),
                            l.value.clone(),
                            balance(black(
                                lr.left.clone(),
                                lr.key.clone(),
                                lr.value.clone(),
                                red(
                                    lr.right.clone(),
                                    n.key.clone(),
                                    n.value.clone(),
                                    unblacken(&n.right),
                                ),
                            )),
                        );
                    }
                }
            }
        }
        DoubleBlack => {}
    }
    link.clone()
}

/// Java `Node.floorIndex`: rank of the greatest key <= probe, None if none.
fn floor_index<K: Ord, V>(link: &Link<K, V>, key: &K, offset: usize) -> Option<usize> {
    let Link::Full(n) = link else {
        return None;
    };
    match key.cmp(&n.key) {
        Ordering::Greater => {
            floor_index(&n.right, key, offset + n.left.size() + 1).or(Some(offset + n.left.size()))
        }
        Ordering::Less => floor_index(&n.left, key, offset),
        Ordering::Equal => Some(offset + n.left.size()),
    }
}

/// Java `Node.ceilIndex`: rank of the smallest key >= probe, None if none.
fn ceil_index<K: Ord, V>(link: &Link<K, V>, key: &K, offset: usize) -> Option<usize> {
    let Link::Full(n) = link else {
        return None;
    };
    match key.cmp(&n.key) {
        Ordering::Greater => ceil_index(&n.right, key, offset + n.left.size() + 1),
        Ordering::Less => ceil_index(&n.left, key, offset).or(Some(offset + n.left.size())),
        Ordering::Equal => Some(offset + n.left.size()),
    }
}

/// Java `Node.slice`: keep nodes with min <= k <= max, pruning the rest
/// (the static `S.slice` in Java is a dead stub and is not ported).
fn slice<K: Clone + Ord, V: Clone>(link: &Link<K, V>, min: &K, max: &K) -> Link<K, V> {
    let Link::Full(n) = link else {
        return link.clone();
    };
    match (n.key.cmp(min), n.key.cmp(max)) {
        (Ordering::Less, _) => slice(&n.right, min, max),
        (_, Ordering::Greater) => slice(&n.left, min, max),
        _ => rotate(node(
            n.color,
            slice(&n.left, min, max),
            n.key.clone(),
            n.value.clone(),
            slice(&n.right, min, max),
        )),
    }
}

/// Java `Node.mapValues`: same shape and colours, mapped values.
fn map_values<K: Clone, V, U>(link: &Link<K, V>, f: &impl Fn(&K, &V) -> U) -> Link<K, U> {
    match link {
        Link::Empty => Link::Empty,
        Link::DoubleEmpty => Link::DoubleEmpty,
        Link::Full(n) => node(
            n.color,
            map_values(&n.left, f),
            n.key.clone(),
            f(&n.key, &n.value),
            map_values(&n.right, f),
        ),
    }
}

/// Java `Node.checkInvariant`: black-height and no-red-red validation, plus
/// the weight record (size field). Panics on any violation.
#[cfg(test)]
fn check_invariant<K, V>(link: &Link<K, V>) -> usize {
    assert_ne!(link.color(), DoubleBlack, "double black left in tree");
    let Link::Full(n) = link else {
        return 1;
    };
    assert!(
        n.color != Red || (n.left.color() != Red && n.right.color() != Red),
        "red-red violation"
    );
    let left_depth = check_invariant(&n.left);
    let right_depth = check_invariant(&n.right);
    assert_eq!(left_depth, right_depth, "black-height violation");
    assert_eq!(
        n.size,
        n.left.size() + n.right.size() + 1,
        "size record violation"
    );
    left_depth + usize::from(n.color == Black)
}

#[derive(Debug, Clone)]
pub struct Standard<K, V> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    root: Link<K, V>,
}
impl<K, V> Default for Standard<K, V> {
    fn default() -> Self {
        Self {
            metadata: None,
            root: Link::Empty,
        }
    }
}
impl<K: Clone + Ord, V: Clone> Standard<K, V> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.root.size()
    }
    pub fn is_empty(&self) -> bool {
        self.root.size() == 0
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        self.find_entry(key).map(|(_, value)| value)
    }
    /// Java `S.find`.
    pub fn find_entry(&self, key: &K) -> Option<(&K, &V)> {
        let mut cursor = &self.root;
        while let Link::Full(current) = cursor {
            match key.cmp(&current.key) {
                Ordering::Less => cursor = &current.left,
                Ordering::Greater => cursor = &current.right,
                Ordering::Equal => return Some((&current.key, &current.value)),
            }
        }
        None
    }
    pub fn assoc_value(&self, key: K, value: V) -> Self {
        Self {
            metadata: self.metadata.clone(),
            root: put(&self.root, key, value),
        }
    }
    pub fn dissoc_value(&self, key: &K) -> Self {
        Self {
            metadata: self.metadata.clone(),
            root: remove(&self.root, key),
        }
    }
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter::new(&self.root)
    }
    /// Java `Base.nth` (order-statistic select), bounds-checked.
    pub fn nth_entry(&self, mut index: usize) -> Option<&Node<K, V>> {
        let mut cursor = &self.root;
        while let Link::Full(current) = cursor {
            let left_size = current.left.size();
            if index < left_size {
                cursor = &current.left;
            } else if index == left_size {
                return Some(current);
            } else {
                index -= left_size + 1;
                cursor = &current.right;
            }
        }
        None
    }
    /// Java `S.indexOf` (order-statistic rank).
    pub fn index_of_key(&self, key: &K) -> Option<usize> {
        let mut offset = 0;
        let mut cursor = &self.root;
        while let Link::Full(current) = cursor {
            match key.cmp(&current.key) {
                Ordering::Less => cursor = &current.left,
                Ordering::Greater => {
                    offset += current.left.size() + 1;
                    cursor = &current.right;
                }
                Ordering::Equal => return Some(offset + current.left.size()),
            }
        }
        None
    }
    /// Java `Base.inclusiveFloorIndex`.
    pub fn inclusive_floor_index(&self, key: &K) -> Option<usize> {
        floor_index(&self.root, key, 0)
    }
    /// Java `Base.ceilIndex`.
    pub fn ceil_index(&self, key: &K) -> Option<usize> {
        ceil_index(&self.root, key, 0)
    }
    pub fn slice(&self, min: &K, max: &K) -> Self {
        Self {
            metadata: self.metadata.clone(),
            root: slice(&self.root, min, max),
        }
    }
    pub fn map_values<U: Clone>(&self, f: impl Fn(&K, &V) -> U) -> Standard<K, U> {
        Standard {
            metadata: self.metadata.clone(),
            root: map_values(&self.root, &f),
        }
    }
}
impl<K: Clone + Ord, V: Clone> FromIterator<(K, V)> for Standard<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(it: T) -> Self {
        it.into_iter()
            .fold(Self::new(), |m, (k, v)| m.assoc_value(k, v))
    }
}
impl<K: Clone + Ord, V: Clone> ICount for Standard<K, V> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<K: Clone + Ord, V: Clone> IFind<K> for Standard<K, V> {
    type Output = (K, V);
    fn find(&self, k: &K) -> Option<Self::Output> {
        self.find_entry(k).map(|(k, v)| (k.clone(), v.clone()))
    }
}
impl<K: Clone + Ord, V: Clone> ILookup<K, V> for Standard<K, V> {
    type Keys = std::vec::IntoIter<K>;
    type Values = std::vec::IntoIter<V>;
    fn keys(&self) -> Self::Keys {
        self.iter()
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>()
            .into_iter()
    }
    fn vals(&self) -> Self::Values {
        self.iter()
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>()
            .into_iter()
    }
}
impl<K: Clone + Ord, V: Clone> IAssoc<K, V> for Standard<K, V> {
    type Output = Self;
    fn assoc(&self, k: K, v: V) -> Self {
        self.assoc_value(k, v)
    }
}
impl<K: Clone + Ord, V: Clone> IDissoc<K> for Standard<K, V> {
    type Output = Self;
    fn dissoc(&self, k: &K) -> Self {
        self.dissoc_value(k)
    }
}
impl<K: Clone + Ord, V: Clone> INth<Node<K, V>> for Standard<K, V> {
    fn nth(&self, index: usize) -> Option<&Node<K, V>> {
        self.nth_entry(index)
    }
}
impl<K: Clone + Ord, V: Clone + PartialEq> IIndexedKV<K, V> for Standard<K, V> {
    fn index_of_key(&self, key: &K) -> Option<usize> {
        Standard::index_of_key(self, key)
    }
    fn index_of_val(&self, value: &V) -> Option<usize> {
        self.iter().position(|(_, candidate)| candidate == value)
    }
}
impl<K: Clone + Ord, V: Clone> IEmpty for Standard<K, V> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.metadata.clone())
    }
}
impl<K: Clone + Ord, V: Clone> IMetadata for Standard<K, V> {
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
impl<K: Clone + Ord, V: Clone> IPersistent for Standard<K, V> {}
impl<K: Clone + Ord, V: Clone> IntoIterator for Standard<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>()
            .into_iter()
    }
}
impl<K: Clone + Ord, V: Clone> IConj<(K, V)> for Standard<K, V> {
    type Output = Self;
    fn conj(&self, (k, v): (K, V)) -> Self {
        self.assoc_value(k, v)
    }
}
impl<K: Clone + Ord, V: Clone + PartialEq> IEquality for Standard<K, V> {
    fn equality(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}
impl<K: Clone + Ord + std::fmt::Debug, V: Clone + std::fmt::Debug> IDisplay for Standard<K, V> {
    fn display(&self) -> String {
        format!(
            "{{{}}}",
            self.iter()
                .map(|(k, v)| format!("{k:?} {v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<K: Clone + Ord + std::hash::Hash + JavaHash, V: Clone + std::hash::Hash + JavaHash> IHash
    for Standard<K, V>
{
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Same unordered entry composition as the hash map:
        // "::MAP" seed, sum of ordered-entry hashes (see lang::hash).
        crate::lang::hash::compose_unordered(
            "MAP",
            self.iter().map(|(k, v)| {
                crate::lang::hash::compose_entry(k.java_hash(hash_type), v.java_hash(hash_type))
            }),
        ) as u64
    }
}
impl<K: Clone + Ord + std::fmt::Debug, V: Clone + std::fmt::Debug> IObjType for Standard<K, V> {
    fn obj_type(&self) -> ObjType {
        ObjType::Map
    }
}
impl<K, V> IColl<(K, V)> for Standard<K, V>
where
    K: Clone + Ord + std::hash::Hash + JavaHash + std::fmt::Debug,
    V: Clone + PartialEq + std::hash::Hash + JavaHash + std::fmt::Debug,
{
    fn start_string(&self) -> &'static str {
        "{"
    }
    fn end_string(&self) -> &'static str {
        "}"
    }
}
impl<K: Clone + Ord, V: Clone> IToMutable for Standard<K, V> {
    type Mutable = Mutable<K, V>;
    fn to_mutable(&self) -> Self::Mutable {
        Mutable {
            editable: Cell::new(true),
            map: self.clone(),
        }
    }
}

/// In-order iterator over an explicit stack (Java uses a fixed [64] stack
/// and precomputes depth; a Vec stack is the Rust-idiomatic equivalent).
pub struct Iter<'a, K, V> {
    stack: Vec<&'a Node<K, V>>,
}
impl<'a, K, V> Iter<'a, K, V> {
    fn new(root: &'a Link<K, V>) -> Self {
        let mut it = Self { stack: Vec::new() };
        it.push_left(root);
        it
    }
    fn push_left(&mut self, mut link: &'a Link<K, V>) {
        while let Link::Full(current) = link {
            self.stack.push(&**current);
            link = &current.left;
        }
    }
}
impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        let n = self.stack.pop()?;
        self.push_left(&n.right);
        Some((&n.key, &n.value))
    }
}

#[derive(Debug, Clone)]
pub struct Mutable<K, V> {
    editable: Cell<bool>,
    map: Standard<K, V>,
}
impl<K: Clone + Ord, V: Clone> Mutable<K, V> {
    fn check(&self) {
        assert!(
            self.editable.get(),
            "mutable sorted map used after to_persistent"
        )
    }
    pub fn assoc(&mut self, k: K, v: V) -> &mut Self {
        self.check();
        self.map = self.map.assoc_value(k, v);
        self
    }
    pub fn dissoc(&mut self, k: &K) -> &mut Self {
        self.check();
        self.map = self.map.dissoc_value(k);
        self
    }
}
impl<K: Clone + Ord, V: Clone> std::ops::Deref for Mutable<K, V> {
    type Target = Standard<K, V>;
    fn deref(&self) -> &Self::Target {
        self.check();
        &self.map
    }
}
impl<K, V> IMutable for Mutable<K, V> {}
impl<K: Clone + Ord, V: Clone> IToPersistent for Mutable<K, V> {
    type Persistent = Standard<K, V>;
    fn to_persistent(&mut self) -> Self::Persistent {
        self.check();
        self.editable.set(false);
        self.map.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{check_invariant, Link, Standard};
    use std::collections::BTreeMap;

    /// splitmix64: deterministic RNG for the churn fuzzes (no new crates).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn tree_updates_slices_maps_empty_and_mutable_preserve_metadata() {
        use crate::lang::protocol::{IEmpty, IMetadata, IToMutable, IToPersistent};
        let map = [(1, 10), (2, 20), (3, 30)]
            .into_iter()
            .collect::<Standard<_, _>>()
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        assert_eq!(
            map.assoc_value(4, 40).meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(
            map.dissoc_value(&1).meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(
            map.slice(&1, &2).meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(
            map.map_values(|_, value| value + 1)
                .meta()
                .map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(map.empty().meta().map(|m| m.doc().unwrap()), Some("doc"));
        let mut mutable = map.to_mutable();
        mutable.assoc(4, 40);
        assert_eq!(
            mutable.to_persistent().meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
    }

    #[test]
    fn stays_sorted_indexed_and_persistent() {
        let a = [(5, "e"), (1, "a"), (3, "c"), (2, "b"), (4, "d")]
            .into_iter()
            .collect::<Standard<_, _>>();
        assert_eq!(
            a.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(a.index_of_key(&3), Some(2));
        assert_eq!(a.nth_entry(2).map(|node| node.key), Some(3));
        assert_eq!(a.inclusive_floor_index(&0), None);
        assert_eq!(a.inclusive_floor_index(&6), Some(4));
        assert_eq!(a.ceil_index(&0), Some(0));
        let b = a.dissoc_value(&3);
        assert!(b.get(&3).is_none());
        assert!(a.get(&3).is_some());
    }

    #[test]
    fn churn_matches_btree_map_model() {
        for seed in 0..4u64 {
            let mut rng = Rng(seed);
            let mut map = Standard::new();
            let mut model = BTreeMap::new();
            for step in 0..2000 {
                let key = rng.below(400) as i64;
                if rng.below(5) < 3 {
                    let value = rng.next() as i64;
                    map = map.assoc_value(key, value);
                    model.insert(key, value);
                } else {
                    map = map.dissoc_value(&key);
                    model.remove(&key);
                }
                check_invariant(&map.root);
                assert_eq!(map.len(), model.len(), "seed {seed} step {step}");
                assert_eq!(map.get(&key), model.get(&key), "seed {seed} step {step}");
                if step % 100 == 99 {
                    assert!(
                        map.iter().eq(model.iter()),
                        "seed {seed} step {step} contents"
                    );
                }
            }
            assert!(map.iter().eq(model.iter()), "seed {seed} final contents");
        }
    }

    #[test]
    fn rank_ops_match_btree_map_model() {
        let mut rng = Rng(42);
        let mut map = Standard::new();
        let mut model = BTreeMap::new();
        for _ in 0..300 {
            let key = rng.below(200) as i64;
            map = map.assoc_value(key, key * 10);
            model.insert(key, key * 10);
        }
        let keys: Vec<i64> = model.keys().copied().collect();
        // present and absent probes, below/above/between the key range
        for probe in -5..205i64 {
            assert_eq!(
                map.index_of_key(&probe),
                keys.iter().position(|k| *k == probe),
                "index_of {probe}"
            );
            assert_eq!(
                map.inclusive_floor_index(&probe),
                keys.iter().rposition(|k| *k <= probe),
                "floor {probe}"
            );
            assert_eq!(
                map.ceil_index(&probe),
                keys.iter().position(|k| *k >= probe),
                "ceil {probe}"
            );
        }
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(map.nth_entry(i).map(|node| node.key), Some(*key), "nth {i}");
            assert_eq!(
                map.nth_entry(i).map(|node| node.value),
                Some(*key * 10),
                "nth value {i}"
            );
        }
        assert!(map.nth_entry(keys.len()).is_none());
    }

    #[test]
    fn slice_matches_btree_range_and_stays_usable() {
        let mut rng = Rng(7);
        let mut map = Standard::new();
        let mut model = BTreeMap::new();
        for _ in 0..200 {
            let key = rng.below(500) as i64;
            map = map.assoc_value(key, key);
            model.insert(key, key);
        }
        for _ in 0..200 {
            let a = rng.below(550) as i64 - 25;
            let b = rng.below(550) as i64 - 25;
            let (min, max) = if a <= b { (a, b) } else { (b, a) };
            let sliced = map.slice(&min, &max);
            assert_eq!(
                sliced.len(),
                model.range(min..=max).count(),
                "slice [{min},{max}]"
            );
            assert!(
                sliced.iter().eq(model.range(min..=max)),
                "slice [{min},{max}] contents"
            );
            // a sliced map keeps accepting assoc (BST order is preserved even
            // where pruning relaxed the black-height record, as in Java)
            let mut grown = sliced;
            let mut grown_model: BTreeMap<i64, i64> =
                model.range(min..=max).map(|(k, v)| (*k, *v)).collect();
            for _ in 0..20 {
                let key = rng.below(550) as i64 - 25;
                grown = grown.assoc_value(key, key);
                grown_model.insert(key, key);
            }
            assert!(
                grown.iter().eq(grown_model.iter()),
                "slice [{min},{max}] grown"
            );
        }
        assert!(map.slice(&1000, &2000).is_empty());
    }

    #[test]
    fn map_values_maps_every_entry_preserving_order() {
        use crate::lang::protocol::IMetadata;
        let map = (0..50i64)
            .map(|k| (k, k))
            .collect::<Standard<_, _>>()
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        let mapped = map.map_values(|k, v| v + k);
        check_invariant(&mapped.root);
        assert!(mapped
            .iter()
            .map(|(k, v)| (*k, *v))
            .eq((0..50i64).map(|k| (k, k + k))));
        assert_eq!(mapped.meta().map(|m| m.doc().unwrap()), Some("doc"));
    }

    #[test]
    fn sequential_deletes_force_double_black_paths() {
        // ascending insert + descending delete is the classic double-black workout
        let mut map = Standard::new();
        for key in 0..400i64 {
            map = map.assoc_value(key, key);
            check_invariant(&map.root);
        }
        for key in (0..400i64).rev() {
            map = map.dissoc_value(&key);
            check_invariant(&map.root);
            assert_eq!(map.len() as i64, key);
            assert!(map.get(&key).is_none());
            if key % 50 == 0 {
                assert!(map.iter().map(|(k, _)| *k).eq(0..key));
            }
        }
        assert!(map.is_empty());
        assert!(matches!(map.root, Link::Empty));
    }

    #[test]
    fn random_delete_order_down_to_empty() {
        let mut rng = Rng(99);
        let mut keys: Vec<i64> = (0..300).collect();
        for i in (1..keys.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            keys.swap(i, j);
        }
        let mut map: Standard<i64, i64> = (0..300i64).map(|k| (k, k)).collect();
        let mut model: BTreeMap<i64, i64> = (0..300i64).map(|k| (k, k)).collect();
        for key in keys {
            map = map.dissoc_value(&key);
            model.remove(&key);
            check_invariant(&map.root);
            assert_eq!(map.len(), model.len());
            if map.len() % 50 == 0 {
                assert!(map.iter().eq(model.iter()));
            }
        }
        assert!(map.is_empty());
        assert!(matches!(map.root, Link::Empty));
    }

    #[test]
    fn iterator_yields_sorted_order_and_exhausts() {
        let map: Standard<i64, i64> = [(3, 3), (1, 1), (2, 2)].into_iter().collect();
        let mut it = map.iter();
        assert_eq!(it.next(), Some((&1, &1)));
        assert_eq!(it.next(), Some((&2, &2)));
        assert_eq!(it.next(), Some((&3, &3)));
        assert_eq!(it.next(), None);
        assert_eq!(it.next(), None);
        let empty: Standard<i64, i64> = Standard::new();
        assert_eq!(empty.iter().next(), None);
        // deep trees iterate without recursion
        let big: Standard<i64, i64> = (0..10_000i64).map(|k| (k, k)).collect();
        assert!(big.iter().map(|(k, _)| *k).eq(0..10_000));
        assert_eq!(big.len(), 10_000);
    }

    #[test]
    fn transient_round_trip_matches_persistent() {
        use crate::lang::protocol::{IToMutable, IToPersistent};
        let mut rng = Rng(5);
        let mut persistent = Standard::new();
        let mut transient = Standard::new().to_mutable();
        for _ in 0..500 {
            let key = rng.below(300) as i64;
            if rng.below(2) == 0 {
                persistent = persistent.assoc_value(key, key);
                transient.assoc(key, key);
            } else {
                persistent = persistent.dissoc_value(&key);
                transient.dissoc(&key);
            }
        }
        let back = transient.to_persistent();
        assert_eq!(back.len(), persistent.len());
        assert!(back.iter().eq(persistent.iter()));
        check_invariant(&back.root);
    }
}
