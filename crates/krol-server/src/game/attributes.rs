//! Player attributes (vanilla `SharedMonsterAttributes`, `ModifiableAttributeInstance`,
//! `AttributeModifier`): base values plus modifiers from sprinting and status effects.

use krol_nbt::{Compound, List, Tag, TagType};

/// `generic.maxHealth`.
pub const MAX_HEALTH: &str = "generic.maxHealth";
/// `generic.knockbackResistance`.
pub const KNOCKBACK_RESISTANCE: &str = "generic.knockbackResistance";
/// `generic.movementSpeed`.
pub const MOVEMENT_SPEED: &str = "generic.movementSpeed";
/// `generic.attackDamage`.
pub const ATTACK_DAMAGE: &str = "generic.attackDamage";

/// A player's attributes with their base values, in the order vanilla saves them.
pub const PLAYER_ATTRIBUTES: [(&str, f64); 4] = [
    (MAX_HEALTH, 20.0),
    (KNOCKBACK_RESISTANCE, 0.0),
    (MOVEMENT_SPEED, 0.100_000_001_490_116_12),
    (ATTACK_DAMAGE, 1.0),
];

/// The attributes clients are told about (`getShouldWatch`).
pub fn watched(attribute: &str) -> bool {
    attribute == MAX_HEALTH || attribute == MOVEMENT_SPEED
}

/// `EntityLivingBase.sprintingSpeedBoostModifier` (not saved).
pub const SPRINT_UUID: u128 = 0x662a6b8d_da3e_4c1c_8813_96ea6097278d;
/// The sprinting speed boost (`0.30000001192092896`, operation 2).
pub const SPRINT_AMOUNT: f64 = 0.300_000_011_920_928_96;

/// One attribute modifier (`AttributeModifier`).
#[derive(Debug, Clone, PartialEq)]
pub struct Modifier {
    /// The attribute it changes.
    pub attribute: &'static str,
    /// Its id.
    pub uuid: u128,
    /// Its name (saved, not sent).
    pub name: String,
    /// Its amount.
    pub amount: f64,
    /// 0: add, 1: add a multiple of the base, 2: multiply the total.
    pub operation: i8,
    /// Written to player data (`isSaved`).
    pub saved: bool,
}

/// `UUID.hashCode`.
fn uuid_hash(uuid: u128) -> i32 {
    let hilo = (uuid >> 64) as u64 ^ uuid as u64;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

/// An attribute's modifiers in `func_111122_c` order (a `HashSet` of modifiers, which
/// hash like their UUIDs).
pub fn modifiers_of<'a>(attribute: &str, modifiers: &'a [Modifier]) -> Vec<&'a Modifier> {
    let ms: Vec<&Modifier> = modifiers
        .iter()
        .filter(|m| m.attribute == attribute)
        .collect();
    let hashes: Vec<i32> = ms.iter().map(|m| uuid_hash(m.uuid)).collect();
    krol_java::java_hash_order(&hashes)
        .into_iter()
        .map(|k| ms[k])
        .collect()
}

/// The base value of a player attribute.
pub fn base(attribute: &str) -> f64 {
    PLAYER_ATTRIBUTES
        .iter()
        .find(|(a, _)| *a == attribute)
        .map_or(0.0, |(_, b)| *b)
}

fn attribute_key(name: &str) -> Option<&'static str> {
    PLAYER_ATTRIBUTES
        .iter()
        .map(|(a, _)| *a)
        .find(|a| *a == name)
}

/// `ModifiableAttributeInstance.computeValue` (clamped like the ranged attributes:
/// max health to 1..1024, speed to 0..1024).
pub fn value(attribute: &str, modifiers: &[Modifier]) -> f64 {
    let ms: Vec<&Modifier> = modifiers
        .iter()
        .filter(|m| m.attribute == attribute)
        .collect();
    let mut v = base(attribute);
    for m in ms.iter().filter(|m| m.operation == 0) {
        v += m.amount;
    }
    let mut total = v;
    for m in ms.iter().filter(|m| m.operation == 1) {
        total += v * m.amount;
    }
    for m in ms.iter().filter(|m| m.operation == 2) {
        total *= 1.0 + m.amount;
    }
    let (lo, hi) = match attribute {
        MAX_HEALTH => (0.0, 1024.0),
        KNOCKBACK_RESISTANCE => (0.0, 1.0),
        MOVEMENT_SPEED => (0.0, 1024.0),
        _ => (0.0, 2048.0),
    };
    total.clamp(lo, hi)
}

/// The `(key, base, modifiers)` entries of an Entity Properties packet.
pub fn properties(
    attributes: &[&'static str],
    modifiers: &[Modifier],
) -> Vec<krol_protocol::packets::play::Property> {
    attributes
        .iter()
        .map(|a| {
            let ms = modifiers_of(a, modifiers)
                .into_iter()
                .map(|m| (m.uuid, m.amount, m.operation))
                .collect();
            ((*a).to_owned(), base(a), ms)
        })
        .collect()
}

/// Reads the saved modifiers of `Attributes`.
pub fn read_modifiers(data: &Compound) -> Vec<Modifier> {
    let mut out = Vec::new();
    let Some(list) = data.get_list("Attributes", TagType::Compound) else {
        return out;
    };
    for a in list.iter() {
        let Tag::Compound(a) = a else { continue };
        let Some(attribute) = attribute_key(a.get_str("Name")) else {
            continue;
        };
        let Some(ms) = a.get_list("Modifiers", TagType::Compound) else {
            continue;
        };
        for m in ms.iter() {
            let Tag::Compound(m) = m else { continue };
            let uuid = (m.get_i64("UUIDMost") as u64 as u128) << 64
                | m.get_i64("UUIDLeast") as u64 as u128;
            out.push(Modifier {
                attribute,
                uuid,
                name: m.get_str("Name").to_owned(),
                amount: m.get_f64("Amount"),
                operation: m.get_i32("Operation") as i8,
                saved: true,
            });
        }
    }
    out
}

/// Writes `Attributes` (`SharedMonsterAttributes.writeBaseAttributeMapToNBT`) with the
/// saved modifiers.
pub fn write_attributes(data: &mut Compound, modifiers: &[Modifier]) {
    let mut list = List::new();
    for (name, base) in PLAYER_ATTRIBUTES {
        let mut a = Compound::new();
        a.insert("Base", Tag::Double(base));
        // Vanilla writes the list whenever there are modifiers, with the saved ones.
        let all = modifiers_of(name, modifiers);
        if !all.is_empty() {
            let mut ms = List::new();
            for m in all.into_iter().filter(|m| m.saved) {
                let mut c = Compound::new();
                c.insert("Name", Tag::String(m.name.clone()));
                c.insert("Amount", Tag::Double(m.amount));
                c.insert("Operation", Tag::Int(i32::from(m.operation)));
                c.insert("UUIDMost", Tag::Long((m.uuid >> 64) as i64));
                c.insert("UUIDLeast", Tag::Long(m.uuid as i64));
                let _ = ms.push(Tag::Compound(c));
            }
            a.insert("Modifiers", Tag::List(ms));
        }
        a.insert("Name", Tag::String(name.into()));
        let _ = list.push(Tag::Compound(a));
    }
    data.insert("Attributes", Tag::List(list));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_like_vanilla() {
        let sprint = Modifier {
            attribute: MOVEMENT_SPEED,
            uuid: SPRINT_UUID,
            name: "Sprinting speed boost".into(),
            amount: SPRINT_AMOUNT,
            operation: 2,
            saved: false,
        };
        let v = value(MOVEMENT_SPEED, &[sprint]);
        assert!((v - 0.13).abs() < 1e-6);
        assert_eq!(value(MAX_HEALTH, &[]), 20.0);
    }
}
