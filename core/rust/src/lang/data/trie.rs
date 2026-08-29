use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IAssoc, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash,
    ILookup, IMetadata, IMutable, IObjType, IPersistent, IToMutable, IToPersistent, MetaType,
    ObjType,
};

#[derive(Debug, Clone)]
struct Node<V> {
    children: BTreeMap<char, Rc<Node<V>>>,
    value: Option<V>,
}
impl<V> Default for Node<V> {
    fn default() -> Self {
        Self {
            children: BTreeMap::new(),
            value: None,
        }
    }
}
fn assoc_node<V: Clone>(node: &Rc<Node<V>>, chars: &[char], value: V) -> Rc<Node<V>> {
    if chars.is_empty() {
        return Rc::new(Node {
            children: node.children.clone(),
            value: Some(value),
        });
    }
    let mut children = node.children.clone();
    let child = children
        .get(&chars[0])
        .cloned()
        .unwrap_or_else(|| Rc::new(Node::default()));
    children.insert(chars[0], assoc_node(&child, &chars[1..], value));
    Rc::new(Node {
        children,
        value: node.value.clone(),
    })
}
fn dissoc_node<V: Clone>(node: &Rc<Node<V>>, chars: &[char]) -> Option<Rc<Node<V>>> {
    let mut children = node.children.clone();
    let value = if chars.is_empty() {
        None
    } else {
        let child = children.get(&chars[0])?;
        match dissoc_node(child, &chars[1..]) {
            Some(next) => {
                children.insert(chars[0], next);
            }
            None => {
                children.remove(&chars[0]);
            }
        }
        node.value.clone()
    };
    if value.is_none() && children.is_empty() {
        None
    } else {
        Some(Rc::new(Node { children, value }))
    }
}
#[derive(Debug, Clone)]
pub struct Standard<V> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    root: Rc<Node<V>>,
    size: usize,
}
impl<V> Default for Standard<V> {
    fn default() -> Self {
        Self {
            metadata: None,
            root: Rc::new(Node::default()),
            size: 0,
        }
    }
}
impl<V: Clone> Standard<V> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    pub fn get(&self, key: &str) -> Option<&V> {
        let mut node = self.root.as_ref();
        for ch in key.chars() {
            node = node.children.get(&ch)?.as_ref();
        }
        node.value.as_ref()
    }
    pub fn assoc_value(&self, key: impl Into<String>, value: V) -> Self {
        let key = key.into();
        let added = self.get(&key).is_none();
        let chars = key.chars().collect::<Vec<_>>();
        Self {
            metadata: self.metadata.clone(),
            root: assoc_node(&self.root, &chars, value),
            size: self.size + usize::from(added),
        }
    }
    pub fn dissoc_value(&self, key: &str) -> Self {
        if self.get(key).is_none() {
            return self.clone();
        }
        let chars = key.chars().collect::<Vec<_>>();
        Self {
            metadata: self.metadata.clone(),
            root: dissoc_node(&self.root, &chars).unwrap_or_else(|| Rc::new(Node::default())),
            size: self.size - 1,
        }
    }
    pub fn entries(&self) -> Vec<(String, &V)> {
        fn collect<'a, V>(n: &'a Node<V>, prefix: &mut String, out: &mut Vec<(String, &'a V)>) {
            if let Some(v) = &n.value {
                out.push((prefix.clone(), v));
            }
            for (ch, child) in &n.children {
                prefix.push(*ch);
                collect(child, prefix, out);
                prefix.pop();
            }
        }
        let mut out = Vec::with_capacity(self.size);
        collect(&self.root, &mut String::new(), &mut out);
        out
    }
    pub fn iter(&self) -> impl Iterator<Item = String> + '_ {
        self.entries().into_iter().map(|(key, _)| key)
    }
}
impl<V: Clone> ICount for Standard<V> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<V: Clone> IFind<String> for Standard<V> {
    type Output = (String, V);
    fn find(&self, key: &String) -> Option<Self::Output> {
        self.get(key).map(|v| (key.clone(), v.clone()))
    }
}
impl<V: Clone> ILookup<String, V> for Standard<V> {
    type Keys = std::vec::IntoIter<String>;
    type Values = std::vec::IntoIter<V>;
    fn keys(&self) -> Self::Keys {
        self.iter().collect::<Vec<_>>().into_iter()
    }
    fn vals(&self) -> Self::Values {
        self.entries()
            .into_iter()
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>()
            .into_iter()
    }
}
impl<V: Clone> IAssoc<String, V> for Standard<V> {
    type Output = Self;
    fn assoc(&self, k: String, v: V) -> Self {
        self.assoc_value(k, v)
    }
}
impl<V: Clone> IDissoc<String> for Standard<V> {
    type Output = Self;
    fn dissoc(&self, k: &String) -> Self {
        self.dissoc_value(k)
    }
}
impl<V: Clone + Default> IConj<String> for Standard<V> {
    type Output = Self;
    fn conj(&self, k: String) -> Self {
        self.assoc_value(k, V::default())
    }
}
impl<V: Clone> IEmpty for Standard<V> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.metadata.clone())
    }
}
impl<V: Clone> IMetadata for Standard<V> {
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
impl<V: Clone> IPersistent for Standard<V> {}
impl<V: Clone> IntoIterator for Standard<V> {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().collect::<Vec<_>>().into_iter()
    }
}
impl<V: Clone + PartialEq> IEquality for Standard<V> {
    fn equality(&self, other: &Self) -> bool {
        self.len() == other.len() && self.entries().iter().all(|(k, v)| other.get(k) == Some(*v))
    }
}
impl<V: Clone + std::fmt::Debug> IDisplay for Standard<V> {
    fn display(&self) -> String {
        format!(
            "#{{{}}}",
            self.entries()
                .iter()
                .map(|(k, v)| format!("{k:?} {v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<V: Clone + std::hash::Hash + JavaHash> IHash for Standard<V> {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        // Java Trie.hashCalc override: acc = "::MAP".hashCode(), then
        // acc += hash(key) + hash(value) per entry (NOT the MapEntry
        // composition used by maps). Keys are plain Strings, so they hash
        // via Java String.hashCode under every hash type (see lang::hash).
        let mut acc = crate::lang::hash::hash_seed("MAP") as i64;
        for (k, v) in self.entries() {
            acc = acc
                .wrapping_add(crate::lang::hash::java_string_hash(&k) as i64)
                .wrapping_add(v.java_hash(hash_type));
        }
        acc as u64
    }
}
impl<V: Clone + std::fmt::Debug> IObjType for Standard<V> {
    fn obj_type(&self) -> ObjType {
        ObjType::Map
    }
}
impl<V> IColl<String> for Standard<V>
where
    V: Clone + Default + PartialEq + std::hash::Hash + JavaHash + std::fmt::Debug,
{
    fn start_string(&self) -> &'static str {
        "#{"
    }
    fn end_string(&self) -> &'static str {
        "}"
    }
}
impl<V: Clone> IToMutable for Standard<V> {
    type Mutable = Mutable<V>;
    fn to_mutable(&self) -> Self::Mutable {
        Mutable {
            editable: Cell::new(true),
            trie: self.clone(),
        }
    }
}
#[derive(Debug, Clone)]
pub struct Mutable<V> {
    editable: Cell<bool>,
    trie: Standard<V>,
}
impl<V: Clone> Mutable<V> {
    fn check(&self) {
        assert!(self.editable.get(), "mutable trie used after to_persistent")
    }
    pub fn assoc(&mut self, k: impl Into<String>, v: V) -> &mut Self {
        self.check();
        self.trie = self.trie.assoc_value(k, v);
        self
    }
    pub fn dissoc(&mut self, k: &str) -> &mut Self {
        self.check();
        self.trie = self.trie.dissoc_value(k);
        self
    }
}
impl<V: Clone> std::ops::Deref for Mutable<V> {
    type Target = Standard<V>;
    fn deref(&self) -> &Self::Target {
        self.check();
        &self.trie
    }
}
impl<V> IMutable for Mutable<V> {}
impl<V: Clone> IToPersistent for Mutable<V> {
    type Persistent = Standard<V>;
    fn to_persistent(&mut self) -> Self::Persistent {
        self.check();
        self.editable.set(false);
        self.trie.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::Standard;
    #[test]
    fn updates_empty_and_mutable_preserve_metadata() {
        use crate::lang::protocol::{IEmpty, IMetadata, IToMutable, IToPersistent};
        let trie = Standard::new()
            .assoc_value("cat", 1)
            .with_meta(Some(crate::lang::data::Metadata::document("doc")));
        assert_eq!(
            trie.assoc_value("car", 2).meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(
            trie.dissoc_value("cat").meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
        assert_eq!(trie.empty().meta().map(|m| m.doc().unwrap()), Some("doc"));
        let mut mutable = trie.to_mutable();
        mutable.assoc("car", 2);
        assert_eq!(
            mutable.to_persistent().meta().map(|m| m.doc().unwrap()),
            Some("doc")
        );
    }

    #[test]
    fn shares_prefixes_and_iterates_lexically() {
        let a = Standard::new()
            .assoc_value("car", 1)
            .assoc_value("cat", 2)
            .assoc_value("dog", 3);
        assert_eq!(a.iter().collect::<Vec<_>>(), vec!["car", "cat", "dog"]);
        let b = a.dissoc_value("cat");
        assert_eq!(b.get("cat"), None);
        assert_eq!(a.get("cat"), Some(&2));
        assert_eq!(b.get("car"), Some(&1));
    }

    #[test]
    fn dissoc_prunes_childless_ancestors_bottom_up() {
        // Java dissocHelper: a node is removed once it is non-terminal and
        // childless, and the removal cascades up the spine.
        let trie = Standard::new()
            .assoc_value("cat", 1)
            .assoc_value("cats", 2)
            .assoc_value("car", 3);
        // removing the leaf word prunes the 's' node; "cat" stays terminal
        let trie = trie.dissoc_value("cats");
        assert_eq!(trie.get("cats"), None);
        assert_eq!(trie.get("cat"), Some(&1));
        // removing "car" prunes the 'r' node but keeps the "cat" spine
        let trie = trie.dissoc_value("car");
        assert_eq!(trie.get("car"), None);
        assert_eq!(trie.entries().len(), 1);
        // removing the last word prunes the whole spine back to an empty root
        let trie = trie.dissoc_value("cat");
        assert_eq!(trie.len(), 0);
        assert!(trie.root.children.is_empty());
        assert!(trie.root.value.is_none());
    }

    #[test]
    fn dissoc_prefix_clears_terminal_but_keeps_children() {
        // Java: dissoc of a word that is a prefix of another word only
        // clears the terminal flag/value; the subtree is untouched.
        let trie = Standard::new()
            .assoc_value("cat", 1)
            .assoc_value("cats", 2)
            .dissoc_value("cat");
        assert_eq!(trie.get("cat"), None);
        assert_eq!(trie.get("cats"), Some(&2));
        assert_eq!(trie.len(), 1);
        assert_eq!(trie.iter().collect::<Vec<_>>(), vec!["cats"]);
    }

    #[test]
    fn empty_string_key_sets_the_root_terminal() {
        // Java assoc("") walks zero chars and marks the root node terminal;
        // iteration yields "" first (it is a prefix of every key).
        let trie = Standard::new().assoc_value("", 7).assoc_value("a", 1);
        assert_eq!(trie.get(""), Some(&7));
        assert_eq!(trie.len(), 2);
        assert_eq!(trie.iter().collect::<Vec<_>>(), vec!["", "a"]);
        // dissoc("") clears the root terminal but keeps the children
        let trie = trie.dissoc_value("");
        assert_eq!(trie.get(""), None);
        assert_eq!(trie.get("a"), Some(&1));
        assert_eq!(trie.len(), 1);
    }

    #[test]
    fn dissoc_absent_prefix_or_divergent_key_is_a_noop() {
        let trie = Standard::new().assoc_value("cat", 1);
        for key in ["cow", "ca", "cats", "dog"] {
            let next = trie.dissoc_value(key);
            assert_eq!(next.len(), 1);
            assert_eq!(next.get("cat"), Some(&1));
        }
    }
}
