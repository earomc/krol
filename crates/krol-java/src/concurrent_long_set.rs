//! Emulation of the iteration order of a Java 8 `Set<Long>` backed by a
//! `java.util.concurrent.ConcurrentHashMap` (`Collections.newSetFromMap(new
//! ConcurrentHashMap<>())`).
//!
//! Vanilla queues chunks to unload in such a set (`ChunkProviderServer.droppedChunksSet`)
//! and unloads at most 100 per tick, always taking the set's first element. Which tick a
//! chunk unloads in is observable (its entities stop aging), so krol needs the exact
//! order. It depends on the whole history of additions and removals:
//!
//! - The table is created on the first addition with 16 bins and doubles whenever the
//!   size reaches 3/4 of it. A key's bin is `spread(h) & (n - 1)` with
//!   `h = Long.hashCode(key)` and `spread(h) = (h ^ (h >>> 16)) & 0x7fffffff`.
//! - Plain bins are lists: new keys go to the tail. Resizing splits a list into the two
//!   bins `i` and `i + n`: the trailing run of keys going to the same half keeps its
//!   order, the keys before it are prepended one by one (reversing them).
//! - A list that already held 8 keys when a key was added (or found at position 8 or
//!   later) becomes a red-black tree if the table has 64 bins or more. A smaller table
//!   grows instead, and Java 8's `tryPresize` grows it up to 8 times at once.
//! - A tree bin iterates in its own linked order, where new keys are prepended. Removing
//!   a key while the tree is small (no right child or left grandchild at the root) turns
//!   the bin back into a list. Resizing splits a tree in order; a half of 6 keys or fewer
//!   becomes a list, a larger one a new tree built in order (or keeps the old tree when
//!   all keys stay together).
//!
//! The trees order keys by spread hash, then by value (`Long.compareTo`), and balance
//! like the textbook red-black tree. Verified against Java 8 by
//! `tests/concurrent_vectors.rs` (`tools/ConcurrentVectors.java`).

/// Java's `ConcurrentHashMap.TREEIFY_THRESHOLD`.
const TREEIFY_THRESHOLD: usize = 8;
/// Java's `ConcurrentHashMap.UNTREEIFY_THRESHOLD`.
const UNTREEIFY_THRESHOLD: usize = 6;
/// Java's `ConcurrentHashMap.MIN_TREEIFY_CAPACITY`.
const MIN_TREEIFY_CAPACITY: usize = 64;
/// Java's `ConcurrentHashMap.DEFAULT_CAPACITY`.
const DEFAULT_CAPACITY: usize = 16;

/// `Long.hashCode` followed by `ConcurrentHashMap.spread`.
fn spread(key: i64) -> i32 {
    let h = (key ^ ((key as u64) >> 32) as i64) as i32;
    (h ^ ((h as u32) >> 16) as i32) & 0x7fff_ffff
}

/// The order keys take in a tree bin.
fn tree_key(key: i64) -> (i32, i64) {
    (spread(key), key)
}

/// A history-faithful model of Java 8 `Collections.newSetFromMap(new
/// ConcurrentHashMap<Long, Boolean>())`.
///
/// ```
/// use krol_java::JavaConcurrentLongSet;
///
/// let mut set = JavaConcurrentLongSet::new();
/// set.insert(17);
/// set.insert(1);
/// set.insert(-1);
/// // -1 hashes to 0 (bin 0); 17 and 1 share bin 1, in the order they were added.
/// assert_eq!(set.iter().collect::<Vec<_>>(), [-1, 17, 1]);
/// assert_eq!(set.first(), Some(-1));
/// ```
#[derive(Debug, Clone, Default)]
pub struct JavaConcurrentLongSet {
    table: Vec<Bin>,
    /// `sizeCtl` once the table exists: the size that triggers the next resize.
    size_ctl: usize,
    len: usize,
}

#[derive(Debug, Clone, Default)]
enum Bin {
    #[default]
    Empty,
    List(Vec<i64>),
    Tree(TreeBin),
}

impl JavaConcurrentLongSet {
    /// An empty set (`new ConcurrentHashMap<>()`: no table until the first addition).
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn index(&self, key: i64) -> usize {
        spread(key) as usize & (self.table.len() - 1)
    }

    /// Whether `key` is in the set.
    pub fn contains(&self, key: i64) -> bool {
        if self.table.is_empty() {
            return false;
        }
        match &self.table[self.index(key)] {
            Bin::Empty => false,
            Bin::List(keys) => keys.contains(&key),
            Bin::Tree(t) => t.order.contains(&key),
        }
    }

    /// Adds `key` (`Set.add`, i.e. `put(key, TRUE)`); returns whether it was new.
    pub fn insert(&mut self, key: i64) -> bool {
        if self.table.is_empty() {
            self.table = vec![Bin::Empty; DEFAULT_CAPACITY];
            self.size_ctl = DEFAULT_CAPACITY - (DEFAULT_CAPACITY >> 2);
        }
        let i = self.index(key);
        // How many nodes `putVal` walked (its `binCount`): it decides on treeifying.
        let (added, walked) = match &mut self.table[i] {
            bin @ Bin::Empty => {
                *bin = Bin::List(vec![key]);
                (true, 0)
            }
            Bin::List(keys) => match keys.iter().position(|&k| k == key) {
                Some(at) => (false, at + 1),
                None => {
                    keys.push(key);
                    (true, keys.len() - 1)
                }
            },
            // putVal counts 2 for a tree bin, below the threshold.
            Bin::Tree(t) => (t.insert(key), 2),
        };
        if walked >= TREEIFY_THRESHOLD {
            self.treeify_bin(i);
        }
        if added {
            self.len += 1;
            // addCount: resize while the size has reached sizeCtl.
            while self.len >= self.size_ctl {
                self.transfer();
            }
        }
        added
    }

    /// Removes `key`; returns whether it was present.
    pub fn remove(&mut self, key: i64) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let i = self.index(key);
        let removed = match &mut self.table[i] {
            Bin::Empty => false,
            Bin::List(keys) => match keys.iter().position(|&k| k == key) {
                Some(at) => {
                    keys.remove(at);
                    if keys.is_empty() {
                        self.table[i] = Bin::Empty;
                    }
                    true
                }
                None => false,
            },
            Bin::Tree(t) => match t.remove(key) {
                None => false,
                Some(false) => true,
                Some(true) => {
                    // removeTreeNode reported a small tree: the bin becomes a list.
                    let order = std::mem::take(&mut t.order);
                    self.table[i] = if order.is_empty() {
                        Bin::Empty
                    } else {
                        Bin::List(order)
                    };
                    true
                }
            },
        };
        if removed {
            self.len -= 1;
        }
        removed
    }

    /// The first key in iteration order.
    pub fn first(&self) -> Option<i64> {
        self.iter().next()
    }

    /// Removes and returns the first key in iteration order.
    pub fn pop_first(&mut self) -> Option<i64> {
        let key = self.first()?;
        self.remove(key);
        Some(key)
    }

    /// The keys in Java's iteration order.
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        self.table.iter().flat_map(|bin| {
            let keys: &[i64] = match bin {
                Bin::Empty => &[],
                Bin::List(keys) => keys,
                Bin::Tree(t) => &t.order,
            };
            keys.iter().copied()
        })
    }

    /// `treeifyBin`: a long list becomes a tree, or a small table grows.
    fn treeify_bin(&mut self, i: usize) {
        let n = self.table.len();
        if n < MIN_TREEIFY_CAPACITY {
            self.try_presize(n << 1);
        } else if let Bin::List(keys) = &self.table[i] {
            self.table[i] = Bin::Tree(TreeBin::from_order(keys.clone()));
        }
    }

    /// `tryPresize(size)`: resizes until `sizeCtl` reaches the capacity wanted for
    /// `size`, which in Java 8 overshoots (from 16 bins, asking for 32 gives 128).
    fn try_presize(&mut self, size: usize) {
        let wanted = (size + (size >> 1) + 1).next_power_of_two();
        while wanted > self.size_ctl {
            self.transfer();
        }
    }

    /// `transfer`: doubles the table, splitting every bin.
    fn transfer(&mut self) {
        let n = self.table.len();
        let mut next = vec![Bin::Empty; n * 2];
        for (i, bin) in std::mem::take(&mut self.table).into_iter().enumerate() {
            let (lo, hi) = match bin {
                Bin::Empty => continue,
                Bin::List(keys) => split_list(keys, n),
                Bin::Tree(t) => t.split(n),
            };
            next[i] = lo;
            next[i + n] = hi;
        }
        self.table = next;
        self.size_ctl = (n << 1) - (n >> 1);
    }
}

/// Splits a list bin for a table growing from `n` bins.
fn split_list(keys: Vec<i64>, n: usize) -> (Bin, Bin) {
    let bit = |k: i64| spread(k) as usize & n;
    let mut last_run = 0;
    let mut run_bit = bit(keys[0]);
    for (at, &k) in keys.iter().enumerate().skip(1) {
        if bit(k) != run_bit {
            run_bit = bit(k);
            last_run = at;
        }
    }
    let mut before = keys;
    let run = before.split_off(last_run);
    let (mut lo, mut hi) = if run_bit == 0 {
        (run, Vec::new())
    } else {
        (Vec::new(), run)
    };
    for k in before {
        let half = if bit(k) == 0 { &mut lo } else { &mut hi };
        half.insert(0, k);
    }
    (list_bin(lo), list_bin(hi))
}

fn list_bin(keys: Vec<i64>) -> Bin {
    if keys.is_empty() {
        Bin::Empty
    } else {
        Bin::List(keys)
    }
}

/// A tree bin: the iteration order plus the red-black tree whose shape decides when the
/// bin turns back into a list.
#[derive(Debug, Clone)]
struct TreeBin {
    order: Vec<i64>,
    tree: RbTree,
}

impl TreeBin {
    /// `new TreeBin(list)`: the tree built by inserting the keys in order.
    fn from_order(order: Vec<i64>) -> Self {
        let mut tree = RbTree::default();
        for &k in &order {
            tree.insert(k);
        }
        Self { order, tree }
    }

    /// `putTreeVal`: a new key is prepended to the order.
    fn insert(&mut self, key: i64) -> bool {
        if !self.tree.insert(key) {
            return false;
        }
        self.order.insert(0, key);
        true
    }

    /// `removeTreeNode`: `None` if absent, otherwise whether the bin should become a
    /// list (the tree is then left as it was).
    fn remove(&mut self, key: i64) -> Option<bool> {
        let node = self.tree.find(key)?;
        self.order.retain(|&k| k != key);
        if self.order.is_empty() || self.tree.is_small() {
            return Some(true);
        }
        self.tree.delete(node);
        Some(false)
    }

    /// Splits for a table growing from `n` bins.
    fn split(self, n: usize) -> (Bin, Bin) {
        let (lo, hi): (Vec<i64>, Vec<i64>) = self
            .order
            .iter()
            .partition(|&&k| spread(k) as usize & n == 0);
        // A half of 6 keys or fewer becomes a list; a larger one a new tree, unless all
        // keys stay together and the old tree is reused.
        let lo_empty = lo.is_empty();
        let lo_bin = if lo.len() <= UNTREEIFY_THRESHOLD {
            list_bin(lo)
        } else if hi.is_empty() {
            Bin::Tree(self.clone())
        } else {
            Bin::Tree(TreeBin::from_order(lo))
        };
        let hi_bin = if hi.len() <= UNTREEIFY_THRESHOLD {
            list_bin(hi)
        } else if !lo_empty {
            Bin::Tree(TreeBin::from_order(hi))
        } else {
            Bin::Tree(self)
        };
        (lo_bin, hi_bin)
    }
}

#[derive(Debug, Clone)]
struct Node {
    key: i64,
    left: Option<usize>,
    right: Option<usize>,
    parent: Option<usize>,
    red: bool,
}

/// An arena red-black tree ordered by [`tree_key`].
#[derive(Debug, Clone, Default)]
struct RbTree {
    nodes: Vec<Node>,
    root: Option<usize>,
}

impl RbTree {
    fn find(&self, key: i64) -> Option<usize> {
        let mut p = self.root;
        while let Some(i) = p {
            let n = &self.nodes[i];
            match tree_key(key).cmp(&tree_key(n.key)) {
                std::cmp::Ordering::Less => p = n.left,
                std::cmp::Ordering::Greater => p = n.right,
                std::cmp::Ordering::Equal => return Some(i),
            }
        }
        None
    }

    /// The check at the start of `removeTreeNode` that gives up on a small tree.
    fn is_small(&self) -> bool {
        let Some(r) = self.root else {
            return true;
        };
        let root = &self.nodes[r];
        root.right.is_none() || root.left.is_none_or(|rl| self.nodes[rl].left.is_none())
    }

    fn parent(&self, i: usize) -> Option<usize> {
        self.nodes[i].parent
    }

    fn red(&self, i: Option<usize>) -> bool {
        i.is_some_and(|i| self.nodes[i].red)
    }

    /// Inserts `key`; false if it was present.
    fn insert(&mut self, key: i64) -> bool {
        let x = self.nodes.len();
        let Some(mut p) = self.root else {
            self.nodes.push(Node {
                key,
                left: None,
                right: None,
                parent: None,
                red: false,
            });
            self.root = Some(x);
            return true;
        };
        loop {
            let n = &self.nodes[p];
            let next = match tree_key(key).cmp(&tree_key(n.key)) {
                std::cmp::Ordering::Less => n.left,
                std::cmp::Ordering::Greater => n.right,
                std::cmp::Ordering::Equal => return false,
            };
            match next {
                Some(c) => p = c,
                None => break,
            }
        }
        self.nodes.push(Node {
            key,
            left: None,
            right: None,
            parent: Some(p),
            red: true,
        });
        if tree_key(key) < tree_key(self.nodes[p].key) {
            self.nodes[p].left = Some(x);
        } else {
            self.nodes[p].right = Some(x);
        }
        self.balance_insertion(x);
        true
    }

    /// Makes `r` take `p`'s place under `p`'s parent (or as the root, which turns
    /// black, as Java's rotations do).
    fn replace_in_parent(&mut self, p: usize, r: usize) {
        let pp = self.nodes[p].parent;
        self.nodes[r].parent = pp;
        match pp {
            None => {
                self.root = Some(r);
                self.nodes[r].red = false;
            }
            Some(pp) => {
                if self.nodes[pp].left == Some(p) {
                    self.nodes[pp].left = Some(r);
                } else {
                    self.nodes[pp].right = Some(r);
                }
            }
        }
    }

    fn rotate_left(&mut self, p: usize) {
        let Some(r) = self.nodes[p].right else {
            return;
        };
        let rl = self.nodes[r].left;
        self.nodes[p].right = rl;
        if let Some(rl) = rl {
            self.nodes[rl].parent = Some(p);
        }
        self.replace_in_parent(p, r);
        self.nodes[r].left = Some(p);
        self.nodes[p].parent = Some(r);
    }

    fn rotate_right(&mut self, p: usize) {
        let Some(l) = self.nodes[p].left else {
            return;
        };
        let lr = self.nodes[l].right;
        self.nodes[p].left = lr;
        if let Some(lr) = lr {
            self.nodes[lr].parent = Some(p);
        }
        self.replace_in_parent(p, l);
        self.nodes[l].right = Some(p);
        self.nodes[p].parent = Some(l);
    }

    fn balance_insertion(&mut self, mut x: usize) {
        self.nodes[x].red = true;
        loop {
            let Some(xp) = self.parent(x) else {
                self.nodes[x].red = false;
                return;
            };
            if !self.nodes[xp].red {
                return;
            }
            let Some(xpp) = self.parent(xp) else {
                return;
            };
            let parent_is_left = self.nodes[xpp].left == Some(xp);
            let uncle = if parent_is_left {
                self.nodes[xpp].right
            } else {
                self.nodes[xpp].left
            };
            if self.red(uncle) {
                if let Some(u) = uncle {
                    self.nodes[u].red = false;
                }
                self.nodes[xp].red = false;
                self.nodes[xpp].red = true;
                x = xpp;
                continue;
            }
            let mut xp = xp;
            let inner = if parent_is_left {
                self.nodes[xp].right == Some(x)
            } else {
                self.nodes[xp].left == Some(x)
            };
            if inner {
                x = xp;
                if parent_is_left {
                    self.rotate_left(x);
                } else {
                    self.rotate_right(x);
                }
                xp = self.parent(x).unwrap_or(x);
            }
            self.nodes[xp].red = false;
            if let Some(xpp) = self.parent(xp) {
                self.nodes[xpp].red = true;
                if parent_is_left {
                    self.rotate_right(xpp);
                } else {
                    self.rotate_left(xpp);
                }
            }
        }
    }

    /// Removes node `p`: a node with two children first trades places (and colors)
    /// with its successor; then its child takes its place, or it rebalances while still
    /// in the tree and is unlinked afterwards.
    fn delete(&mut self, p: usize) {
        let (pl, pr) = (self.nodes[p].left, self.nodes[p].right);
        let replacement = if let (Some(pl), Some(pr)) = (pl, pr) {
            let mut s = pr;
            while let Some(sl) = self.nodes[s].left {
                s = sl;
            }
            let colors = (self.nodes[s].red, self.nodes[p].red);
            self.nodes[p].red = colors.0;
            self.nodes[s].red = colors.1;
            let sr = self.nodes[s].right;
            let pp = self.nodes[p].parent;
            if s == pr {
                self.nodes[p].parent = Some(s);
                self.nodes[s].right = Some(p);
            } else {
                let sp = self.nodes[s].parent;
                self.nodes[p].parent = sp;
                if let Some(sp) = sp {
                    if self.nodes[sp].left == Some(s) {
                        self.nodes[sp].left = Some(p);
                    } else {
                        self.nodes[sp].right = Some(p);
                    }
                }
                self.nodes[s].right = Some(pr);
                self.nodes[pr].parent = Some(s);
            }
            self.nodes[p].left = None;
            self.nodes[p].right = sr;
            if let Some(sr) = sr {
                self.nodes[sr].parent = Some(p);
            }
            self.nodes[s].left = Some(pl);
            self.nodes[pl].parent = Some(s);
            self.nodes[s].parent = pp;
            match pp {
                None => self.root = Some(s),
                Some(pp) => {
                    if self.nodes[pp].left == Some(p) {
                        self.nodes[pp].left = Some(s);
                    } else {
                        self.nodes[pp].right = Some(s);
                    }
                }
            }
            sr.unwrap_or(p)
        } else {
            pl.or(pr).unwrap_or(p)
        };
        if replacement != p {
            let pp = self.nodes[p].parent;
            self.nodes[replacement].parent = pp;
            match pp {
                None => self.root = Some(replacement),
                Some(pp) => {
                    if self.nodes[pp].left == Some(p) {
                        self.nodes[pp].left = Some(replacement);
                    } else {
                        self.nodes[pp].right = Some(replacement);
                    }
                }
            }
            self.nodes[p].left = None;
            self.nodes[p].right = None;
            self.nodes[p].parent = None;
        }
        if !self.nodes[p].red {
            self.balance_deletion(self.root, replacement);
        }
        if replacement == p
            && let Some(pp) = self.nodes[p].parent
        {
            if self.nodes[pp].left == Some(p) {
                self.nodes[pp].left = None;
            } else if self.nodes[pp].right == Some(p) {
                self.nodes[pp].right = None;
            }
            self.nodes[p].parent = None;
        }
    }

    /// Rebalances after removing a black node; `root` is the root once the replacement
    /// took the removed node's place, then the root after each rotation. A replacement
    /// that became the root keeps its color, even red (unlike `HashMap`'s trees).
    fn balance_deletion(&mut self, mut root: Option<usize>, mut x: usize) {
        loop {
            if root == Some(x) {
                return;
            }
            let Some(xp) = self.parent(x) else {
                self.nodes[x].red = false;
                return;
            };
            if self.nodes[x].red {
                self.nodes[x].red = false;
                return;
            }
            let x_is_left = self.nodes[xp].left == Some(x);
            let sibling = |t: &Self, xp: usize| {
                if x_is_left {
                    t.nodes[xp].right
                } else {
                    t.nodes[xp].left
                }
            };
            let mut xp = xp;
            let mut s = sibling(self, xp);
            if self.red(s) {
                if let Some(s) = s {
                    self.nodes[s].red = false;
                }
                self.nodes[xp].red = true;
                if x_is_left {
                    self.rotate_left(xp);
                    root = self.root;
                } else {
                    self.rotate_right(xp);
                    root = self.root;
                }
                match self.parent(x) {
                    Some(p) => {
                        xp = p;
                        s = sibling(self, xp);
                    }
                    None => s = None,
                }
            }
            let Some(sib) = s else {
                x = xp;
                continue;
            };
            let (near, far) = if x_is_left {
                (self.nodes[sib].left, self.nodes[sib].right)
            } else {
                (self.nodes[sib].right, self.nodes[sib].left)
            };
            if !self.red(far) && !self.red(near) {
                self.nodes[sib].red = true;
                x = xp;
                continue;
            }
            let mut sib = sib;
            if !self.red(far) {
                if let Some(near) = near {
                    self.nodes[near].red = false;
                }
                self.nodes[sib].red = true;
                if x_is_left {
                    self.rotate_right(sib);
                    root = self.root;
                } else {
                    self.rotate_left(sib);
                    root = self.root;
                }
                xp = self.parent(x).unwrap_or(xp);
                match sibling(self, xp) {
                    Some(s2) => sib = s2,
                    None => {
                        x = root.unwrap_or(x);
                        continue;
                    }
                }
            }
            self.nodes[sib].red = self.nodes[xp].red;
            let far = if x_is_left {
                self.nodes[sib].right
            } else {
                self.nodes[sib].left
            };
            if let Some(far) = far {
                self.nodes[far].red = false;
            }
            self.nodes[xp].red = false;
            if x_is_left {
                self.rotate_left(xp);
            } else {
                self.rotate_right(xp);
            }
            return;
        }
    }
}
