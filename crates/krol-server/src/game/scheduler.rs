//! Scheduled block ticks (vanilla: `NextTickListEntry` and `WorldServer`'s
//! `pendingTickListEntriesTreeSet` / `HashSet`, `scheduleUpdate`,
//! `scheduleBlockUpdate`, `tickUpdates`, `getPendingBlockUpdates`).
//!
//! Entries run in order of time, then priority, then creation; at most
//! [`MAX_PER_TICK`] per tick. An entry is unique per position and block.

use std::collections::{BTreeMap, HashSet};

use krol_nbt::{Compound, List, Tag};

use super::block_data::BLOCKS;

/// At most this many scheduled ticks run per game tick.
pub const MAX_PER_TICK: usize = 1000;
/// Saved chunks also store the entries this far before their west and north borders
/// (`getPendingBlockUpdates`). Vanilla quirk: the range ends at the chunk's own east and
/// south edge (`start - 2` up to `start + 16`), so the margin is one-sided.
const SAVE_MARGIN: i32 = 2;

type Pos = (i32, i32, i32);

/// Order of the tree set: scheduled time, priority, creation id.
type Key = (i64, i32, u64);

/// The pending scheduled ticks of one dimension.
#[derive(Debug, Default)]
pub struct Scheduler {
    pending: BTreeMap<Key, (Pos, u16)>,
    present: HashSet<(Pos, u16)>,
    next_id: u64,
}

/// The registry name of a block id (`minecraft:` + name), as saved in `TileTicks`.
fn block_name(block: u16) -> Option<String> {
    BLOCKS
        .get(usize::from(block))
        .map(|(name, _, _)| format!("minecraft:{name}"))
}

/// The block id of a registry name (`Block.getBlockFromName`).
fn block_by_name(name: &str) -> Option<u16> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    BLOCKS
        .iter()
        .position(|(n, _, _)| *n == name)
        .map(|k| k as u16)
}

impl Scheduler {
    /// `scheduleBlockUpdate`: adds an entry unless one for the position and block is
    /// pending. Air entries get time 0 (they run at once).
    pub fn schedule(&mut self, pos: Pos, block: u16, delay: i32, priority: i32, now: i64) {
        // Lit and unlit variants count as the same block (`Block.isEqualTo`).
        let key = (pos, super::redstone::associated(block));
        if self.present.contains(&key) {
            return;
        }
        let time = if block == 0 {
            0
        } else {
            i64::from(delay) + now
        };
        let id = self.next_id;
        self.next_id += 1;
        self.present.insert(key);
        self.pending.insert((time, priority, id), (pos, block));
    }

    /// `tickUpdates(false)`: removes and returns the entries due at `now`, in order.
    pub fn take_due(&mut self, now: i64) -> Vec<(Pos, u16)> {
        let mut due = Vec::new();
        while due.len() < MAX_PER_TICK {
            let Some((&key, _)) = self.pending.first_key_value() else {
                break;
            };
            if key.0 > now {
                break;
            }
            if let Some(entry) = self.pending.remove(&key) {
                self.present
                    .remove(&(entry.0, super::redstone::associated(entry.1)));
                due.push(entry);
            }
        }
        due
    }

    /// `getPendingBlockUpdates(chunk, false)` as saved: the entries within the chunk and
    /// [`SAVE_MARGIN`] blocks west and north of it, with their delay relative to `now`. `None` if
    /// there are none (vanilla then writes no `TileTicks`).
    pub fn chunk_ticks(&self, cx: i32, cz: i32, now: i64) -> Option<List> {
        let (x0, z0) = ((cx << 4) - SAVE_MARGIN, (cz << 4) - SAVE_MARGIN);
        let (x1, z1) = (x0 + 16 + SAVE_MARGIN, z0 + 16 + SAVE_MARGIN);
        let mut list = List::new();
        for (&(time, priority, _), &(pos, block)) in &self.pending {
            if pos.0 < x0 || pos.0 >= x1 || pos.2 < z0 || pos.2 >= z1 {
                continue;
            }
            let mut c = Compound::new();
            c.insert("i", Tag::String(block_name(block).unwrap_or_default()));
            c.insert("x", Tag::Int(pos.0));
            c.insert("y", Tag::Int(pos.1));
            c.insert("z", Tag::Int(pos.2));
            c.insert("t", Tag::Int((time - now) as i32));
            c.insert("p", Tag::Int(priority));
            let _ = list.push(Tag::Compound(c));
        }
        (!list.is_empty()).then_some(list)
    }

    /// Schedules a loaded chunk's saved `TileTicks` (block by name, or by numeric id in
    /// old data).
    pub fn load(&mut self, ticks: &List, now: i64) {
        for tag in ticks.iter() {
            let Tag::Compound(c) = tag else { continue };
            let block = match c.get("i") {
                Some(Tag::String(name)) => block_by_name(name),
                Some(t) => t.as_i32().map(|v| v as u16),
                None => None,
            };
            let Some(block) = block else { continue };
            let pos = (c.get_i32("x"), c.get_i32("y"), c.get_i32("z"));
            self.schedule(pos, block, c.get_i32("t"), c.get_i32("p"), now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_in_vanilla_order() {
        let mut s = Scheduler::default();
        s.schedule((0, 0, 0), 77, 20, 0, 100);
        s.schedule((1, 0, 0), 77, 10, 0, 100);
        s.schedule((2, 0, 0), 77, 10, -1, 100);
        // Duplicate position and block: ignored.
        s.schedule((1, 0, 0), 77, 1, 0, 100);
        assert!(s.take_due(109).is_empty());
        assert_eq!(s.take_due(110), vec![((2, 0, 0), 77), ((1, 0, 0), 77)]);
        assert_eq!(s.take_due(200), vec![((0, 0, 0), 77)]);
    }

    #[test]
    fn saves_and_loads_ticks() {
        let mut s = Scheduler::default();
        s.schedule((-1, 4, 3), 77, 15, 0, 1000);
        let list = s.chunk_ticks(0, 0, 1005).expect("within the margin");
        let mut t = Scheduler::default();
        t.load(&list, 2000);
        assert!(t.take_due(2009).is_empty());
        assert_eq!(t.take_due(2010), vec![((-1, 4, 3), 77)]);
        // Not east of the chunk (the margin is one-sided).
        s.schedule((16, 4, 3), 77, 15, 0, 1000);
        assert_eq!(s.chunk_ticks(0, 0, 1005).map(|l| l.len()), Some(1));
        assert!(s.chunk_ticks(5, 5, 1005).is_none());
    }
}
