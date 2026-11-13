//! Emulation of `java.util.Hashtable` key ordering.
//!
//! `java.util.Properties` (Java 8) is a `Hashtable`, and `Properties.store` writes entries
//! in the table's enumeration order. To reproduce files like `server.properties` byte for
//! byte, krol must know that order. It is a pure function of the insertion/removal
//! history and Java's `String.hashCode`.

use crate::java_string_hash;

const LOAD_FACTOR: f32 = 0.75;
const DEFAULT_CAPACITY: usize = 11;
/// Java's `MAX_ARRAY_SIZE`.
const MAX_ARRAY_SIZE: usize = i32::MAX as usize - 8;

/// An insertion-history-faithful model of a `java.util.Hashtable<String, V>`.
///
/// Holds keys and values; [`iter`](Self::iter) yields entries in exactly the order a Java 8
/// `Hashtable` (and therefore `Properties.store`) enumerates them.
///
/// ```
/// use krol_java::JavaHashtable;
///
/// let mut t = JavaHashtable::new();
/// t.insert("a".to_string(), 1);
/// t.insert("b".to_string(), 2);
/// let keys: Vec<_> = t.iter().map(|(k, _)| k.as_str()).collect();
/// assert_eq!(keys, ["b", "a"]); // higher buckets first, as in Java
/// ```
#[derive(Debug, Clone)]
pub struct JavaHashtable<V> {
    /// Buckets; each chain is stored head-first.
    table: Vec<Vec<Entry<V>>>,
    count: usize,
    threshold: usize,
}

#[derive(Debug, Clone)]
struct Entry<V> {
    hash: i32,
    key: String,
    value: V,
}

impl<V> Default for JavaHashtable<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> JavaHashtable<V> {
    /// Creates a table like `new Hashtable()` / `new Properties()` (capacity 11, load
    /// factor 0.75).
    pub fn new() -> Self {
        Self {
            table: (0..DEFAULT_CAPACITY).map(|_| Vec::new()).collect(),
            count: 0,
            threshold: threshold_for(DEFAULT_CAPACITY),
        }
    }

    fn index_for(hash: i32, capacity: usize) -> usize {
        (hash & 0x7fff_ffff) as usize % capacity
    }

    /// Inserts or replaces the value for `key`, like `Hashtable.put`. Replacing keeps the
    /// key's position. Returns the previous value.
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        let hash = java_string_hash(&key);
        let index = Self::index_for(hash, self.table.len());
        if let Some(entry) = self.table[index].iter_mut().find(|e| e.key == key) {
            return Some(std::mem::replace(&mut entry.value, value));
        }
        let mut index = index;
        if self.count >= self.threshold {
            self.rehash();
            index = Self::index_for(hash, self.table.len());
        }
        self.table[index].insert(0, Entry { hash, key, value });
        self.count += 1;
        None
    }

    /// Removes `key`, like `Hashtable.remove`.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        let index = Self::index_for(java_string_hash(key), self.table.len());
        let chain = &mut self.table[index];
        let pos = chain.iter().position(|e| e.key == key)?;
        self.count -= 1;
        Some(chain.remove(pos).value)
    }

    /// The value for `key`.
    pub fn get(&self, key: &str) -> Option<&V> {
        let index = Self::index_for(java_string_hash(key), self.table.len());
        self.table[index]
            .iter()
            .find(|e| e.key == key)
            .map(|e| &e.value)
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Iterates entries in Java's enumeration order: buckets from the highest index down,
    /// each chain from its head.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.table
            .iter()
            .rev()
            .flat_map(|chain| chain.iter().map(|e| (&e.key, &e.value)))
    }

    fn rehash(&mut self) {
        let old_capacity = self.table.len();
        let new_capacity = (old_capacity << 1) + 1;
        let mut new_table: Vec<Vec<Entry<V>>> = (0..new_capacity).map(|_| Vec::new()).collect();
        self.threshold = threshold_for(new_capacity);
        // Java walks old buckets from the top and pushes each entry onto the head of its
        // new chain, which reverses relative order within a bucket.
        for chain in std::mem::take(&mut self.table).into_iter().rev() {
            for entry in chain {
                let index = Self::index_for(entry.hash, new_capacity);
                new_table[index].insert(0, entry);
            }
        }
        self.table = new_table;
    }
}

fn threshold_for(capacity: usize) -> usize {
    // (int) Math.min(capacity * loadFactor, MAX_ARRAY_SIZE + 1)
    let t = capacity as f32 * LOAD_FACTOR;
    (t.min((MAX_ARRAY_SIZE + 1) as f32)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_keeps_position_and_remove_works() {
        let mut t = JavaHashtable::new();
        for k in ["x", "y", "z"] {
            t.insert(k.to_string(), 0);
        }
        let before: Vec<_> = t.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(t.insert("y".into(), 5), Some(0));
        let after: Vec<_> = t.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(before, after);
        assert_eq!(t.remove("y"), Some(5));
        assert_eq!(t.len(), 2);
        assert!(!t.contains_key("y"));
    }
}
