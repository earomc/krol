//! Replays the operations of `tools/ConcurrentVectors.java` on [`JavaConcurrentLongSet`].

use krol_java::{JavaConcurrentLongSet, JavaRandom};

/// One snapshot: size, first key, order hash.
pub type Snapshot = (usize, Option<i64>, i64);

/// `ChunkCoordIntPair.chunkXZ2Int`.
fn key(x: i32, z: i32) -> i64 {
    (x as u32 as i64) | ((z as u32 as i64) << 32)
}

fn snapshot(set: &JavaConcurrentLongSet) -> Snapshot {
    let mut h = 0xcbf2_9ce4_8422_2325_u64 as i64;
    for k in set.iter() {
        h = (h ^ k).wrapping_mul(0x0100_0000_01b3);
    }
    (set.len(), set.first(), h)
}

fn compare(label: &str, got: &[Snapshot], expected: &[Snapshot]) {
    assert_eq!(got.len(), expected.len(), "{label}: snapshot count");
    for (i, (g, e)) in got.iter().zip(expected).enumerate() {
        assert_eq!(g, e, "{label}: snapshot {i}");
    }
}

pub fn check_square(cx: i32, cz: i32, r: i32, expected: &[Snapshot]) {
    let mut set = JavaConcurrentLongSet::new();
    for x in cx - r..=cx + r {
        for z in cz - r..=cz + r {
            set.insert(key(x, z));
        }
    }
    let mut got = vec![snapshot(&set)];
    while !set.is_empty() {
        for _ in 0..100 {
            set.pop_first();
        }
        got.push(snapshot(&set));
    }
    compare(&format!("square ({cx}, {cz}) r {r}"), &got, expected);
}

pub fn check_random(seed: i64, count: usize, cx: i32, cz: i32, range: i32, expected: &[Snapshot]) {
    let mut set = JavaConcurrentLongSet::new();
    let mut r = JavaRandom::new(seed);
    let mut got = Vec::new();
    for i in 0..count {
        let kind = r.next_int_bounded(10);
        if kind < 8 {
            let x = cx + r.next_int_bounded(2 * range + 1) - range;
            let z = cz + r.next_int_bounded(2 * range + 1) - range;
            if kind < 6 {
                set.insert(key(x, z));
            } else {
                set.remove(key(x, z));
            }
        } else {
            set.pop_first();
        }
        if i % 50 == 49 {
            got.push(snapshot(&set));
        }
    }
    compare(&format!("random seed {seed}"), &got, expected);
}
