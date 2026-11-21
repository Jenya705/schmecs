use std::hash::{BuildHasher, Hash, Hasher};

use hashbrown::{HashMap, hash_map::RawEntryMut};

pub struct TraitlessHashMapRawEntryBuilderMut<'a, K, V, S> {
    map: &'a mut HashMap<K, V, S>,
}

impl<'a, K, V, S> TraitlessHashMapRawEntryBuilderMut<'a, K, V, S>
where
    S: BuildHasher,
{
    pub fn new(map: &'a mut HashMap<K, V, S>) -> Self {
        Self { map }
    }

    pub fn build<K1>(
        self,
        key: &K1,
        is_match: impl for<'b> FnMut(&'b K) -> bool,
    ) -> (u64, RawEntryMut<'a, K, V, S>)
    where
        K1: Hash,
    {
        let hash = {
            let mut hasher = self.map.hasher().build_hasher();
            key.hash(&mut hasher);
            hasher.finish()
        };

        let entry = self.map.raw_entry_mut().from_hash(hash, is_match);

        (hash, entry)
    }

    pub fn from_key(self, key: &K) -> (u64, RawEntryMut<'a, K, V, S>)
    where
        K: Hash + PartialEq,
    {
        self.build(key, |other| other.eq(key))
    }
}
