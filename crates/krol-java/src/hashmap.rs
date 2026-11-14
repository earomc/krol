//! Emulation of Java 8 `java.util.HashMap<String, V>` iteration order.
//!
//! Vanilla stores many collections in `HashMap`s and writes or iterates them in table
//! order (e.g. `ops.json`, `banned-players.json`). This reproduces that order: table
//! sizes are powers of two (16 initially, load factor 0.75), the bucket index is
//! `(h ^ (h >>> 16)) & (n - 1)` with `h = String.hashCode()`, new entries go to the tail
//! of their bucket, and resizing splits each bucket preserving relative order.
//!
//! Known limitation: Java converts buckets with 8+ entries (in tables of 64+) into
//! red-black trees, which iterate differently. That needs pathological hash collisions
//! and is not reproduced.

use crate::java_string_hash;

const DEFAULT_CAPACITY: usize = 16;

/// An insertion-history-faithful model of a Java 8 `HashMap<String, V>`.
///
/// ```
/// use krol_java::JavaHashMap;
///
/// let mut m = JavaHashMap::new();
/// m.insert("b".to_string(), 2);
/// m.insert("a".to_string(), 1);
/// let keys: Vec<_> = m.iter().map(|(k, _)| k.as_str()).collect();
/// assert_eq!(keys, ["a", "b"]); // bucket order, not insertion order
/// ```
#[derive(Debug, Clone)]
pub struct JavaHashMap<V> {
    table: Vec<Vec<(i32, String, V)>>,
    len: usize,
}

impl<V> Default for JavaHashMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

fn spread(h: i32) -> i32 {
    h ^ ((h as u32) >> 16) as i32
}

impl<V> JavaHashMap<V> {
    /// Creates an empty map like `new HashMap<>()`.
    pub fn new() -> Self {
        Self {
            table: Vec::new(),
            len: 0,
        }
    }

    fn index(hash: i32, n: usize) -> usize {
        (hash as u32 as usize) & (n - 1)
    }

    /// Inserts or replaces the value for `key` (`put`). Returns the previous value.
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        if self.table.is_empty() {
            self.table = (0..DEFAULT_CAPACITY).map(|_| Vec::new()).collect();
        }
        let hash = spread(java_string_hash(&key));
        let n = self.table.len();
        let bucket = &mut self.table[Self::index(hash, n)];
        if let Some(entry) = bucket.iter_mut().find(|(h, k, _)| *h == hash && *k == key) {
            return Some(std::mem::replace(&mut entry.2, value));
        }
        bucket.push((hash, key, value));
        self.len += 1;
        if self.len > n * 3 / 4 {
            self.resize();
        }
        None
    }

    fn resize(&mut self) {
        let old_n = self.table.len();
        let new_n = old_n * 2;
        let mut new_table: Vec<Vec<(i32, String, V)>> = (0..new_n).map(|_| Vec::new()).collect();
        for (j, bucket) in std::mem::take(&mut self.table).into_iter().enumerate() {
            // Entries keep their relative order in the "lo" (j) and "hi" (j + old_n) lists.
            for entry in bucket {
                let target = if (entry.0 as u32 as usize) & old_n == 0 {
                    j
                } else {
                    j + old_n
                };
                new_table[target].push(entry);
            }
        }
        self.table = new_table;
    }

    /// Removes `key` (`remove`).
    pub fn remove(&mut self, key: &str) -> Option<V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(java_string_hash(key));
        let n = self.table.len();
        let bucket = &mut self.table[Self::index(hash, n)];
        let pos = bucket.iter().position(|(h, k, _)| *h == hash && k == key)?;
        self.len -= 1;
        Some(bucket.remove(pos).2)
    }

    /// The value for `key`.
    pub fn get(&self, key: &str) -> Option<&V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(java_string_hash(key));
        self.table[Self::index(hash, self.table.len())]
            .iter()
            .find(|(h, k, _)| *h == hash && k == key)
            .map(|(_, _, v)| v)
    }

    /// Mutable access to the value for `key`.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(java_string_hash(key));
        let n = self.table.len();
        self.table[Self::index(hash, n)]
            .iter_mut()
            .find(|(h, k, _)| *h == hash && k == key)
            .map(|(_, _, v)| v)
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Removes all entries; like Java's `clear()`, the table keeps its size.
    pub fn clear(&mut self) {
        for bucket in &mut self.table {
            bucket.clear();
        }
        self.len = 0;
    }

    /// Keeps only entries for which `f` returns `true` (like `removeIf` on the entry set).
    pub fn retain(&mut self, mut f: impl FnMut(&str, &V) -> bool) {
        for bucket in &mut self.table {
            let before = bucket.len();
            bucket.retain(|(_, k, v)| f(k, v));
            self.len -= before - bucket.len();
        }
    }

    /// Iterates entries in Java's iteration order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.table
            .iter()
            .flat_map(|bucket| bucket.iter().map(|(_, k, v)| (k, v)))
    }
}

/// The order in which a Java 8 `HashMap` iterates distinct keys with these
/// `hashCode()`s, inserted in the given order (indices into `hashes`).
///
/// Used for maps keyed by other types than strings, e.g. a chunk's tile entities by
/// `BlockPos`.
///
/// ```
/// use krol_java::java_hash_order;
///
/// // Bucket order (hash & 15), then insertion order within a bucket.
/// assert_eq!(java_hash_order(&[17, 2, 1]), vec![0, 2, 1]);
/// ```
pub fn java_hash_order(hashes: &[i32]) -> Vec<usize> {
    let mut table: Vec<Vec<(i32, usize)>> = (0..DEFAULT_CAPACITY).map(|_| Vec::new()).collect();
    for (k, &h) in hashes.iter().enumerate() {
        let hash = spread(h);
        let n = table.len();
        table[(hash as u32 as usize) & (n - 1)].push((hash, k));
        if k + 1 > n * 3 / 4 {
            let mut grown: Vec<Vec<(i32, usize)>> = (0..n * 2).map(|_| Vec::new()).collect();
            for (j, bucket) in table.into_iter().enumerate() {
                for entry in bucket {
                    let target = if (entry.0 as u32 as usize) & n == 0 {
                        j
                    } else {
                        j + n
                    };
                    grown[target].push(entry);
                }
            }
            table = grown;
        }
    }
    table.into_iter().flatten().map(|(_, k)| k).collect()
}
