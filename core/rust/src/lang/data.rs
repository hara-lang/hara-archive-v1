#[path = "data/adapters.rs"]
pub mod adapters;
#[path = "data/atom.rs"]
pub mod atom;
#[path = "data/cons.rs"]
pub mod cons;
#[path = "data/deque.rs"]
pub mod deque;
#[path = "data/keyword.rs"]
pub mod keyword;
#[path = "data/list.rs"]
pub mod list;
#[path = "data/map.rs"]
pub mod map;
#[path = "data/map_entry.rs"]
pub mod map_entry;
#[path = "data/metadata.rs"]
pub mod metadata;
#[path = "data/ordered_map.rs"]
pub mod ordered_map;
#[path = "data/ordered_set.rs"]
pub mod ordered_set;
#[path = "data/pointer.rs"]
pub mod pointer;
#[path = "data/priority_map.rs"]
pub mod priority_map;
#[path = "data/queue.rs"]
pub mod queue;
#[path = "data/seq.rs"]
pub mod seq;
#[path = "data/set.rs"]
pub mod set;
#[path = "data/sorted_map.rs"]
pub mod sorted_map;
#[path = "data/sorted_set.rs"]
pub mod sorted_set;
#[path = "data/symbol.rs"]
pub mod symbol;
#[path = "data/tagged_literal.rs"]
pub mod tagged_literal;
#[path = "data/trie.rs"]
pub mod trie;
#[path = "data/tuple.rs"]
pub mod tuple;
#[path = "data/vector.rs"]
pub mod vector;
pub use adapters::{AsList, AsMap, AsSet};
pub use atom::{Atom, WatchEntry};
pub use cons::Cons;
pub use deque::Standard as Deque;
pub use keyword::Keyword;
pub use list::{Mutable as MutableList, Standard as List};
pub use map::{Mutable as MutableMap, Standard as Map};
pub use map_entry::MapEntry;
pub use metadata::{Metadata, MetadataValue};
pub use ordered_map::{Mutable as MutableOrderedMap, Standard as OrderedMap};
pub use ordered_set::{Mutable as MutableOrderedSet, Standard as OrderedSet};
pub use pointer::Pointer;
pub use priority_map::Standard as PriorityMap;
pub use queue::{Mutable as MutableQueue, Standard as Queue};
pub use seq::Seq;
pub use set::{Mutable as MutableSet, Standard as Set};
pub use sorted_map::{Mutable as MutableSortedMap, Standard as SortedMap};
pub use sorted_set::{Mutable as MutableSortedSet, Standard as SortedSet};
pub use symbol::Symbol;
pub use tagged_literal::TaggedLiteral;
pub use trie::{Mutable as MutableTrie, Standard as Trie};
pub use tuple::Tuple;

pub use vector::{Mutable as MutableVector, Standard as Vector, SubView as VectorSubView};

#[cfg(test)]
#[path = "data/conformance_tests.rs"]
mod conformance_tests;
