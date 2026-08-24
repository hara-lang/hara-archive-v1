use super::ihash::{HashType, IHash};
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.ihashcached",
    name = "IHashCached",
    parents = ["IHash"]
)]
pub trait IHashCached: IHash {
    #[hara_method(value = "hash-current", arity = 1)]
    fn hash_current(&self) -> u64;
    #[hara_method(value = "hash-put", arity = 2)]
    fn hash_put(&self, hash: u64);

    fn hash_cached(&self) -> u64 {
        let current = self.hash_current();
        if current != 0 {
            return current;
        }
        let hash = self.hash();
        self.hash_put(hash);
        hash
    }

    fn hash_cached_as(&self, hash_type: HashType) -> u64 {
        if hash_type == HashType::Rapid {
            self.hash_cached()
        } else {
            self.hash_calc(hash_type)
        }
    }
}
