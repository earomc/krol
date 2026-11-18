//! Entity selectors in commands (`@p`, `@a`, `@r`, `@e` with arguments; vanilla:
//! `PlayerSelector`) and the target lookups built on them (`CommandBase.getPlayer`,
//! `getEntity`), plus Java's lenient `UUID.fromString`.
//!
//! TODO(M5): entities in other dimensions, custom entity names (`CustomName`).

use std::collections::HashMap;

use super::Game;
use super::commands::Sender;

/// Every name `EntityList` knows (`isStringValidEntityName`), plus `Player`.
const ENTITY_NAMES: &[&str] = &[
    "Player",
    "Item",
    "XPOrb",
    "ThrownEgg",
    "LeashKnot",
    "Painting",
    "Arrow",
    "Snowball",
    "Fireball",
    "SmallFireball",
    "ThrownEnderpearl",
    "EyeOfEnderSignal",
    "ThrownPotion",
    "ThrownExpBottle",
    "ItemFrame",
    "WitherSkull",
    "PrimedTnt",
    "FallingSand",
    "FireworksRocketEntity",
    "ArmorStand",
    "Boat",
    "MinecartRideable",
    "MinecartChest",
    "MinecartFurnace",
    "MinecartTNT",
    "MinecartHopper",
    "MinecartSpawner",
    "MinecartCommandBlock",
    "Mob",
    "Monster",
    "Creeper",
    "Skeleton",
    "Spider",
    "Giant",
    "Zombie",
    "Slime",
    "Ghast",
    "PigZombie",
    "Enderman",
    "CaveSpider",
    "Silverfish",
    "Blaze",
    "LavaSlime",
    "EnderDragon",
    "WitherBoss",
    "Bat",
    "Witch",
    "Endermite",
    "Guardian",
    "Pig",
    "Sheep",
    "Cow",
    "Chicken",
    "Squid",
    "Wolf",
    "MushroomCow",
    "SnowMan",
    "Ozelot",
    "VillagerGolem",
    "EntityHorse",
    "Rabbit",
    "Villager",
    "EnderCrystal",
];

/// Arguments that bind the search to the sender's world.
const WORLD_BINDING_ARGS: [&str; 8] = ["x", "y", "z", "dx", "dy", "dz", "rm", "r"];

/// `WorldSettings.GameType.NOT_SET`.
const GAME_TYPE_NOT_SET: i32 = -1;

/// A selector target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A player, by index into `Game::players`.
    Player(usize),
    /// Another entity, by entity id.
    Entity(i32),
}

/// A parsed selector token.
#[derive(Debug, Clone)]
pub struct Selector {
    kind: char,
    args: HashMap<String, String>,
}

/// `\w`.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Parses `^@([pare])(?:\[([\w=,!-]*)\])?$`.
pub fn parse(token: &str) -> Option<Selector> {
    let rest = token.strip_prefix('@')?;
    let mut chars = rest.chars();
    let kind = chars
        .next()
        .filter(|c| matches!(c, 'p' | 'a' | 'r' | 'e'))?;
    let rest = chars.as_str();
    let args = if rest.is_empty() {
        None
    } else {
        let inner = rest.strip_prefix('[')?.strip_suffix(']')?;
        if !inner
            .chars()
            .all(|c| is_word(c) || matches!(c, '=' | ',' | '!' | '-'))
        {
            return None;
        }
        Some(inner)
    };
    Some(Selector {
        kind,
        args: argument_map(args),
    })
}

/// Whether `token` is a selector (`PlayerSelector.hasArguments`).
pub fn is_selector(token: &str) -> bool {
    parse(token).is_some()
}

/// `PlayerSelector.matchesMultiplePlayers`: a selector whose count is not 1.
pub fn matches_multiple(token: &str) -> bool {
    parse(token).is_some_and(|s| s.count() != 1)
}

/// Matches `[-!]?[\w-]*` at the start of `s`, returning its length.
fn value_len(s: &str) -> usize {
    let mut n = 0;
    let bytes = s.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'!')) {
        n = 1;
    }
    n + s[n..]
        .chars()
        .take_while(|&c| is_word(c) || c == '-')
        .count()
}

/// `PlayerSelector.getArgumentMap`: up to four leading positional values (x, y, z, r),
/// then `key=value` pairs.
fn argument_map(args: Option<&str>) -> HashMap<String, String> {
    const POSITIONAL: [&str; 4] = ["x", "y", "z", "r"];
    let mut map = HashMap::new();
    let Some(s) = args else {
        return map;
    };
    // \G([-!]?[\w-]*)(?:$|,), found repeatedly where the previous match ended; an
    // empty match ends the search (the next one would have to start further on).
    let mut pos = 0;
    let mut index = 0;
    let mut last_end = None;
    while let Some((len, consumed)) = longest_positional(&s[pos..]) {
        let value = &s[pos..pos + len];
        if let Some(key) = POSITIONAL.get(index)
            && !value.is_empty()
        {
            map.insert((*key).to_owned(), value.to_owned());
        }
        index += 1;
        pos += consumed;
        last_end = Some(pos);
        if consumed == 0 {
            break;
        }
    }
    if last_end.is_none_or(|e| e < s.len()) {
        // \G(\w+)=([-!]?[\w-]*)(?:$|,)
        let mut rest = &s[last_end.unwrap_or(0)..];
        loop {
            let key_len = rest.chars().take_while(|&c| is_word(c)).count();
            if key_len == 0 || rest.as_bytes().get(key_len) != Some(&b'=') {
                break;
            }
            let after = &rest[key_len + 1..];
            let Some((len, consumed)) = longest_positional(after) else {
                break;
            };
            map.insert(rest[..key_len].to_owned(), after[..len].to_owned());
            rest = &after[consumed..];
        }
    }
    map
}

/// `([-!]?[\w-]*)(?:$|,)` at the start of `s`, with backtracking: the value length and
/// how much was consumed (including the comma).
fn longest_positional(s: &str) -> Option<(usize, usize)> {
    let max = value_len(s);
    // The greedy value either ends the string or is followed by a comma; a shorter
    // value cannot be followed by either (it would be followed by a value character).
    if max == s.len() {
        return Some((max, max));
    }
    if s.as_bytes()[max] == b',' {
        return Some((max, max + 1));
    }
    // A lone sign followed by neither: backtrack past the optional sign.
    None
}

/// `MathHelper.parseIntWithDefault`.
fn int_or(s: &str, default: i32) -> i32 {
    crate::config::parse_java_int(s).unwrap_or(default)
}

impl Selector {
    fn int(&self, key: &str, default: i32) -> i32 {
        self.args.get(key).map_or(default, |v| int_or(v, default))
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.args.get(key).map(String::as_str)
    }

    /// The `c` argument (1 for `@p` and `@r`, all for `@a` and `@e`).
    fn count(&self) -> i32 {
        let default = i32::from(!matches!(self.kind, 'a' | 'e'));
        self.int("c", default)
    }

    /// The `type` argument without its `!`, and whether it is negated.
    fn entity_type(&self) -> Option<(&str, bool)> {
        self.get("type")
            .map(|t| t.strip_prefix('!').map_or((t, false), |t| (t, true)))
    }
}

/// What a selector looks at in one entity.
struct Candidate {
    target: Target,
    pos: (f64, f64, f64),
    /// Width and height of the bounding box.
    size: (f32, f32),
    /// The bounding box of entities whose box is not centered on their position
    /// (paintings and item frames).
    bb: Option<super::physics::Aabb>,
    alive: bool,
    type_name: &'static str,
    name: Option<String>,
    rotation: (f32, f32),
    entity_id: i32,
}

/// `PlayerSelector.func_179650_a`: an angle in degrees into vanilla's odd range.
fn wrap_angle(a: i32) -> i32 {
    let mut a = a % 360;
    // Vanilla quirk: 160, not 180.
    if a >= 160 {
        a -= 360;
    }
    if a < 0 {
        a += 360;
    }
    a
}

fn in_range(angle: i32, min: i32, max: i32) -> bool {
    if min > max {
        angle >= min || angle <= max
    } else {
        angle >= min && angle <= max
    }
}

impl Game {
    /// `ICommandSender.getPosition`: the console's is the origin, a player's the block
    /// at his feet (half a block up).
    pub(super) fn sender_position(&self, sender: Sender) -> (i32, i32, i32) {
        match sender {
            Sender::Console => (0, 0, 0),
            Sender::Player(i) => {
                let p = self.players[i].pos;
                (
                    krol_java::math::floor_f64(p.0),
                    krol_java::math::floor_f64(p.1 + 0.5),
                    krol_java::math::floor_f64(p.2),
                )
            }
        }
    }

    /// `Entity.getName` of a non-player entity, as far as krol knows it.
    pub(super) fn entity_name(&self, id: i32) -> String {
        self.selector_candidates()
            .into_iter()
            .find(|c| c.target == Target::Entity(id))
            .and_then(|c| c.name)
            .unwrap_or_default()
    }

    /// The UUID of a target (`Entity.getUniqueID`).
    pub(super) fn target_uuid(&self, target: Target) -> u128 {
        match target {
            Target::Player(i) => self.players[i].profile.id,
            Target::Entity(id) => self.entity_uuids.get(&id).copied().unwrap_or_default(),
        }
    }

    /// `MinecraftServer.getEntityFromUuid`, then `getPlayerByUUID`.
    pub(super) fn entity_by_uuid(&self, uuid: u128) -> Option<Target> {
        self.selector_candidates()
            .into_iter()
            .map(|c| c.target)
            .find(|&t| self.target_uuid(t) == uuid)
    }

    /// `Entity.getDisplayName` of a non-player entity: its name with a `show_entity`
    /// hover (UUID, type and name as NBT text) and its UUID as the insertion.
    pub(super) fn entity_display_name(&self, id: i32) -> crate::chat::Component {
        use krol_nbt::{Compound, Tag};
        let Some(c) = self
            .selector_candidates()
            .into_iter()
            .find(|c| c.target == Target::Entity(id))
        else {
            return crate::chat::Component::text("");
        };
        let name = c.name.unwrap_or_default();
        let uuid = crate::status::java_uuid_string(self.target_uuid(c.target));
        let mut hover = Compound::new();
        hover.insert("id", Tag::String(uuid.clone()));
        hover.insert("type", Tag::String(c.type_name.to_owned()));
        hover.insert("name", Tag::String(name.clone()));
        let hover = krol_nbt::text::to_java_string(&Tag::Compound(hover));
        let mut text = crate::chat::Component::text(name);
        text.style.hover = Some((
            "show_entity".into(),
            Box::new(crate::chat::Component::text(hover)),
        ));
        text.style.insertion = Some(uuid);
        text
    }

    /// A target's position and rotation (yaw, pitch).
    pub(super) fn target_location(&self, target: Target) -> ((f64, f64, f64), (f32, f32)) {
        match target {
            Target::Player(i) => (self.players[i].pos, self.players[i].rotation),
            Target::Entity(_) => self
                .selector_candidates()
                .into_iter()
                .find(|c| c.target == target)
                .map_or(((0.0, 0.0, 0.0), (0.0, 0.0)), |c| (c.pos, c.rotation)),
        }
    }

    /// `Entity.getName` of a target.
    pub(super) fn target_name(&self, target: Target) -> String {
        match target {
            Target::Player(i) => self.players[i].profile.name.clone(),
            Target::Entity(id) => self.entity_name(id),
        }
    }

    /// `Entity.setLocationAndAngles` of a non-player entity (paintings and item frames
    /// move by block spaces).
    pub(super) fn set_location_and_angles(
        &mut self,
        id: i32,
        pos: (f64, f64, f64),
        rotation: (f32, f32),
    ) {
        if let Some(it) = self.items.get_mut(&id) {
            it.body.pos = pos;
            it.base.yaw = rotation.0;
            it.base.pitch = rotation.1;
        } else if let Some(o) = self.orbs.get_mut(&id) {
            o.body.pos = pos;
        } else if let Some(f) = self.falling.get_mut(&id) {
            f.body.pos = pos;
        } else if let Some(t) = self.tnt.get_mut(&id) {
            t.body.pos = pos;
        } else if let Some(t) = self.thrown.get_mut(&id) {
            t.body.pos = pos;
            t.base.yaw = rotation.0;
            t.base.pitch = rotation.1;
        } else if let Some(a) = self.arrows.get_mut(&id) {
            a.body.pos = pos;
            a.base.yaw = rotation.0;
            a.base.pitch = rotation.1;
        } else if let Some(h) = self.hanging.get_mut(&id) {
            if let super::hanging::Kind::Painting { .. } = h.kind {
                // EntityPainting.setLocationAndAngles: the hanging position moves by the
                // offset, and the position becomes that block's corner.
                let (p, t) = (h.body.pos, h.tile);
                let tile = (
                    krol_java::math::floor_f64(f64::from(t.0) + (pos.0 - p.0)),
                    krol_java::math::floor_f64(f64::from(t.1) + (pos.1 - p.1)),
                    krol_java::math::floor_f64(f64::from(t.2) + (pos.2 - p.2)),
                );
                let corner = (f64::from(tile.0), f64::from(tile.1), f64::from(tile.2));
                self.move_hanging(id, corner);
            } else {
                h.base.yaw = rotation.0;
                h.base.pitch = rotation.1;
                self.move_hanging(id, pos);
            }
        } else if let Some(s) = self.armor_stands.get_mut(&id) {
            s.body.pos = pos;
            s.base.yaw = rotation.0;
            s.base.pitch = rotation.1;
        }
    }

    /// `Entity.setDead` of a non-player entity: it leaves the world at its next turn in
    /// the entity loop.
    pub(super) fn set_entity_dead(&mut self, id: i32) {
        if let Some(it) = self.items.get_mut(&id) {
            it.dead = true;
        } else if let Some(o) = self.orbs.get_mut(&id) {
            o.dead = true;
        } else if let Some(f) = self.falling.get_mut(&id) {
            f.dead = true;
        } else if let Some(t) = self.tnt.get_mut(&id) {
            t.dead = true;
        } else if let Some(t) = self.thrown.get_mut(&id) {
            t.dead = true;
        } else if let Some(a) = self.arrows.get_mut(&id) {
            a.dead = true;
        } else if let Some(h) = self.hanging.get_mut(&id) {
            h.dead = true;
        } else if let Some(s) = self.armor_stands.get_mut(&id) {
            s.dead = true;
        }
    }

    /// Every entity in `loadedEntityList` order (entity ids, like the update loop).
    fn selector_candidates(&self) -> Vec<Candidate> {
        let mut all = Vec::new();
        // A dead player's entity leaves the world after its death animation.
        for (i, p) in self.players.iter().enumerate().filter(|(_, p)| !p.removed) {
            all.push(Candidate {
                target: Target::Player(i),
                pos: p.pos,
                size: (0.6, 1.8),
                bb: None,
                alive: p.health() > 0.0,
                type_name: "Player",
                name: Some(p.profile.name.clone()),
                rotation: p.rotation,
                entity_id: p.entity_id,
            });
        }
        for (&id, it) in &self.items {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: it.body.pos,
                size: (it.body.width, it.body.height),
                bb: None,
                alive: !it.dead,
                type_name: "Item",
                name: Some(super::item_data::item_entity_name(&it.stack)),
                rotation: (it.base.yaw, it.base.pitch),
                entity_id: id,
            });
        }
        for (&id, o) in &self.orbs {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: o.body.pos,
                size: (o.body.width, o.body.height),
                bb: None,
                alive: !o.dead,
                type_name: "XPOrb",
                name: Some("Experience Orb".into()),
                rotation: (0.0, 0.0),
                entity_id: id,
            });
        }
        for (&id, f) in &self.falling {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: f.body.pos,
                size: (f.body.width, f.body.height),
                bb: None,
                alive: !f.dead,
                type_name: "FallingSand",
                name: Some("Falling Block".into()),
                rotation: (0.0, 0.0),
                entity_id: id,
            });
        }
        for (&id, t) in &self.tnt {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: t.body.pos,
                size: (t.body.width, t.body.height),
                bb: None,
                alive: !t.dead,
                type_name: "PrimedTnt",
                name: Some("Block of TNT".into()),
                rotation: (0.0, 0.0),
                entity_id: id,
            });
        }
        for (&id, t) in &self.thrown {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: t.body.pos,
                size: (t.body.width, t.body.height),
                bb: None,
                alive: !t.dead,
                type_name: t.kind.saved_id(),
                name: Some(t.kind.name()),
                rotation: (t.base.yaw, t.base.pitch),
                entity_id: id,
            });
        }
        for (&id, a) in &self.arrows {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: a.body.pos,
                size: (a.body.width, a.body.height),
                bb: None,
                alive: !a.dead,
                type_name: super::arrow::SAVED_ID,
                name: Some(super::arrow::NAME.into()),
                rotation: (a.base.yaw, a.base.pitch),
                entity_id: id,
            });
        }
        for (&id, h) in &self.hanging {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: h.body.pos,
                size: (h.body.width, h.body.height),
                bb: Some(h.bb),
                alive: !h.dead,
                type_name: h.kind.saved_id(),
                name: Some(h.kind.name().into()),
                rotation: (h.base.yaw, h.base.pitch),
                entity_id: id,
            });
        }
        for (&id, s) in &self.armor_stands {
            all.push(Candidate {
                target: Target::Entity(id),
                pos: s.body.pos,
                size: (s.body.width, s.body.height),
                bb: None,
                alive: s.alive(),
                type_name: super::armor_stand::SAVED_ID,
                name: Some(s.name()),
                rotation: (s.base.yaw, s.base.pitch),
                entity_id: id,
            });
        }
        all.sort_by_key(|c| c.entity_id);
        all
    }

    /// `PlayerSelector.matchEntities`; `players_only` stands for the `EntityPlayerMP`
    /// class filter of `matchOnePlayer`. A non-selector or a sender without permission
    /// level 1 matches nothing; an unknown `type` tells the sender and matches nothing.
    pub(super) fn match_entities(
        &self,
        sender: Sender,
        token: &str,
        players_only: bool,
    ) -> Vec<Target> {
        let Some(sel) = parse(token) else {
            return Vec::new();
        };
        let permitted = match sender {
            Sender::Console => true,
            Sender::Player(i) => self.op_level(i).is_some_and(|l| l >= 1),
        };
        if !permitted {
            return Vec::new();
        }
        if let Some((t, _)) = sel.entity_type()
            && !ENTITY_NAMES.contains(&t)
        {
            let message = crate::chat::Component::translate(
                "commands.generic.entity.invalidType",
                vec![crate::chat::Arg::Str(t.to_owned())],
            )
            .color("red");
            self.reply(sender, &message);
            return Vec::new();
        }
        let base = self.sender_position(sender);
        let pos = (
            sel.int("x", base.0),
            sel.int("y", base.1),
            sel.int("z", base.2),
        );
        // Only the overworld holds entities in krol, so binding to the sender's world
        // and searching every world give the same.
        let _bound = WORLD_BINDING_ARGS.iter().any(|k| sel.args.contains_key(*k));
        let all = self.selector_candidates();
        let filters = |c: &Candidate| self.selector_filters(&sel, c, pos);
        let kind = sel.kind;
        let typed_random = kind == 'r' && sel.entity_type().is_some();
        let not_e = kind != 'e';
        let players_count = self.players.len();
        let few_players = players_count < all.len() / 16;
        let class_ok = |c: &Candidate| !players_only || matches!(c.target, Target::Player(_));
        let player_list = || -> Vec<&Candidate> {
            // world.playerEntities, in join order.
            let mut v: Vec<&Candidate> = all
                .iter()
                .filter(|c| matches!(c.target, Target::Player(_)))
                .collect();
            v.sort_by_key(|c| match c.target {
                Target::Player(i) => i,
                Target::Entity(_) => usize::MAX,
            });
            v
        };
        let dx = sel.int("dx", 0);
        let dy = sel.int("dy", 0);
        let dz = sel.int("dz", 0);
        let r = sel.int("r", -1);
        let mut found: Vec<&Candidate> = if sel.args.contains_key("dx")
            || sel.args.contains_key("dy")
            || sel.args.contains_key("dz")
        {
            let bb = volume(pos, dx, dy, dz);
            if not_e && few_players && !typed_random {
                player_list()
                    .into_iter()
                    .filter(|c| {
                        c.alive
                            && filters(c)
                            && c.pos.0 >= bb.0
                            && c.pos.1 >= bb.1
                            && c.pos.2 >= bb.2
                            && c.pos.0 < bb.3
                            && c.pos.1 < bb.4
                            && c.pos.2 < bb.5
                    })
                    .collect()
            } else {
                within_box(&all, bb)
                    .into_iter()
                    .filter(|c| class_ok(c) && c.alive && filters(c))
                    .collect()
            }
        } else if r >= 0 {
            let bb = (
                f64::from(pos.0 - r),
                f64::from(pos.1 - r),
                f64::from(pos.2 - r),
                f64::from(pos.0 + r + 1),
                f64::from(pos.1 + r + 1),
                f64::from(pos.2 + r + 1),
            );
            if not_e && few_players && !typed_random {
                player_list()
                    .into_iter()
                    .filter(|c| c.alive && filters(c))
                    .collect()
            } else {
                within_box(&all, bb)
                    .into_iter()
                    .filter(|c| class_ok(c) && c.alive && filters(c))
                    .collect()
            }
        } else if kind == 'a' {
            // Vanilla quirk: @a has no liveness check, so dead players count.
            player_list().into_iter().filter(|c| filters(c)).collect()
        } else if kind != 'p' && (kind != 'r' || typed_random) {
            all.iter()
                .filter(|c| class_ok(c) && c.alive && filters(c))
                .collect()
        } else {
            player_list()
                .into_iter()
                .filter(|c| c.alive && filters(c))
                .collect()
        };
        let count = sel.count();
        if matches!(kind, 'p' | 'a' | 'e') {
            let corner = (f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
            let dist = |c: &Candidate| {
                let d = (corner.0 - c.pos.0, corner.1 - c.pos.1, corner.2 - c.pos.2);
                d.0 * d.0 + d.1 * d.1 + d.2 * d.2
            };
            // A stable sort, like Collections.sort.
            found.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
        } else if kind == 'r' {
            // Collections.shuffle with its own unseeded random.
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64);
            let mut rand = krol_java::JavaRandom::new(seed);
            for k in (1..found.len()).rev() {
                let j = rand.next_int_bounded(k as i32 + 1) as usize;
                found.swap(k, j);
            }
        }
        let mut targets: Vec<Target> = found.iter().map(|c| c.target).collect();
        if let Sender::Player(i) = sender
            && count == 1
            && targets.contains(&Target::Player(i))
            && kind != 'r'
        {
            targets = vec![Target::Player(i)];
        }
        if count != 0 {
            if count < 0 {
                targets.reverse();
            }
            targets.truncate(count.unsigned_abs() as usize);
        }
        targets
    }

    /// The argument predicates (`func_179663_a` to `func_179662_g`).
    fn selector_filters(&self, sel: &Selector, c: &Candidate, pos: (i32, i32, i32)) -> bool {
        let player = match c.target {
            Target::Player(i) => Some(&self.players[i]),
            Target::Entity(_) => None,
        };
        // Type: players only unless @e, or a type is given for @e / @r.
        let typed = sel.entity_type();
        let typed_random = sel.kind == 'r' && typed.is_some();
        if (typed.is_none() || sel.kind != 'e') && !typed_random {
            if sel.kind != 'e' && player.is_none() {
                return false;
            }
        } else if let Some((t, negated)) = typed
            && (c.type_name == t) == negated
        {
            return false;
        }
        // Experience levels.
        let lm = sel.int("lm", -1);
        let l = sel.int("l", -1);
        if lm > -1 || l > -1 {
            let Some(p) = player else { return false };
            let level = p.data.get_i32("XpLevel");
            if (lm > -1 && level < lm) || (l > -1 && level > l) {
                return false;
            }
        }
        // Game mode.
        let m = sel.int("m", GAME_TYPE_NOT_SET);
        if m != GAME_TYPE_NOT_SET && player.is_none_or(|p| p.gamemode != m) {
            return false;
        }
        // Team: living entities only (players are the only ones krol has).
        if let Some(team) = sel.get("team") {
            let (team, negated) = team.strip_prefix('!').map_or((team, false), |t| (t, true));
            let Target::Player(i) = c.target else {
                return false;
            };
            let own = self.team_of_player(i).unwrap_or("");
            if (own == team) == negated {
                return false;
            }
        }
        // Scores: score_<objective> is a maximum, score_<objective>_min a minimum.
        for (key, value) in &sel.args {
            let Some(objective) = key.strip_prefix("score_").filter(|o| !o.is_empty()) else {
                continue;
            };
            let (objective, min) = match objective.strip_suffix("_min") {
                Some(o) if objective.len() > "_min".len() => (o, true),
                _ => (objective, false),
            };
            let bound = int_or(value, 1);
            if self.scoreboard.objectives.get(objective).is_none() {
                return false;
            }
            // Players hold scores under their name, other entities under their UUID.
            let name = match c.target {
                Target::Player(_) => c.name.clone().unwrap_or_default(),
                Target::Entity(_) => crate::status::java_uuid_string(self.target_uuid(c.target)),
            };
            let Some(score) = self.scoreboard.score(&name, objective) else {
                return false;
            };
            if (min && score.points < bound) || (!min && score.points > bound) {
                return false;
            }
        }
        // Name.
        if let Some(name) = sel.get("name") {
            let (name, negated) = name.strip_prefix('!').map_or((name, false), |n| (n, true));
            if (c.name.as_deref() == Some(name)) == negated {
                return false;
            }
        }
        // Distance from the block's center, truncated.
        let rm = sel.int("rm", -1);
        let r = sel.int("r", -1);
        if rm >= 0 || r >= 0 {
            let d = (
                c.pos.0 - (f64::from(pos.0) + 0.5),
                c.pos.1 - (f64::from(pos.1) + 0.5),
                c.pos.2 - (f64::from(pos.2) + 0.5),
            );
            let dist = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2) as i32;
            if (rm >= 0 && dist < rm.wrapping_mul(rm)) || (r >= 0 && dist > r.wrapping_mul(r)) {
                return false;
            }
        }
        // Rotation.
        if sel.args.contains_key("rym") || sel.args.contains_key("ry") {
            let min = wrap_angle(sel.int("rym", 0));
            let max = wrap_angle(sel.int("ry", 359));
            let yaw = wrap_angle(f64::from(c.rotation.0).floor() as i32);
            if !in_range(yaw, min, max) {
                return false;
            }
        }
        if sel.args.contains_key("rxm") || sel.args.contains_key("rx") {
            let min = wrap_angle(sel.int("rxm", 0));
            let max = wrap_angle(sel.int("rx", 359));
            let pitch = wrap_angle(f64::from(c.rotation.1).floor() as i32);
            if !in_range(pitch, min, max) {
                return false;
            }
        }
        true
    }
}

/// `func_179661_a`: the box from the position spanning `d` blocks per axis (negative
/// extents go the other way), one block more on the far side.
fn volume(pos: (i32, i32, i32), dx: i32, dy: i32, dz: i32) -> (f64, f64, f64, f64, f64, f64) {
    let lo = |p: i32, d: i32| p + if d < 0 { d } else { 0 };
    let hi = |p: i32, d: i32| p + if d < 0 { 0 } else { d } + 1;
    (
        f64::from(lo(pos.0, dx)),
        f64::from(lo(pos.1, dy)),
        f64::from(lo(pos.2, dz)),
        f64::from(hi(pos.0, dx)),
        f64::from(hi(pos.1, dy)),
        f64::from(hi(pos.2, dz)),
    )
}

/// `World.getEntitiesWithinAABB`: entities whose bounding box intersects the box, by
/// chunk (x, then z) and section.
///
/// TODO(M5): within a chunk section vanilla keeps the order entities entered it; krol
/// uses entity id order.
fn within_box(all: &[Candidate], bb: (f64, f64, f64, f64, f64, f64)) -> Vec<&Candidate> {
    let mut v: Vec<&Candidate> = all
        .iter()
        .filter(|c| {
            let half = f64::from(c.size.0) / 2.0;
            let ((x0, y0, z0), (x1, y1, z1)) = match c.bb {
                Some(b) => (
                    (b.min[0], b.min[1], b.min[2]),
                    (b.max[0], b.max[1], b.max[2]),
                ),
                None => (
                    (c.pos.0 - half, c.pos.1, c.pos.2 - half),
                    (
                        c.pos.0 + half,
                        c.pos.1 + f64::from(c.size.1),
                        c.pos.2 + half,
                    ),
                ),
            };
            x1 > bb.0 && x0 < bb.3 && y1 > bb.1 && y0 < bb.4 && z1 > bb.2 && z0 < bb.5
        })
        .collect();
    let key = |c: &Candidate| {
        let section = krol_java::math::floor_f64(c.pos.1 / 16.0).clamp(0, 15);
        (
            krol_java::math::floor_f64(c.pos.0) >> 4,
            krol_java::math::floor_f64(c.pos.2) >> 4,
            section,
        )
    };
    v.sort_by_key(|c| key(c));
    v
}

/// Java's `UUID.fromString`: five dash-separated hex numbers of any length (each fitting
/// a positive `long`), combined with shifts that drop excess bits.
pub fn java_uuid_from_string(s: &str) -> Option<u128> {
    let parts: Vec<&str> = s.split('-').collect();
    // String.split drops trailing empty strings.
    let mut n = parts.len();
    while n > 0 && parts[n - 1].is_empty() {
        n -= 1;
    }
    if n != 5 {
        return None;
    }
    let mut v = [0i64; 5];
    for (k, p) in parts[..5].iter().enumerate() {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let x = u64::from_str_radix(p, 16).ok()?;
        v[k] = i64::try_from(x).ok()?;
    }
    let mut most = v[0];
    most = most.wrapping_shl(16) | v[1];
    most = most.wrapping_shl(16) | v[2];
    let least = v[3].wrapping_shl(48) | v[4];
    Some(u128::from(most as u64) << 64 | u128::from(least as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<(String, String)> {
        let mut v: Vec<_> = parse(s).unwrap().args.into_iter().collect();
        v.sort();
        v
    }

    #[test]
    fn parsing() {
        assert!(parse("@p").is_some());
        assert!(parse("@x").is_none());
        assert!(parse("@a[").is_none());
        assert!(parse("@a[r=3 ]").is_none());
        assert_eq!(
            args("@a[1,2,3,4,m=1]"),
            [("m", "1"), ("r", "4"), ("x", "1"), ("y", "2"), ("z", "3")]
                .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
        assert_eq!(
            args("@e[type=!Item,name=jeb_]"),
            [("name", "jeb_"), ("type", "!Item")].map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
        assert_eq!(
            args("@a[,5]"),
            [("y", "5")].map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
        assert!(matches_multiple("@a"));
        assert!(!matches_multiple("@p"));
        assert!(matches_multiple("@p[c=2]"));
        assert!(!matches_multiple("@a[c=1]"));
    }

    #[test]
    fn uuids() {
        assert_eq!(
            java_uuid_from_string("1-2-3-4-5"),
            Some(0x0000_0001_0002_0003_0004_0000_0000_0005)
        );
        assert_eq!(java_uuid_from_string("1-2-3-4"), None);
        assert_eq!(java_uuid_from_string("1-2-3-4-g"), None);
        assert_eq!(
            java_uuid_from_string("1-2-3-4-5-"),
            Some(0x0000_0001_0002_0003_0004_0000_0000_0005)
        );
    }

    #[test]
    fn angles() {
        assert_eq!(wrap_angle(170), 170);
        assert_eq!(wrap_angle(-10), 350);
        assert_eq!(wrap_angle(359), 359);
    }
}
