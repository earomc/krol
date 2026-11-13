//! Emulation of Java 8 `java.util.concurrent.ConcurrentHashMap<String, V>` iteration order.
//!
//! Vanilla stores player statistics in Guava's `Maps.newConcurrentMap()`, i.e.
//! `new ConcurrentHashMap(16, 0.75f, 16)`, whose table starts at 32 bins, and writes
//! `stats/<uuid>.json` in its iteration order. New entries go to the tail of their bin;
//! a resize (at 3/4 load) splits each bin with the "lastRun" trick, which reverses the
//! nodes before the last run.
//!
//! Known limitation: bins of 8+ nodes become trees in Java (tables of 64+), which is not
//! reproduced.

use crate::java_string_hash;

/// An insertion-history-faithful model of a Java 8 `ConcurrentHashMap<String, V>`.
///
/// ```
/// use krol_java::JavaConcurrentHashMap;
///
/// let mut m = JavaConcurrentHashMap::with_guava_defaults();
/// m.insert("stat.leaveGame".to_string(), 1);
/// m.insert("stat.playOneMinute".to_string(), 74);
/// let keys: Vec<_> = m.iter().map(|(k, _)| k.as_str()).collect();
/// assert_eq!(keys, ["stat.playOneMinute", "stat.leaveGame"]);
/// ```
#[derive(Debug, Clone)]
pub struct JavaConcurrentHashMap<V> {
    table: Vec<Vec<(i32, String, V)>>,
    len: usize,
}

fn spread(h: i32) -> i32 {
    (h ^ ((h as u32) >> 16) as i32) & 0x7fff_ffff
}

impl<V> JavaConcurrentHashMap<V> {
    /// `new ConcurrentHashMap(initialCapacity, 0.75f, concurrencyLevel)`.
    pub fn new(initial_capacity: usize, concurrency_level: usize) -> Self {
        let initial = initial_capacity.max(concurrency_level);
        let size = (1.0 + initial as f64 / 0.75) as usize;
        let cap = size.next_power_of_two().max(1);
        Self {
            table: (0..cap).map(|_| Vec::new()).collect(),
            len: 0,
        }
    }

    /// The map Guava's `Maps.newConcurrentMap()` creates (32 bins).
    pub fn with_guava_defaults() -> Self {
        Self::new(16, 16)
    }

    fn index(hash: i32, n: usize) -> usize {
        hash as usize & (n - 1)
    }

    /// Inserts or replaces the value for `key` (`put`). Returns the previous value.
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        let hash = spread(java_string_hash(&key));
        let n = self.table.len();
        let bin = &mut self.table[Self::index(hash, n)];
        if let Some(e) = bin.iter_mut().find(|(h, k, _)| *h == hash && *k == key) {
            return Some(std::mem::replace(&mut e.2, value));
        }
        bin.push((hash, key, value));
        self.len += 1;
        // addCount: resize once the count reaches sizeCtl = n - n/4.
        if self.len >= n - (n >> 2) {
            self.resize();
        }
        None
    }

    fn resize(&mut self) {
        let n = self.table.len();
        let mut next: Vec<Vec<(i32, String, V)>> = (0..n * 2).map(|_| Vec::new()).collect();
        for (i, bin) in std::mem::take(&mut self.table).into_iter().enumerate() {
            if bin.is_empty() {
                continue;
            }
            // lastRun: the start of the trailing run of nodes going to the same half.
            let bit = |e: &(i32, String, V)| e.0 as usize & n;
            let mut last_run = 0;
            let mut run_bit = bit(&bin[0]);
            for (k, e) in bin.iter().enumerate().skip(1) {
                let b = bit(e);
                if b != run_bit {
                    run_bit = b;
                    last_run = k;
                }
            }
            let mut nodes = bin;
            let tail = nodes.split_off(last_run);
            let (mut lo, mut hi) = if run_bit == 0 {
                (tail, Vec::new())
            } else {
                (Vec::new(), tail)
            };
            // Nodes before lastRun are prepended one by one (reversing their order).
            for e in nodes {
                if bit(&e) == 0 {
                    lo.insert(0, e);
                } else {
                    hi.insert(0, e);
                }
            }
            next[i] = lo;
            next[i + n] = hi;
        }
        self.table = next;
    }

    /// The value for `key`.
    pub fn get(&self, key: &str) -> Option<&V> {
        let hash = spread(java_string_hash(key));
        self.table[Self::index(hash, self.table.len())]
            .iter()
            .find(|(h, k, _)| *h == hash && k == key)
            .map(|(_, _, v)| v)
    }

    /// Mutable access to the value for `key`.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        let hash = spread(java_string_hash(key));
        let n = self.table.len();
        self.table[Self::index(hash, n)]
            .iter_mut()
            .find(|(h, k, _)| *h == hash && k == key)
            .map(|(_, _, v)| v)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iterates entries in Java's iteration order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.table
            .iter()
            .flat_map(|bin| bin.iter().map(|(_, k, v)| (k, v)))
    }
}
