//! Which status effects a potion's damage value gives (vanilla
//! `PotionHelper.getPotionEffects`).
//!
//! Each effect has a requirement over the damage value's bits, written in vanilla's
//! little expression language: terms are bit numbers (`3` is "bit 3 set", `!3` "not
//! set"), signed and summed (`0+6`), optionally multiplied by one digit (`*`) or compared
//! with the number of set bits (`=`, `<`, `>`); `&` needs every side positive (and takes
//! the largest), `|` takes the first positive side. A positive result is the duration
//! level; the amplifier expression picks the level.
//!
//! TODO(M5): `applyIngredient` for brewing stands.

use super::potions::effect_type;

/// The requirement of each effect that potions can give (`potionRequirements`).
const REQUIREMENTS: [(i32, &str); 13] = [
    (REGENERATION, "0 & !1 & !2 & !3 & 0+6"),
    (SPEED, "!0 & 1 & !2 & !3 & 1+6"),
    (FIRE_RESISTANCE, "0 & 1 & !2 & !3 & 0+6"),
    (INSTANT_HEALTH, "0 & !1 & 2 & !3"),
    (POISON, "!0 & !1 & 2 & !3 & 2+6"),
    (WEAKNESS, "!0 & !1 & !2 & 3 & 3+6"),
    (INSTANT_DAMAGE, "!0 & !1 & 2 & 3"),
    (SLOWNESS, "!0 & 1 & !2 & 3 & 3+6"),
    (STRENGTH, "0 & !1 & !2 & 3 & 3+6"),
    (NIGHT_VISION, "!0 & 1 & 2 & !3 & 2+6"),
    (INVISIBILITY, "!0 & 1 & 2 & 3 & 2+6"),
    (WATER_BREATHING, "0 & !1 & 2 & 3 & 2+6"),
    (JUMP_BOOST, "0 & 1 & !2 & 3 & 3+6"),
];

/// The effects whose level depends on bit 5 (`potionAmplifiers`).
const AMPLIFIED: [i32; 9] = [
    SPEED,
    HASTE,
    STRENGTH,
    REGENERATION,
    INSTANT_DAMAGE,
    INSTANT_HEALTH,
    RESISTANCE,
    POISON,
    JUMP_BOOST,
];
const AMPLIFIER: &str = "5";

/// Effect ids.
const SPEED: i32 = 1;
const SLOWNESS: i32 = 2;
const HASTE: i32 = 3;
const STRENGTH: i32 = 5;
const INSTANT_HEALTH: i32 = 6;
const INSTANT_DAMAGE: i32 = 7;
const JUMP_BOOST: i32 = 8;
const REGENERATION: i32 = 10;
const RESISTANCE: i32 = 11;
const FIRE_RESISTANCE: i32 = 12;
const WATER_BREATHING: i32 = 13;
const INVISIBILITY: i32 = 14;
const NIGHT_VISION: i32 = 16;
const WEAKNESS: i32 = 18;
const POISON: i32 = 19;
/// Effect ids run up to this one.
const LAST_EFFECT: i32 = 31;

/// The damage bit of splash potions (shorter effects).
pub const SPLASH: i32 = 16384;
/// One duration level lasts this many ticks (`1200 * (level * 3 + (level - 1) * 2)`).
const LEVEL_TICKS: i32 = 1200;

/// A potion effect: `(effect id, duration in ticks, amplifier)`.
pub type PotionEffect = (i32, i32, i32);

fn bit(damage: i32, n: i32) -> bool {
    damage & 1i32.wrapping_shl(n as u32) != 0
}

/// One term of an expression.
#[derive(Default)]
struct Term {
    negated: bool,
    has_factor: bool,
    factor_next: bool,
    minus: bool,
    compare: Option<u8>,
    number: i32,
    factor: i32,
    has_number: bool,
}

impl Term {
    /// The term's value for `damage` (`func_77904_a`).
    fn value(&self, damage: i32) -> i32 {
        let mut v = if self.negated {
            i32::from(!bit(damage, self.number))
        } else if let Some(c) = self.compare {
            let set = damage.count_ones() as i32;
            let hit = match c {
                b'=' => set == self.number,
                b'>' => set > self.number,
                _ => set < self.number,
            };
            i32::from(hit)
        } else {
            i32::from(bit(damage, self.number))
        };
        if self.has_factor {
            v *= self.factor;
        }
        if self.minus {
            v = -v;
        }
        v
    }
}

/// `parsePotionEffects` over `s[start..end]`.
fn evaluate(s: &[u8], start: usize, end: usize, damage: i32) -> i32 {
    if start >= s.len() || start >= end {
        return 0;
    }
    let find = |c: u8| s[start..].iter().position(|&b| b == c).map(|k| k + start);
    if let Some(k) = find(b'|').filter(|&k| k < end) {
        // The character before the operator is left out (a space in vanilla's strings).
        let left = evaluate(s, start, k.saturating_sub(1), damage);
        if left > 0 {
            return left;
        }
        return evaluate(s, k + 1, end, damage).max(0);
    }
    if let Some(k) = find(b'&').filter(|&k| k < end) {
        let left = evaluate(s, start, k.saturating_sub(1), damage);
        if left <= 0 {
            return 0;
        }
        let right = evaluate(s, k + 1, end, damage);
        if right <= 0 {
            return 0;
        }
        return left.max(right);
    }
    let mut total = 0;
    let mut t = Term::default();
    let flush = |t: &mut Term, total: &mut i32| {
        if t.has_number {
            *total += t.value(damage);
            *t = Term::default();
        }
    };
    for &c in &s[start..end] {
        match c {
            b'0'..=b'9' => {
                let d = i32::from(c - b'0');
                if t.factor_next {
                    t.factor = d;
                    t.has_factor = true;
                } else {
                    t.number = t.number * 10 + d;
                    t.has_number = true;
                }
            }
            b'*' => t.factor_next = true,
            b'!' => {
                flush(&mut t, &mut total);
                t.negated = true;
            }
            b'-' => {
                flush(&mut t, &mut total);
                t.minus = true;
            }
            b'=' | b'<' | b'>' => {
                flush(&mut t, &mut total);
                t.compare = Some(c);
            }
            b'+' => flush(&mut t, &mut total),
            _ => {}
        }
    }
    flush(&mut t, &mut total);
    total
}

/// `Math.round(double)` cast to `int`.
fn java_round(v: f64) -> i32 {
    (v + 0.5).floor() as i64 as i32
}

/// `PotionHelper.getPotionEffects(damage, false)`: the effects a potion with this damage
/// value gives, in effect id order.
pub fn potion_effects(damage: i32) -> Vec<PotionEffect> {
    let mut out = Vec::new();
    for id in 1..=LAST_EFFECT {
        let Some(kind) = effect_type(id) else {
            continue;
        };
        if kind.usable {
            continue;
        }
        let Some((_, requirement)) = REQUIREMENTS.iter().find(|(e, _)| *e == id) else {
            continue;
        };
        let level = evaluate(requirement.as_bytes(), 0, requirement.len(), damage);
        if level <= 0 {
            continue;
        }
        let amplifier = if AMPLIFIED.contains(&id) {
            evaluate(AMPLIFIER.as_bytes(), 0, AMPLIFIER.len(), damage).max(0)
        } else {
            0
        };
        let duration = if kind.instant {
            1
        } else {
            let mut d = LEVEL_TICKS * (level * 3 + (level - 1) * 2);
            d >>= amplifier;
            d = java_round(f64::from(d) * kind.effectiveness);
            if damage & SPLASH != 0 {
                d = java_round(f64::from(d) * 0.75 + 0.5);
            }
            d
        };
        out.push((id, duration, amplifier));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values from `PotionHelper.getPotionEffects` (tools/datagen/PotionEffectsDump.java).
    #[test]
    fn vanilla_vectors() {
        let cases: &[(i32, &[PotionEffect])] = &[
            (0, &[]),
            (1, &[(10, 900, 0)]),
            (2, &[(1, 3600, 0)]),
            (3, &[(12, 3600, 0)]),
            (4, &[(19, 900, 0)]),
            (5, &[(6, 1, 0)]),
            (16, &[]),
            (8193, &[(10, 900, 0)]),
            (8194, &[(1, 3600, 0)]),
            (8225, &[(10, 450, 1)]),
            (16385, &[(10, 676, 0)]),
        ];
        for (damage, expected) in cases {
            assert_eq!(
                potion_effects(*damage),
                expected.to_vec(),
                "damage {damage}"
            );
        }
    }
}
