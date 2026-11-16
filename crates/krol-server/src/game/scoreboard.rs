//! The scoreboard (vanilla: `Scoreboard`, `ServerScoreboard`, `ScoreObjective`, `Score`,
//! `ScorePlayerTeam`, `IScoreObjectiveCriteria`, `ScoreboardSaveData`): objectives,
//! scores, display slots and teams, the packets that keep clients in step, and
//! `<world>/data/scoreboard.dat`.
//!
//! Maps vanilla keys by string are Java `HashMap`s, so lists and saved data come out
//! in vanilla's order. A name's scores are a `HashMap` keyed by objective *object*
//! (identity hash codes, different on every run); krol keeps them in the order they
//! were created.
//!
//! TODO(M5): kill criteria (`playerKillCount`, `totalKillCount`, `teamkill.*`,
//! `killedByTeam.*`) need killers, scores and teams of non-player entities need their
//! UUIDs.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use krol_java::JavaHashMap;
use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, TeamInfo};
use tracing::error;

use super::Game;

/// Number of display slots: list, sidebar, below name and one sidebar per team color.
pub const DISPLAY_SLOTS: usize = 19;
/// The first per-team-color sidebar slot.
const TEAM_SIDEBAR_FIRST: usize = 3;
/// Longest objective and team names.
pub const MAX_NAME: usize = 16;
/// Longest objective and team display names.
pub const MAX_DISPLAY: usize = 32;
/// Longest score holder names.
pub const MAX_HOLDER: usize = 40;

/// `EnumChatFormatting` in order: name and code; the first 16 are colors.
pub const FORMATTING: [(&str, char); 22] = [
    ("black", '0'),
    ("dark_blue", '1'),
    ("dark_green", '2'),
    ("dark_aqua", '3'),
    ("dark_red", '4'),
    ("dark_purple", '5'),
    ("gold", '6'),
    ("gray", '7'),
    ("dark_gray", '8'),
    ("blue", '9'),
    ("green", 'a'),
    ("aqua", 'b'),
    ("red", 'c'),
    ("light_purple", 'd'),
    ("yellow", 'e'),
    ("white", 'f'),
    ("obfuscated", 'k'),
    ("bold", 'l'),
    ("strikethrough", 'm'),
    ("underline", 'n'),
    ("italic", 'o'),
    ("reset", 'r'),
];
/// Number of colors among [`FORMATTING`].
const COLORS: usize = 16;
/// `EnumChatFormatting.RESET`.
pub const RESET: usize = 21;
/// The first fancy style (obfuscated to italic come before reset).
const FANCY_FIRST: usize = 16;

/// `EnumChatFormatting.getValueByName`: case and non-letters are ignored.
pub fn formatting_by_name(name: &str) -> Option<usize> {
    let squash = |s: &str| -> String {
        s.chars()
            .flat_map(char::to_lowercase)
            .filter(char::is_ascii_lowercase)
            .collect()
    };
    let wanted = squash(name);
    FORMATTING.iter().position(|(n, _)| squash(n) == wanted)
}

/// Whether formatting `f` is a style rather than a color or reset.
pub fn is_fancy(f: usize) -> bool {
    (FANCY_FIRST..RESET).contains(&f)
}

/// `EnumChatFormatting.toString`: `§` and the code.
pub fn formatting_code(f: usize) -> String {
    format!("\u{a7}{}", FORMATTING[f].1)
}

/// `getValidValues(true, false)`: the colors and reset.
pub fn color_names() -> Vec<&'static str> {
    FORMATTING
        .iter()
        .enumerate()
        .filter(|(k, _)| !is_fancy(*k))
        .map(|(_, (n, _))| *n)
        .collect()
}

/// `Team.EnumVisible` internal names in declaration order.
pub const VISIBILITIES: [&str; 4] = ["always", "never", "hideForOtherTeams", "hideForOwnTeam"];

/// `Team.EnumVisible.func_178825_a`: the names in `HashMap` order.
pub fn visibility_names() -> Vec<String> {
    let mut map = JavaHashMap::new();
    for v in VISIBILITIES {
        map.insert(v.to_owned(), ());
    }
    map.iter().map(|(k, _)| k.clone()).collect()
}

/// `Scoreboard.getObjectiveDisplaySlot`.
pub fn slot_name(slot: usize) -> Option<String> {
    match slot {
        0 => Some("list".into()),
        1 => Some("sidebar".into()),
        2 => Some("belowName".into()),
        s if (TEAM_SIDEBAR_FIRST..DISPLAY_SLOTS).contains(&s) => Some(format!(
            "sidebar.team.{}",
            FORMATTING[s - TEAM_SIDEBAR_FIRST].0
        )),
        _ => None,
    }
}

/// `Scoreboard.getObjectiveDisplaySlotNumber`.
pub fn slot_by_name(name: &str) -> Option<usize> {
    if name.eq_ignore_ascii_case("list") {
        return Some(0);
    }
    if name.eq_ignore_ascii_case("sidebar") {
        return Some(1);
    }
    if name.eq_ignore_ascii_case("belowName") {
        return Some(2);
    }
    let color = name.strip_prefix("sidebar.team.")?;
    formatting_by_name(color)
        .filter(|&f| f < COLORS)
        .map(|f| f + TEAM_SIDEBAR_FIRST)
}

/// The criteria names `IScoreObjectiveCriteria.INSTANCES` holds (extracted by
/// `tools/datagen/CriteriaDump.java` into `data/criteria.txt`).
pub fn criteria_exists(name: &str) -> bool {
    use std::sync::OnceLock;
    static NAMES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    NAMES
        .get_or_init(|| include_str!("../../data/criteria.txt").lines().collect())
        .contains(name)
}

/// `ScoreHealthCriteria`: the only read-only criterion, shown as hearts.
pub const HEALTH: &str = "health";
/// `IScoreObjectiveCriteria.TRIGGER`.
pub const TRIGGER: &str = "trigger";
/// `IScoreObjectiveCriteria.deathCount`.
pub const DEATH_COUNT: &str = "deathCount";

/// `IScoreObjectiveCriteria.isReadOnly`.
pub fn read_only(criteria: &str) -> bool {
    criteria == HEALTH
}

/// `IScoreObjectiveCriteria.getRenderType`.
fn criteria_render(criteria: &str) -> &'static str {
    if criteria == HEALTH {
        "hearts"
    } else {
        "integer"
    }
}

/// `EnumRenderType.func_178795_a`: unknown names are integers.
fn render_by_name(name: &str) -> &'static str {
    if name == "hearts" {
        "hearts"
    } else {
        "integer"
    }
}

/// An objective.
#[derive(Debug, Clone)]
pub struct Objective {
    /// Criteria name.
    pub criteria: String,
    /// Display name.
    pub display: String,
    /// Render type (`integer` or `hearts`).
    pub render: &'static str,
}

/// A score.
#[derive(Debug, Clone)]
pub struct Score {
    /// The value.
    pub points: i32,
    /// Locked (`/trigger` used it).
    pub locked: bool,
    /// Sends the next set even if the value does not change (new scores).
    force_update: bool,
}

/// A team.
#[derive(Debug, Clone)]
pub struct Team {
    /// Display name.
    pub display: String,
    /// Prefix before members' names.
    pub prefix: String,
    /// Suffix after members' names.
    pub suffix: String,
    /// Members can hurt each other.
    pub friendly_fire: bool,
    /// Members see invisible teammates.
    pub see_invisibles: bool,
    /// Name tag visibility (one of [`VISIBILITIES`]).
    pub nametag: &'static str,
    /// Death message visibility (one of [`VISIBILITIES`]).
    pub death_messages: &'static str,
    /// `chatFormat` (index into [`FORMATTING`]).
    pub color: usize,
    /// Members (a `HashSet`).
    pub members: JavaHashMap<()>,
}

impl Team {
    fn new(name: &str) -> Self {
        Self {
            display: name.to_owned(),
            prefix: String::new(),
            suffix: String::new(),
            friendly_fire: true,
            see_invisibles: true,
            nametag: VISIBILITIES[0],
            death_messages: VISIBILITIES[0],
            color: RESET,
            members: JavaHashMap::new(),
        }
    }

    /// `func_98299_i`: the friendly flags byte.
    fn flags(&self) -> u8 {
        u8::from(self.friendly_fire) | u8::from(self.see_invisibles) << 1
    }

    /// `getColorIndex` of the team color (-1 unless a color).
    fn color_index(&self) -> i8 {
        if self.color < COLORS {
            self.color as i8
        } else {
            -1
        }
    }
}

/// The world's scoreboard.
#[derive(Debug, Default)]
pub struct Scoreboard {
    /// Objectives by name.
    pub objectives: JavaHashMap<Objective>,
    /// Objective names per criteria, in creation order.
    by_criteria: HashMap<String, Vec<String>>,
    /// Each holder's scores by objective name.
    pub scores: JavaHashMap<Vec<(String, Score)>>,
    /// The objective shown in each display slot.
    pub slots: [Option<String>; DISPLAY_SLOTS],
    /// Teams by name.
    pub teams: JavaHashMap<Team>,
    /// Each holder's team.
    memberships: HashMap<String, String>,
    /// Objectives clients know about (`ServerScoreboard.field_96553_b`).
    shown: HashSet<String>,
    /// Changed since the last save.
    dirty: bool,
}

impl Scoreboard {
    /// The objectives of a criteria, in creation order.
    pub fn objectives_for(&self, criteria: &str) -> Vec<String> {
        self.by_criteria.get(criteria).cloned().unwrap_or_default()
    }

    /// A holder's score, if it exists.
    pub fn score(&self, holder: &str, objective: &str) -> Option<&Score> {
        self.scores
            .get(holder)?
            .iter()
            .find(|(o, _)| o == objective)
            .map(|(_, s)| s)
    }

    /// `getValueFromObjective`: the score, created at 0 if needed.
    pub fn get_or_create(&mut self, holder: &str, objective: &str) -> &mut Score {
        if self.scores.get(holder).is_none() {
            self.scores.insert(holder.to_owned(), Vec::new());
        }
        let list = self.scores.get_mut(holder).expect("just inserted");
        if let Some(k) = list.iter().position(|(o, _)| o == objective) {
            return &mut list[k].1;
        }
        list.push((
            objective.to_owned(),
            Score {
                points: 0,
                locked: false,
                force_update: true,
            },
        ));
        &mut list.last_mut().expect("just pushed").1
    }

    /// `getPlayersTeam`.
    pub fn team_of(&self, holder: &str) -> Option<&str> {
        self.memberships.get(holder).map(String::as_str)
    }

    /// `getSortedScores`: by value, then name ignoring case.
    pub fn sorted_scores(&self, objective: &str) -> Vec<(String, i32)> {
        let mut v: Vec<(String, i32)> = self
            .scores
            .iter()
            .filter_map(|(h, l)| {
                l.iter()
                    .find(|(o, _)| o == objective)
                    .map(|(_, s)| (h.clone(), s.points))
            })
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| compare_ignore_case(&a.0, &b.0)));
        v
    }

    fn slot_count(&self, objective: &str) -> usize {
        self.slots
            .iter()
            .filter(|s| s.as_deref() == Some(objective))
            .count()
    }
}

/// `String.compareToIgnoreCase`.
pub fn compare_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: char| {
        let u = c.to_uppercase().next().unwrap_or(c);
        u.to_lowercase().next().unwrap_or(u)
    };
    a.chars().map(fold).cmp(b.chars().map(fold))
}

fn objective_packet(name: &str, o: &Objective, mode: u8) -> ClientboundPlay {
    ClientboundPlay::ScoreboardObjective {
        name: name.to_owned(),
        mode,
        display: o.display.clone(),
        // Vanilla quirk: the packet takes the criteria's render type, not the
        // objective's.
        render: criteria_render(&o.criteria).to_owned(),
    }
}

fn team_packet(name: &str, t: &Team, mode: u8) -> ClientboundPlay {
    let info = (mode == 0 || mode == 2).then(|| TeamInfo {
        display: t.display.clone(),
        prefix: t.prefix.clone(),
        suffix: t.suffix.clone(),
        flags: t.flags(),
        nametag: t.nametag.to_owned(),
        color: t.color_index(),
    });
    let players = if mode == 0 {
        t.members.iter().map(|(k, _)| k.clone()).collect()
    } else {
        Vec::new()
    };
    ClientboundPlay::Teams {
        name: name.to_owned(),
        mode,
        info,
        players,
    }
}

fn score_packet(holder: &str, objective: &str, points: i32) -> ClientboundPlay {
    ClientboundPlay::UpdateScore {
        name: holder.to_owned(),
        remove: false,
        objective: objective.to_owned(),
        value: points,
    }
}

impl Game {
    fn send_all(&self, packet: &ClientboundPlay) {
        for p in &self.players {
            p.send(packet.clone());
        }
    }

    /// `addScoreObjective` (the name is valid and free).
    pub(super) fn sb_add_objective(&mut self, name: &str, criteria: &str) {
        let sb = &mut self.scoreboard;
        sb.objectives.insert(
            name.to_owned(),
            Objective {
                criteria: criteria.to_owned(),
                display: name.to_owned(),
                render: criteria_render(criteria),
            },
        );
        sb.by_criteria
            .entry(criteria.to_owned())
            .or_default()
            .push(name.to_owned());
        sb.dirty = true;
    }

    /// `ScoreObjective.setDisplayName` / `setRenderType`.
    pub(super) fn sb_set_objective(&mut self, name: &str, display: String, render: &'static str) {
        let sb = &mut self.scoreboard;
        let Some(o) = sb.objectives.get_mut(name) else {
            return;
        };
        o.display = display;
        o.render = render;
        sb.dirty = true;
        if sb.shown.contains(name) {
            let p = objective_packet(name, &sb.objectives.get(name).expect("exists").clone(), 2);
            self.send_all(&p);
        }
    }

    /// `removeObjective`.
    pub(super) fn sb_remove_objective(&mut self, name: &str) {
        let Some(o) = self.scoreboard.objectives.get(name).cloned() else {
            return;
        };
        for slot in 0..DISPLAY_SLOTS {
            if self.scoreboard.slots[slot].as_deref() == Some(name) {
                self.sb_set_display_slot(slot, None);
            }
        }
        let sb = &mut self.scoreboard;
        sb.objectives.remove(name);
        if let Some(list) = sb.by_criteria.get_mut(&o.criteria) {
            list.retain(|n| n != name);
        }
        let holders: Vec<String> = sb.scores.iter().map(|(h, _)| h.clone()).collect();
        for h in holders {
            if let Some(list) = sb.scores.get_mut(&h) {
                list.retain(|(n, _)| n != name);
            }
        }
        sb.dirty = true;
        if sb.shown.contains(name) {
            self.sb_send_removal(name, &o);
        }
    }

    /// `sendDisplaySlotRemovalPackets`.
    fn sb_send_removal(&mut self, name: &str, o: &Objective) {
        let mut packets = vec![objective_packet(name, o, 1)];
        for slot in 0..DISPLAY_SLOTS {
            if self.scoreboard.slots[slot].as_deref() == Some(name) {
                packets.push(ClientboundPlay::DisplayScoreboard {
                    position: slot as u8,
                    name: name.to_owned(),
                });
            }
        }
        for p in &packets {
            self.send_all(p);
        }
        self.scoreboard.shown.remove(name);
    }

    /// `func_96550_d`: everything a client needs to show an objective.
    fn sb_show_packets(&self, name: &str) -> Vec<ClientboundPlay> {
        let sb = &self.scoreboard;
        let Some(o) = sb.objectives.get(name) else {
            return Vec::new();
        };
        let mut packets = vec![objective_packet(name, o, 0)];
        for slot in 0..DISPLAY_SLOTS {
            if sb.slots[slot].as_deref() == Some(name) {
                packets.push(ClientboundPlay::DisplayScoreboard {
                    position: slot as u8,
                    name: name.to_owned(),
                });
            }
        }
        for (holder, points) in sb.sorted_scores(name) {
            packets.push(score_packet(&holder, name, points));
        }
        packets
    }

    /// `ServerScoreboard.setObjectiveInDisplaySlot`.
    pub(super) fn sb_set_display_slot(&mut self, slot: usize, objective: Option<&str>) {
        let old = self.scoreboard.slots[slot].clone();
        self.scoreboard.slots[slot] = objective.map(str::to_owned);
        if let Some(old) = old.as_deref()
            && Some(old) != objective
        {
            if self.scoreboard.slot_count(old) > 0 {
                self.send_all(&ClientboundPlay::DisplayScoreboard {
                    position: slot as u8,
                    name: objective.unwrap_or_default().to_owned(),
                });
            } else if let Some(o) = self.scoreboard.objectives.get(old).cloned() {
                self.sb_send_removal(old, &o);
            }
        }
        if let Some(new) = objective {
            if self.scoreboard.shown.contains(new) {
                self.send_all(&ClientboundPlay::DisplayScoreboard {
                    position: slot as u8,
                    name: new.to_owned(),
                });
            } else {
                for p in self.sb_show_packets(new) {
                    self.send_all(&p);
                }
                self.scoreboard.shown.insert(new.to_owned());
            }
        }
        self.scoreboard.dirty = true;
    }

    /// `Score.setScorePoints` (creating the score if needed).
    pub(super) fn sb_set_score(&mut self, holder: &str, objective: &str, points: i32) {
        let score = self.scoreboard.get_or_create(holder, objective);
        let changed = score.points != points || score.force_update;
        score.points = points;
        if changed {
            score.force_update = false;
            if self.scoreboard.shown.contains(objective) {
                self.send_all(&score_packet(holder, objective, points));
            }
            self.scoreboard.dirty = true;
        }
    }

    /// `Score.increseScore` (Java `int` arithmetic).
    pub(super) fn sb_add_score(&mut self, holder: &str, objective: &str, amount: i32) {
        let points = self.scoreboard.get_or_create(holder, objective).points;
        self.sb_set_score(holder, objective, points.wrapping_add(amount));
    }

    /// `Score.setLocked`, creating the score like `getValueFromObjective`.
    pub(super) fn sb_set_locked(&mut self, holder: &str, objective: &str, locked: bool) {
        self.scoreboard.get_or_create(holder, objective).locked = locked;
    }

    /// `removeObjectiveFromEntity`: one score, or every score of the holder.
    pub(super) fn sb_reset(&mut self, holder: &str, objective: Option<&str>) {
        let sb = &mut self.scoreboard;
        match objective {
            None => {
                if sb.scores.remove(holder).is_some() {
                    self.sb_send_reset_all(holder);
                }
            }
            Some(objective) => {
                let Some(list) = sb.scores.get_mut(holder) else {
                    return;
                };
                let before = list.len();
                list.retain(|(o, _)| o != objective);
                let removed = list.len() < before;
                if list.is_empty() {
                    sb.scores.remove(holder);
                    self.sb_send_reset_all(holder);
                } else if removed {
                    self.send_all(&ClientboundPlay::UpdateScore {
                        name: holder.to_owned(),
                        remove: true,
                        objective: objective.to_owned(),
                        value: 0,
                    });
                    self.scoreboard.dirty = true;
                }
            }
        }
    }

    fn sb_send_reset_all(&mut self, holder: &str) {
        self.send_all(&ClientboundPlay::UpdateScore {
            name: holder.to_owned(),
            remove: true,
            objective: String::new(),
            value: 0,
        });
        self.scoreboard.dirty = true;
    }

    /// `createTeam` (the name is valid and free).
    pub(super) fn sb_create_team(&mut self, name: &str) {
        let team = Team::new(name);
        let packet = team_packet(name, &team, 0);
        self.scoreboard.teams.insert(name.to_owned(), team);
        self.send_all(&packet);
        self.scoreboard.dirty = true;
    }

    /// Changes a team (`setTeamName`, `setNamePrefix`, ...): one update per call.
    pub(super) fn sb_update_team(&mut self, name: &str, change: impl FnOnce(&mut Team)) {
        let Some(team) = self.scoreboard.teams.get_mut(name) else {
            return;
        };
        change(team);
        let packet = team_packet(name, team, 2);
        self.send_all(&packet);
        self.scoreboard.dirty = true;
    }

    /// `removeTeam`.
    pub(super) fn sb_remove_team(&mut self, name: &str) {
        let Some(team) = self.scoreboard.teams.remove(name) else {
            return;
        };
        for (member, _) in team.members.iter() {
            self.scoreboard.memberships.remove(member);
        }
        self.send_all(&team_packet(name, &team, 1));
        self.scoreboard.dirty = true;
    }

    /// `addPlayerToTeam`: false if there is no such team.
    pub(super) fn sb_join_team(&mut self, holder: &str, team: &str) -> bool {
        if self.scoreboard.teams.get(team).is_none() {
            return false;
        }
        if self.scoreboard.team_of(holder).is_some() {
            self.sb_leave_teams(holder);
        }
        self.scoreboard
            .memberships
            .insert(holder.to_owned(), team.to_owned());
        if let Some(t) = self.scoreboard.teams.get_mut(team) {
            t.members.insert(holder.to_owned(), ());
        }
        self.send_all(&ClientboundPlay::Teams {
            name: team.to_owned(),
            mode: 3,
            info: None,
            players: vec![holder.to_owned()],
        });
        self.scoreboard.dirty = true;
        true
    }

    /// `removePlayerFromTeams`: false if the holder is on no team.
    pub(super) fn sb_leave_teams(&mut self, holder: &str) -> bool {
        let Some(team) = self.scoreboard.memberships.remove(holder) else {
            return false;
        };
        if let Some(t) = self.scoreboard.teams.get_mut(&team) {
            t.members.remove(holder);
        }
        self.send_all(&ClientboundPlay::Teams {
            name: team,
            mode: 4,
            info: None,
            players: vec![holder.to_owned()],
        });
        self.scoreboard.dirty = true;
        true
    }

    /// `ServerConfigurationManager.sendScoreboard`: the teams, then each displayed
    /// objective once.
    pub(super) fn sb_send_to_player(&self, i: usize) {
        let p = &self.players[i];
        for (name, team) in self.scoreboard.teams.iter() {
            p.send(team_packet(name, team, 0));
        }
        let mut sent: HashSet<&str> = HashSet::new();
        for slot in &self.scoreboard.slots {
            if let Some(name) = slot.as_deref()
                && sent.insert(name)
            {
                for packet in self.sb_show_packets(name) {
                    p.send(packet);
                }
            }
        }
    }

    /// The health criterion (`EntityPlayerMP.onUpdateEntity`): every health objective
    /// gets the player's health plus absorption, rounded up, when that sum changed.
    pub(super) fn sb_update_health(&mut self, i: usize) {
        let p = &mut self.players[i];
        let combined = p.health() + p.data.get_f32("AbsorptionAmount");
        if combined == p.combined_health {
            return;
        }
        p.combined_health = combined;
        let name = p.profile.name.clone();
        let value = krol_java::math::ceil_f32(combined);
        for objective in self.scoreboard.objectives_for(HEALTH) {
            self.sb_set_score(&name, &objective, value);
        }
    }

    /// `func_96648_a` on every objective of a criteria (death counts).
    pub(super) fn sb_increment(&mut self, holder: &str, criteria: &str) {
        for objective in self.scoreboard.objectives_for(criteria) {
            self.sb_add_score(holder, &objective, 1);
        }
    }

    /// Statistic criteria: the stat increases a player made since the last call
    /// (`EntityPlayerMP.addStat`).
    ///
    /// TODO(M5): vanilla updates the scores the moment the stat changes; krol applies
    /// them after the packet or tick step that changed it.
    pub(super) fn sb_apply_stats(&mut self, i: usize) {
        let changes = self.players[i].stats.take_increases();
        if changes.is_empty() {
            return;
        }
        let name = self.players[i].profile.name.clone();
        for (stat, amount) in changes {
            for objective in self.scoreboard.objectives_for(&stat) {
                self.sb_add_score(&name, &objective, amount);
            }
        }
    }

    /// `EntityPlayerMP.addStat`: raises a statistic and, at once, the scores of the
    /// objectives counting it.
    pub(super) fn add_stat(&mut self, i: usize, key: &str, amount: i32) {
        self.players[i].stats.add(key, amount);
        self.sb_apply_stats(i);
    }

    /// `EntityPlayerMP.func_175145_a`: resets a statistic and the scores of the
    /// objectives counting it to 0.
    pub(super) fn reset_stat(&mut self, i: usize, key: &str) {
        self.sb_apply_stats(i);
        self.players[i].stats.set(key, 0);
        let name = self.players[i].profile.name.clone();
        for objective in self.scoreboard.objectives_for(key) {
            self.sb_set_score(&name, &objective, 0);
        }
    }

    /// [`Self::sb_apply_stats`] for every player.
    pub(super) fn sb_apply_all_stats(&mut self) {
        for i in 0..self.players.len() {
            self.sb_apply_stats(i);
        }
    }

    /// `EntityPlayer.getTeam` of a player: its team's name.
    pub(super) fn team_of_player(&self, i: usize) -> Option<&str> {
        self.scoreboard.team_of(&self.players[i].profile.name)
    }

    /// `ScorePlayerTeam.formatPlayerName`: the name with its team's prefix and suffix.
    pub(super) fn team_formatted_name(&self, name: &str) -> String {
        match self
            .scoreboard
            .team_of(name)
            .and_then(|t| self.scoreboard.teams.get(t))
        {
            Some(t) => format!("{}{name}{}", t.prefix, t.suffix),
            None => name.to_owned(),
        }
    }
}

impl Scoreboard {
    /// Loads `<world>/data/scoreboard.dat` (`ScoreboardSaveData.readFromNBT`); missing
    /// or unreadable gives an empty scoreboard. Loading leaves it unchanged (not dirty).
    pub fn load(world_dir: &Path) -> Self {
        let mut sb = Scoreboard::default();
        let path = world_dir.join("data").join("scoreboard.dat");
        let Ok(file) = std::fs::File::open(&path) else {
            return sb;
        };
        let root = match krol_nbt::read_gzip(std::io::BufReader::new(file)) {
            Ok((_, root)) => root,
            Err(e) => {
                error!("Couldn't load scoreboard data: {e}");
                return sb;
            }
        };
        let Some(data) = root.get_compound("data") else {
            return sb;
        };
        sb.read(data);
        sb.dirty = false;
        sb
    }

    fn read(&mut self, data: &Compound) {
        let truncate = |s: &str, n: usize| -> String { s.chars().take(n).collect() };
        if let Some(list) = data.get_list("Objectives", TagType::Compound) {
            for t in list.iter() {
                let Tag::Compound(c) = t else { continue };
                let criteria = c.get_str("CriteriaName");
                if !criteria_exists(criteria) {
                    continue;
                }
                let name = truncate(c.get_str("Name"), MAX_NAME);
                if self.objectives.get(&name).is_some() {
                    // Vanilla throws here and gives up on the rest of the file.
                    error!("Couldn't load scoreboard data: duplicate objective {name}");
                    return;
                }
                self.objectives.insert(
                    name.clone(),
                    Objective {
                        criteria: criteria.to_owned(),
                        display: c.get_str("DisplayName").to_owned(),
                        render: render_by_name(c.get_str("RenderType")),
                    },
                );
                self.by_criteria
                    .entry(criteria.to_owned())
                    .or_default()
                    .push(name);
            }
        }
        if let Some(list) = data.get_list("PlayerScores", TagType::Compound) {
            for t in list.iter() {
                let Tag::Compound(c) = t else { continue };
                let objective = c.get_str("Objective");
                if self.objectives.get(objective).is_none() {
                    // TODO(M5): vanilla crashes on scores of unknown objectives.
                    continue;
                }
                let holder = truncate(c.get_str("Name"), MAX_HOLDER);
                let score = self.get_or_create(&holder, objective);
                score.points = c.get_i32("Score");
                score.force_update = false;
                if c.contains_key("Locked") {
                    score.locked = c.get_bool("Locked");
                }
            }
        }
        if let Some(slots) = data.get_compound("DisplaySlots") {
            for slot in 0..DISPLAY_SLOTS {
                if let Some(Tag::String(name)) = slots.get(&format!("slot_{slot}")) {
                    let found = self.objectives.get(name).is_some();
                    self.slots[slot] = found.then(|| name.clone());
                    if found {
                        // func_96549_e with nobody online: the objective counts as shown.
                        self.shown.insert(name.clone());
                    }
                }
            }
        }
        if let Some(list) = data.get_list("Teams", TagType::Compound) {
            for t in list.iter() {
                let Tag::Compound(c) = t else { continue };
                let name = truncate(c.get_str("Name"), MAX_NAME);
                if self.teams.get(&name).is_some() {
                    error!("Couldn't load scoreboard data: duplicate team {name}");
                    return;
                }
                let mut team = Team::new(&name);
                team.display = truncate(c.get_str("DisplayName"), MAX_DISPLAY);
                if let Some(Tag::String(color)) = c.get("TeamColor") {
                    // getValueByName may give null, which vanilla stores as is.
                    team.color = formatting_by_name(color).unwrap_or(RESET);
                }
                team.prefix = c.get_str("Prefix").to_owned();
                team.suffix = c.get_str("Suffix").to_owned();
                if c.contains_key_of("AllowFriendlyFire", None) {
                    team.friendly_fire = c.get_bool("AllowFriendlyFire");
                }
                if c.contains_key_of("SeeFriendlyInvisibles", None) {
                    team.see_invisibles = c.get_bool("SeeFriendlyInvisibles");
                }
                let visibility = |key: &str| match c.get(key) {
                    Some(Tag::String(v)) => {
                        VISIBILITIES.iter().find(|n| **n == v.as_str()).copied()
                    }
                    _ => None,
                };
                if let Some(v) = visibility("NameTagVisibility") {
                    team.nametag = v;
                }
                if let Some(v) = visibility("DeathMessageVisibility") {
                    team.death_messages = v;
                }
                self.teams.insert(name.clone(), team);
                if let Some(players) = c.get_list("Players", TagType::String) {
                    for p in players.iter() {
                        let Tag::String(p) = p else { continue };
                        if let Some(old) = self.memberships.insert(p.clone(), name.clone())
                            && let Some(t) = self.teams.get_mut(&old)
                        {
                            t.members.remove(p);
                        }
                        if let Some(t) = self.teams.get_mut(&name) {
                            t.members.insert(p.clone(), ());
                        }
                    }
                }
            }
        }
    }

    /// Writes `<world>/data/scoreboard.dat` if anything changed
    /// (`ScoreboardSaveData.writeToNBT`, saved by `MapStorage.saveAllData`).
    pub fn save(&mut self, world_dir: &Path) {
        if !self.dirty {
            return;
        }
        let mut data = Compound::new();
        let mut objectives = List::new();
        for (name, o) in self.objectives.iter() {
            let mut c = Compound::new();
            c.insert("Name", Tag::String(name.clone()));
            c.insert("CriteriaName", Tag::String(o.criteria.clone()));
            c.insert("DisplayName", Tag::String(o.display.clone()));
            c.insert("RenderType", Tag::String(o.render.to_owned()));
            let _ = objectives.push(Tag::Compound(c));
        }
        data.insert("Objectives", Tag::List(objectives));
        let mut scores = List::new();
        for (holder, list) in self.scores.iter() {
            for (objective, s) in list {
                let mut c = Compound::new();
                c.insert("Name", Tag::String(holder.clone()));
                c.insert("Objective", Tag::String(objective.clone()));
                c.insert("Score", Tag::Int(s.points));
                c.insert("Locked", Tag::Byte(i8::from(s.locked)));
                let _ = scores.push(Tag::Compound(c));
            }
        }
        data.insert("PlayerScores", Tag::List(scores));
        let mut teams = List::new();
        for (name, t) in self.teams.iter() {
            let mut c = Compound::new();
            c.insert("Name", Tag::String(name.clone()));
            c.insert("DisplayName", Tag::String(t.display.clone()));
            if t.color < COLORS {
                c.insert("TeamColor", Tag::String(FORMATTING[t.color].0.to_owned()));
            }
            c.insert("Prefix", Tag::String(t.prefix.clone()));
            c.insert("Suffix", Tag::String(t.suffix.clone()));
            c.insert("AllowFriendlyFire", Tag::Byte(i8::from(t.friendly_fire)));
            c.insert(
                "SeeFriendlyInvisibles",
                Tag::Byte(i8::from(t.see_invisibles)),
            );
            c.insert("NameTagVisibility", Tag::String(t.nametag.to_owned()));
            c.insert(
                "DeathMessageVisibility",
                Tag::String(t.death_messages.to_owned()),
            );
            let mut players = List::new();
            for (m, _) in t.members.iter() {
                let _ = players.push(Tag::String(m.clone()));
            }
            c.insert("Players", Tag::List(players));
            let _ = teams.push(Tag::Compound(c));
        }
        data.insert("Teams", Tag::List(teams));
        let mut slots = Compound::new();
        for (slot, name) in self.slots.iter().enumerate() {
            if let Some(name) = name {
                slots.insert(format!("slot_{slot}"), Tag::String(name.clone()));
            }
        }
        if !slots.is_empty() {
            data.insert("DisplaySlots", Tag::Compound(slots));
        }
        let mut root = Compound::new();
        root.insert("data", Tag::Compound(data));
        let dir = world_dir.join("data");
        let result = std::fs::create_dir_all(&dir).and_then(|()| {
            let file = std::fs::File::create(dir.join("scoreboard.dat"))?;
            krol_nbt::write_gzip(std::io::BufWriter::new(file), "", &root)
                .map_err(std::io::Error::other)
        });
        match result {
            Ok(()) => self.dirty = false,
            Err(e) => error!("Could not save data scoreboard: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_and_colors() {
        assert_eq!(slot_by_name("SIDEBAR"), Some(1));
        assert_eq!(slot_by_name("sidebar.team.dark_blue"), Some(4));
        assert_eq!(slot_by_name("sidebar.team.darkblue"), Some(4));
        assert_eq!(slot_by_name("sidebar.team.bold"), None);
        assert_eq!(slot_name(18).as_deref(), Some("sidebar.team.white"));
        assert_eq!(formatting_by_name("Light_Purple"), Some(13));
        assert_eq!(color_names().len(), 17);
        assert!(criteria_exists("stat.mineBlock.minecraft.stone"));
        assert!(!criteria_exists("nope"));
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("krol-sb-{}", std::process::id()));
        let mut sb = Scoreboard::default();
        sb.objectives.insert(
            "k".into(),
            Objective {
                criteria: "dummy".into(),
                display: "Kills".into(),
                render: "integer",
            },
        );
        sb.by_criteria.insert("dummy".into(), vec!["k".into()]);
        sb.get_or_create("jeb_", "k").points = 4;
        sb.slots[1] = Some("k".into());
        let mut team = Team::new("red");
        team.color = 12;
        team.members.insert("jeb_".into(), ());
        sb.teams.insert("red".into(), team);
        sb.memberships.insert("jeb_".into(), "red".into());
        sb.dirty = true;
        sb.save(&dir);
        let loaded = Scoreboard::load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            loaded.objectives.get("k").map(|o| o.display.as_str()),
            Some("Kills")
        );
        assert_eq!(loaded.score("jeb_", "k").map(|s| s.points), Some(4));
        assert_eq!(loaded.slots[1].as_deref(), Some("k"));
        assert!(loaded.shown.contains("k"));
        assert_eq!(loaded.team_of("jeb_"), Some("red"));
        assert_eq!(loaded.teams.get("red").map(|t| t.color), Some(12));
        assert!(!loaded.dirty);
    }

    #[test]
    fn sorted_scores() {
        let mut sb = Scoreboard::default();
        sb.get_or_create("b", "o").points = 1;
        sb.get_or_create("A", "o").points = 1;
        sb.get_or_create("c", "o").points = -5;
        assert_eq!(
            sb.sorted_scores("o"),
            [("c".into(), -5), ("A".into(), 1), ("b".into(), 1)]
        );
    }
}
