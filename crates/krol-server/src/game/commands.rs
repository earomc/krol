//! Commands (vanilla: `ServerCommandManager`, `CommandBase` and the `Command*` classes).
//!
//! Players receive translation-key components (rendered by their client); the console
//! log gets the en_US text (see [`crate::lang`]). Permission checks, operator
//! notifications (`[Server: ...]`) and argument parsing follow vanilla. Commands vanilla
//! has but krol does not implement yet are listed by `/help` and answer with a notice.

use krol_nbt::Tag;
use krol_protocol::packets::play::ClientboundPlay;
use tracing::{info, warn};

mod scoreboard;

use super::Game;
use super::ids::block;
use super::selector::{self, Target};
use crate::chat::{Arg, Component};
use crate::status::java_uuid_string;
use crate::userlist::{BanInfo, IpBan, OpEntry, PlayerBan, WhitelistEntry};

/// A vanilla command: name, usage translation key, en_US usage, permission level.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    /// Command name.
    pub name: &'static str,
    /// Usage translation key.
    pub usage_key: &'static str,
    /// en_US usage text.
    pub usage: &'static str,
    /// Required permission level (vanilla `getRequiredPermissionLevel`).
    pub level: i32,
}

macro_rules! cmd {
    ($name:literal, $key:literal, $usage:literal, $level:literal) => {
        CommandSpec {
            name: $name,
            usage_key: $key,
            usage: $usage,
            level: $level,
        }
    };
}

/// Every vanilla 1.8.8 dedicated-server command, in `/help` order.
pub const COMMANDS: &[CommandSpec] = &[
    cmd!(
        "achievement",
        "commands.achievement.usage",
        "/achievement <give|take> <stat_name|*> [player]",
        2
    ),
    cmd!("ban", "commands.ban.usage", "/ban <name> [reason ...]", 3),
    cmd!(
        "ban-ip",
        "commands.banip.usage",
        "/ban-ip <address|name> [reason ...]",
        3
    ),
    cmd!(
        "banlist",
        "commands.banlist.usage",
        "/banlist [ips|players]",
        3
    ),
    cmd!(
        "blockdata",
        "commands.blockdata.usage",
        "/blockdata <x> <y> <z> <dataTag>",
        2
    ),
    cmd!(
        "clear",
        "commands.clear.usage",
        "/clear [player] [item] [data] [maxCount] [dataTag]",
        2
    ),
    cmd!(
        "clone",
        "commands.clone.usage",
        "/clone <x1> <y1> <z1> <x2> <y2> <z2> <x> <y> <z> [maskMode] [cloneMode]",
        2
    ),
    cmd!("debug", "commands.debug.usage", "/debug <start|stop>", 3),
    cmd!(
        "defaultgamemode",
        "commands.defaultgamemode.usage",
        "/defaultgamemode <mode>",
        2
    ),
    cmd!("deop", "commands.deop.usage", "/deop <player>", 3),
    cmd!(
        "difficulty",
        "commands.difficulty.usage",
        "/difficulty <new difficulty>",
        2
    ),
    cmd!(
        "effect",
        "commands.effect.usage",
        "/effect <player> <effect> [seconds] [amplifier] [hideParticles] OR /effect <player> clear",
        2
    ),
    cmd!(
        "enchant",
        "commands.enchant.usage",
        "/enchant <player> <enchantment ID> [level]",
        2
    ),
    cmd!(
        "entitydata",
        "commands.entitydata.usage",
        "/entitydata <entity> <dataTag>",
        2
    ),
    cmd!(
        "execute",
        "commands.execute.usage",
        "/execute <entity> <x> <y> <z> <command> OR /execute <entity> <x> <y> <z> detect <x> <y> <z> <block> <data> <command>",
        2
    ),
    cmd!(
        "fill",
        "commands.fill.usage",
        "/fill <x1> <y1> <z1> <x2> <y2> <z2> <TileName> [dataValue] [oldBlockHandling] [dataTag]",
        2
    ),
    cmd!(
        "gamemode",
        "commands.gamemode.usage",
        "/gamemode <mode> [player]",
        2
    ),
    cmd!(
        "gamerule",
        "commands.gamerule.usage",
        "/gamerule <rule name> [value]",
        2
    ),
    cmd!(
        "give",
        "commands.give.usage",
        "/give <player> <item> [amount] [data] [dataTag]",
        2
    ),
    cmd!(
        "help",
        "commands.help.usage",
        "/help [page|command name]",
        0
    ),
    cmd!(
        "kick",
        "commands.kick.usage",
        "/kick <player> [reason ...]",
        3
    ),
    cmd!("kill", "commands.kill.usage", "/kill [player|entity]", 2),
    cmd!("list", "commands.players.usage", "/list", 0),
    cmd!("me", "commands.me.usage", "/me <action ...>", 0),
    cmd!("op", "commands.op.usage", "/op <player>", 3),
    cmd!("pardon", "commands.unban.usage", "/pardon <name>", 3),
    cmd!(
        "pardon-ip",
        "commands.unbanip.usage",
        "/pardon-ip <address>",
        3
    ),
    cmd!(
        "particle",
        "commands.particle.usage",
        "/particle <name> <x> <y> <z> <xd> <yd> <zd> <speed> [count] [mode]",
        2
    ),
    cmd!(
        "playsound",
        "commands.playsound.usage",
        "/playsound <sound> <player> [x] [y] [z] [volume] [pitch] [minimumVolume]",
        2
    ),
    cmd!(
        "replaceitem",
        "commands.replaceitem.usage",
        "/replaceitem <entity|block> ...",
        2
    ),
    cmd!("save-all", "commands.save.usage", "/save-all", 4),
    cmd!("save-off", "commands.save-off.usage", "/save-off", 4),
    cmd!("save-on", "commands.save-on.usage", "/save-on", 4),
    cmd!("say", "commands.say.usage", "/say <message ...>", 1),
    cmd!(
        "scoreboard",
        "commands.scoreboard.usage",
        "/scoreboard <objectives|players|teams> ...",
        2
    ),
    cmd!("seed", "commands.seed.usage", "/seed", 2),
    cmd!(
        "setblock",
        "commands.setblock.usage",
        "/setblock <x> <y> <z> <TileName> [dataValue] [oldBlockHandling] [dataTag]",
        2
    ),
    cmd!(
        "setidletimeout",
        "commands.setidletimeout.usage",
        "/setidletimeout <Minutes until kick>",
        3
    ),
    cmd!(
        "setworldspawn",
        "commands.setworldspawn.usage",
        "/setworldspawn [<x> <y> <z>]",
        2
    ),
    cmd!(
        "spawnpoint",
        "commands.spawnpoint.usage",
        "/spawnpoint [player] [<x> <y> <z>]",
        2
    ),
    cmd!(
        "spreadplayers",
        "commands.spreadplayers.usage",
        "/spreadplayers <x> <z> <spreadDistance> <maxRange> <respectTeams true|false> <player ...>",
        2
    ),
    cmd!(
        "stats",
        "commands.stats.usage",
        "/stats <entity|block> ...",
        2
    ),
    cmd!("stop", "commands.stop.usage", "/stop", 4),
    cmd!(
        "summon",
        "commands.summon.usage",
        "/summon <EntityName> [x] [y] [z] [dataTag]",
        2
    ),
    cmd!(
        "tell",
        "commands.message.usage",
        "/tell <player> <private message ...>",
        0
    ),
    cmd!(
        "tellraw",
        "commands.tellraw.usage",
        "/tellraw <player> <raw json message>",
        2
    ),
    cmd!(
        "testfor",
        "commands.testfor.usage",
        "/testfor <player> [dataTag]",
        2
    ),
    cmd!(
        "testforblock",
        "commands.testforblock.usage",
        "/testforblock <x> <y> <z> <TileName> [dataValue] [dataTag]",
        2
    ),
    cmd!(
        "testforblocks",
        "commands.compare.usage",
        "/testforblocks <x1> <y1> <z1> <x2> <y2> <z2> <x> <y> <z> [mode]",
        2
    ),
    cmd!(
        "time",
        "commands.time.usage",
        "/time <set|add|query> <value>",
        2
    ),
    cmd!(
        "title",
        "commands.title.usage",
        "/title <player> <title|subtitle|clear|reset|times> ...",
        2
    ),
    cmd!(
        "toggledownfall",
        "commands.downfall.usage",
        "/toggledownfall",
        2
    ),
    cmd!(
        "tp",
        "commands.tp.usage",
        "/tp [target player] <destination player> OR /tp [target player] <x> <y> <z> [<y-rot> <x-rot>]",
        2
    ),
    cmd!(
        "trigger",
        "commands.trigger.usage",
        "/trigger <objective> <add|set> <value>",
        0
    ),
    cmd!(
        "weather",
        "commands.weather.usage",
        "/weather <clear|rain|thunder> [duration in seconds]",
        2
    ),
    cmd!(
        "whitelist",
        "commands.whitelist.usage",
        "/whitelist <on|off|list|add|remove|reload>",
        3
    ),
    cmd!(
        "worldborder",
        "commands.worldborder.usage",
        "/worldborder <set|center|damage|warning|get|add> ...",
        2
    ),
    cmd!(
        "xp",
        "commands.xp.usage",
        "/xp <amount> [player] OR /xp <amount>L [player]",
        2
    ),
];

/// Aliases vanilla registers (`ICommand.getCommandAliases`).
const ALIASES: &[(&str, &str)] = &[("?", "help"), ("msg", "tell"), ("w", "tell")];

/// Commands any player may use (vanilla `EntityPlayerMP.canCommandSenderUseCommand`).
const PUBLIC: &[&str] = &["tell", "help", "me", "trigger"];

/// Vanilla's default game rules (1.8.8), alphabetical.
pub const DEFAULT_GAME_RULES: &[(&str, &str)] = &[
    ("commandBlockOutput", "true"),
    ("doDaylightCycle", "true"),
    ("doEntityDrops", "true"),
    ("doFireTick", "true"),
    ("doMobLoot", "true"),
    ("doMobSpawning", "true"),
    ("doTileDrops", "true"),
    ("keepInventory", "false"),
    ("logAdminCommands", "true"),
    ("mobGriefing", "true"),
    ("naturalRegeneration", "true"),
    ("randomTickSpeed", "3"),
    ("reducedDebugInfo", "false"),
    ("sendCommandFeedback", "true"),
    ("showDeathMessages", "true"),
];

/// The en_US usage text for a usage key.
pub fn usage_text(key: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|c| c.usage_key == key)
        .map(|c| c.usage)
}

fn spec(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// Who runs a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sender {
    /// The server console.
    Console,
    /// The player at this index in `Game::players`.
    Player(usize),
}

/// A command failure, rendered like vanilla's `CommandException`s.
#[derive(Debug, Clone)]
pub enum CommandError {
    /// `WrongUsageException` with a usage key.
    Usage(&'static str),
    /// `WrongUsageException` with a message key and arguments.
    UsageWith(&'static str, Vec<Arg>),
    /// A translated error.
    Translated(&'static str, Vec<Arg>),
    /// An unexpected exception in vanilla (reported as `commands.generic.exception` and
    /// logged).
    Exception,
}

type CommandResult = Result<(), CommandError>;

/// `CommandBase.getItemByText`: an item by resource location (no numeric ids;
/// `ResourceLocation` lowercases the namespace only).
fn item_by_text(text: &str) -> Result<i16, CommandError> {
    let location = match text.find(':') {
        // Vanilla quirk: a one-character domain is dropped (only `i > 1` takes it).
        Some(i) if i > 1 => format!("{}:{}", text[..i].to_lowercase(), &text[i + 1..]),
        Some(i) => format!("minecraft:{}", &text[i + 1..]),
        None => format!("minecraft:{text}"),
    };
    krol_world::items::id_for_name(&location)
        .ok_or_else(|| err("commands.give.item.notFound", vec![s(location)]))
}

/// `ICommand.isUsernameIndex`: which argument names a player (and may be a selector
/// for several), for the commands that have one.
fn username_index(name: &str, args: &[&str]) -> Option<usize> {
    let index = match name {
        "clear" | "effect" | "enchant" | "entitydata" | "execute" | "give" | "kill"
        | "spawnpoint" | "title" | "tell" | "tellraw" | "tp" | "testfor" => 0,
        "gamemode" | "playsound" | "xp" => 1,
        "achievement" => 2,
        "replaceitem" | "stats" if args.first() == Some(&"entity") => 1,
        // CommandScoreboard: `players <action> <player>` and friends.
        "scoreboard" => return scoreboard_username_index(args),
        _ => return None,
    };
    (index < args.len()).then_some(index)
}

/// `CommandScoreboard.isUsernameIndex` for one position.
fn scoreboard_is_username_index(args: &[&str], k: usize) -> bool {
    let Some(first) = args.first() else {
        return false;
    };
    if !first.eq_ignore_ascii_case("players") {
        return first.eq_ignore_ascii_case("teams") && k == 2;
    }
    if args.len() > 1 && args[1].eq_ignore_ascii_case("operation") {
        k == 2 || k == 5
    } else {
        k == 2
    }
}

/// `CommandScoreboard.isUsernameIndex`.
fn scoreboard_username_index(args: &[&str]) -> Option<usize> {
    let first = args.first()?;
    let index = if !first.eq_ignore_ascii_case("players") {
        if first.eq_ignore_ascii_case("teams") {
            2
        } else {
            return None;
        }
    } else if args.len() > 1 && args[1].eq_ignore_ascii_case("operation") {
        // Vanilla checks index 2 and 5; the first one found wins.
        return [2, 5]
            .into_iter()
            .find(|&k| k < args.len() && selector::matches_multiple(args[k]));
    } else {
        2
    };
    (index < args.len()).then_some(index)
}

/// The first main inventory slot of the player window (armor comes before it).
const MAIN_INVENTORY_FIRST: usize = 9;

/// Parses a data tag from the remaining arguments (joined with spaces), logging the
/// parser's warnings like vanilla; syntax errors become `error_key`.
fn data_tag(args: &[&str], error_key: &'static str) -> Result<krol_nbt::Compound, CommandError> {
    let mut warnings = Vec::new();
    let parsed =
        krol_nbt::text::parse_data_tag(&args.join(" "), &mut |w| warnings.push(w.to_owned()));
    for w in warnings {
        warn!("{w}");
    }
    parsed.map_err(|e| match e {
        krol_nbt::text::ParseError::Syntax(message) => err(error_key, vec![s(message)]),
        krol_nbt::text::ParseError::Internal => CommandError::Exception,
    })
}

/// The most blocks `/fill` sets at once.
const MAX_FILL: i64 = 32768;

/// The most items `/give` hands out at once.
const MAX_GIVE: i8 = 64;

/// The creative game mode.
const CREATIVE: i32 = 1;

fn usage(name: &str) -> CommandError {
    CommandError::Usage(spec(name).map_or("", |c| c.usage_key))
}

fn s(v: impl ToString) -> Arg {
    Arg::Str(v.to_string())
}

fn err(key: &'static str, args: Vec<Arg>) -> CommandError {
    CommandError::Translated(key, args)
}

fn t(key: &str, args: Vec<Arg>) -> Component {
    Component::translate(key, args)
}

/// Vanilla `CommandBase.parseInt(input, min, max)`.
fn parse_int(input: &str, min: i32, max: i32) -> Result<i32, CommandError> {
    let v = crate::config::parse_java_int(input)
        .ok_or_else(|| err("commands.generic.num.invalid", vec![s(input)]))?;
    if v < min {
        return Err(err("commands.generic.num.tooSmall", vec![s(v), s(min)]));
    }
    if v > max {
        return Err(err("commands.generic.num.tooBig", vec![s(v), s(max)]));
    }
    Ok(v)
}

/// Vanilla `CommandBase.joinNiceString`: "a, b and c".
pub fn join_nice<S: AsRef<str>>(items: &[S]) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(if i == items.len() - 1 { " and " } else { ", " });
        }
        out.push_str(item.as_ref());
    }
    out
}

fn difficulty_key(d: i32) -> &'static str {
    match d {
        0 => "options.difficulty.peaceful",
        1 => "options.difficulty.easy",
        2 => "options.difficulty.normal",
        _ => "options.difficulty.hard",
    }
}

fn game_mode_key(m: i32) -> &'static str {
    match m {
        1 => "gameMode.creative",
        2 => "gameMode.adventure",
        3 => "gameMode.spectator",
        _ => "gameMode.survival",
    }
}

/// Vanilla `CommandGameMode.getGameModeFromCommand`.
fn parse_game_mode(s: &str) -> Result<i32, CommandError> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "survival" | "s" => 0,
        "creative" | "c" => 1,
        "adventure" | "a" => 2,
        "spectator" | "sp" => 3,
        _ => parse_int(s, 0, 3)?,
    })
}

/// Vanilla's IPv4 pattern for `/ban-ip` and `/pardon-ip`.
fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u32>().is_ok_and(|v| v <= 255)
        })
}

impl Game {
    /// Sends `message` to the sender: logged for the console, a system message for
    /// players.
    pub(super) fn reply(&self, sender: Sender, message: &Component) {
        match sender {
            Sender::Console => info!("{}", message.unformatted()),
            Sender::Player(i) => self.players[i].send(ClientboundPlay::Chat {
                json: message.to_json_string(),
                position: 1,
            }),
        }
    }

    fn sender_name(&self, sender: Sender) -> String {
        match sender {
            Sender::Console => "Server".into(),
            Sender::Player(i) => self.players[i].profile.name.clone(),
        }
    }

    fn sender_component(&self, sender: Sender) -> Component {
        match sender {
            Sender::Console => Component::text("Server"),
            Sender::Player(i) => self.display_name(&self.players[i].profile),
        }
    }

    pub(super) fn op_level(&self, i: usize) -> Option<i32> {
        let key = java_uuid_string(self.players[i].profile.id);
        let mut lists = self.shared.user_lists.lock().ok()?;
        lists.ops.get(&key).map(|op| op.level)
    }

    /// Vanilla `canCommandSenderUseCommand`.
    fn can_use(&self, sender: Sender, command: &CommandSpec) -> bool {
        match sender {
            Sender::Console => true,
            Sender::Player(i) => {
                PUBLIC.contains(&command.name)
                    || self.op_level(i).is_some_and(|l| l >= command.level)
            }
        }
    }

    fn game_rule_true(&self, rule: &str) -> bool {
        self.game_rules()
            .iter()
            .find(|(k, _)| k == rule)
            .is_none_or(|(_, v)| v == "true")
    }

    /// Vanilla `ServerCommandManager.notifyOperators`: tells other operators
    /// (`[sender: message]`, gray italic), logs that for player senders when
    /// `logAdminCommands` is on, and gives the sender feedback unless `quiet` (flag 1)
    /// or `sendCommandFeedback` is off (the console always gets it).
    fn notify(&self, sender: Sender, command: &str, quiet: bool, key: &str, args: Vec<Arg>) {
        let message = t(key, args);
        let admin = t(
            "chat.type.admin",
            vec![s(self.sender_name(sender)), Arg::Component(message.clone())],
        )
        .color("gray")
        .italic();
        let spec = spec(command);
        for (i, p) in self.players.iter().enumerate() {
            if Sender::Player(i) != sender
                && self.op_level(i).is_some()
                && spec.is_some_and(|c| self.can_use(Sender::Player(i), c))
            {
                p.send(ClientboundPlay::Chat {
                    json: admin.to_json_string(),
                    position: 1,
                });
            }
        }
        if sender != Sender::Console && self.game_rule_true("logAdminCommands") {
            info!("{}", admin.unformatted());
        }
        if sender == Sender::Console || (!quiet && self.game_rule_true("sendCommandFeedback")) {
            self.reply(sender, &message);
        }
    }

    /// Vanilla `getPlayer` by name (case-insensitive).
    fn find_player(&self, name: &str) -> Result<usize, CommandError> {
        self.players
            .iter()
            .position(|p| p.profile.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| err("commands.generic.player.notFound", vec![]))
    }

    /// Vanilla `CommandBase.getPlayer`: exactly one player matched by a selector, else
    /// the player with this UUID, else by name.
    fn get_player(&self, sender: Sender, text: &str) -> Result<usize, CommandError> {
        if let [Target::Player(i)] = self.match_entities(sender, text, true)[..] {
            return Ok(i);
        }
        if let Some(uuid) = selector::java_uuid_from_string(text)
            && let Some(i) = self.players.iter().position(|p| p.profile.id == uuid)
        {
            return Ok(i);
        }
        self.find_player(text)
    }

    /// Vanilla `CommandBase.getEntity`: exactly one entity matched by a selector, else a
    /// player by name, else the entity or player with this UUID. `living` stands for
    /// the `EntityLivingBase` class filter (players are the only living entities krol
    /// has).
    fn get_entity(&self, sender: Sender, text: &str, living: bool) -> Result<Target, CommandError> {
        // Vanilla quirk: the class filter applies while matching, so a selector that
        // only finds other entities falls through to the UUID parse ("invalid format").
        let matched = self.match_entities(sender, text, living);
        let found = match matched[..] {
            [t] => Some(t),
            _ => None,
        };
        let found = found.or_else(|| self.find_player(text).ok().map(Target::Player));
        let found = match found {
            Some(t) => Some(t),
            None => {
                let Some(uuid) = selector::java_uuid_from_string(text) else {
                    return Err(err("commands.generic.entity.invalidUuid", vec![]));
                };
                self.entity_by_uuid(uuid)
            }
        };
        match found {
            Some(Target::Entity(_)) if living => {
                Err(err("commands.generic.entity.notFound", vec![]))
            }
            Some(t) => Ok(t),
            None => Err(err("commands.generic.entity.notFound", vec![])),
        }
    }

    /// Vanilla `getChatComponentFromNthArg`: the words as text siblings with spaces
    /// between; with `selectors`, a selector word becomes the matched entities' names
    /// (`CommandBase.join`), and a selector matching nothing fails.
    fn words_component(
        &self,
        sender: Sender,
        words: &[&str],
        selectors: bool,
    ) -> Result<Component, CommandError> {
        let mut c = Component::text("");
        for (k, word) in words.iter().enumerate() {
            if k > 0 {
                c.extra.push(Component::text(" "));
            }
            let mut part = Component::text(*word);
            if selectors {
                let matched = self.match_entities(sender, word, false);
                if !matched.is_empty() {
                    part = self.join_names(&matched);
                } else if selector::is_selector(word) {
                    return Err(err("commands.generic.player.notFound", vec![]));
                }
            }
            c.extra.push(part);
        }
        Ok(c)
    }

    /// `CommandBase.join` of the targets' display names: "a, b and c".
    fn join_names(&self, targets: &[Target]) -> Component {
        let mut c = Component::text("");
        for (k, target) in targets.iter().enumerate() {
            if k > 0 {
                c.extra.push(Component::text(if k == targets.len() - 1 {
                    " and "
                } else {
                    ", "
                }));
            }
            c.extra.push(match *target {
                Target::Player(i) => self.display_name(&self.players[i].profile),
                Target::Entity(id) => self.entity_display_name(id),
            });
        }
        c
    }

    /// Vanilla `PlayerProfileCache.getGameProfileForUsername`.
    ///
    /// Known deviation: for names not in the cache vanilla asks Mojang's profile API
    /// (even in offline mode, falling back to the offline UUID). krol uses online players,
    /// then the offline UUID in offline mode, and fails in online mode.
    fn profile_for_name(&mut self, name: &str) -> Option<(u128, String)> {
        if let Some(found) = self.user_cache.get(name) {
            return Some(found);
        }
        if let Ok(i) = self.find_player(name) {
            let p = &self.players[i].profile;
            return Some((p.id, p.name.clone()));
        }
        if self.shared.config.online_mode {
            warn!("krol: looking up unknown player names with Mojang is not implemented yet");
            None
        } else {
            Some((crate::auth::offline_uuid(name), name.to_owned()))
        }
    }

    /// Vanilla `ServerCommandManager.executeCommand`.
    pub(super) fn execute_command(&mut self, sender: Sender, raw: &str) {
        let raw = raw.trim();
        let raw = raw.strip_prefix('/').unwrap_or(raw);
        let mut parts = raw.split(' ');
        let name = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();
        let name = ALIASES
            .iter()
            .find(|(a, _)| *a == name)
            .map_or(name, |(_, n)| n);
        let Some(command) = spec(name) else {
            self.reply_error(sender, &err("commands.generic.notFound", vec![]));
            return;
        };
        if !self.can_use(sender, command) {
            self.reply_error(sender, &err("commands.generic.permission", vec![]));
            return;
        }
        // A selector for several entities runs the command once per entity, with the
        // entity's UUID in its place.
        if let Some(index) = username_index(name, &args)
            && selector::matches_multiple(args[index])
        {
            let targets = self.match_entities(sender, args[index], false);
            for target in targets {
                let mut expanded: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
                expanded[index] = java_uuid_string(self.target_uuid(target));
                let expanded: Vec<&str> = expanded.iter().map(String::as_str).collect();
                self.try_execute(sender, name, &expanded, raw);
            }
            return;
        }
        self.try_execute(sender, name, &args, raw);
    }

    /// Vanilla `CommandHandler.tryExecute`: runs the command and reports its failure.
    fn try_execute(&mut self, sender: Sender, name: &str, args: &[&str], raw: &str) {
        match self.run_command(sender, name, args) {
            Ok(()) => {}
            Err(CommandError::Exception) => {
                self.reply_error(sender, &CommandError::Exception);
                warn!("Couldn't process command: '{raw}'");
            }
            Err(e) => self.reply_error(sender, &e),
        }
    }

    fn reply_error(&self, sender: Sender, e: &CommandError) {
        let message = match e {
            CommandError::Usage(key) => t(
                "commands.generic.usage",
                vec![Arg::Component(t(key, vec![]))],
            ),
            CommandError::UsageWith(key, args) => t(
                "commands.generic.usage",
                vec![Arg::Component(t(key, args.clone()))],
            ),
            CommandError::Translated(key, args) => t(key, args.clone()),
            CommandError::Exception => t("commands.generic.exception", vec![]),
        };
        self.reply(sender, &message.color("red"));
    }

    fn require_player(&self, sender: Sender, arg: Option<&&str>) -> Result<usize, CommandError> {
        match (arg, sender) {
            (Some(p), _) => self.get_player(sender, p),
            (None, Sender::Player(i)) => Ok(i),
            (None, Sender::Console) => Err(err("commands.generic.player.unspecified", vec![])),
        }
    }

    fn run_command(&mut self, sender: Sender, name: &str, args: &[&str]) -> CommandResult {
        match name {
            "help" => self.cmd_help(sender, args),
            "list" => {
                let uuids = args
                    .first()
                    .is_some_and(|a| a.eq_ignore_ascii_case("uuids"));
                let names: Vec<String> = self
                    .players
                    .iter()
                    .map(|p| {
                        if uuids {
                            format!("{} ({})", p.profile.name, java_uuid_string(p.profile.id))
                        } else {
                            p.profile.name.clone()
                        }
                    })
                    .collect();
                self.reply(
                    sender,
                    &t(
                        "commands.players.list",
                        vec![s(self.players.len()), s(self.config.max_players)],
                    ),
                );
                self.reply(sender, &Component::text(join_nice(&names)));
                Ok(())
            }
            "say" | "me" => {
                // Vanilla: /say needs a non-empty first argument, /me any argument.
                let missing = if name == "say" {
                    args.first().is_none_or(|a| a.is_empty())
                } else {
                    args.is_empty()
                };
                if missing {
                    return Err(usage(name));
                }
                let key = if name == "say" {
                    "chat.type.announcement"
                } else {
                    "chat.type.emote"
                };
                // /say resolves selectors always, /me only for senders other than players.
                let selectors = name == "say" || !matches!(sender, Sender::Player(_));
                let words = self.words_component(sender, args, selectors)?;
                let msg = t(
                    key,
                    vec![
                        Arg::Component(self.sender_component(sender)),
                        Arg::Component(words),
                    ],
                );
                self.broadcast_chat(&msg, None);
                Ok(())
            }
            "tell" => {
                if args.len() < 2 {
                    return Err(usage(name));
                }
                let target = self.get_player(sender, args[0])?;
                if Sender::Player(target) == sender {
                    return Err(err("commands.message.sameTarget", vec![]));
                }
                let selectors = !matches!(sender, Sender::Player(_));
                let text = self.words_component(sender, &args[1..], selectors)?;
                let incoming = t(
                    "commands.message.display.incoming",
                    vec![
                        Arg::Component(self.sender_component(sender)),
                        Arg::Component(text.clone()),
                    ],
                )
                .color("gray")
                .italic();
                let outgoing = t(
                    "commands.message.display.outgoing",
                    vec![
                        Arg::Component(self.display_name(&self.players[target].profile)),
                        Arg::Component(text),
                    ],
                )
                .color("gray")
                .italic();
                self.reply(Sender::Player(target), &incoming);
                self.reply(sender, &outgoing);
                Ok(())
            }
            "time" => self.cmd_time(sender, args),
            "weather" => self.cmd_weather(sender, args),
            "toggledownfall" => {
                let raining = self.level.data.raining();
                self.level
                    .data
                    .data
                    .insert("raining", Tag::Byte(i8::from(!raining)));
                self.notify(sender, name, false, "commands.downfall.success", vec![]);
                Ok(())
            }
            "difficulty" => {
                let a = args.first().ok_or_else(|| usage(name))?;
                let d = match a.to_ascii_lowercase().as_str() {
                    "peaceful" | "p" => 0,
                    "easy" | "e" => 1,
                    "normal" | "n" => 2,
                    "hard" | "h" => 3,
                    _ => parse_int(a, 0, 3)?,
                };
                self.config.difficulty = d;
                self.level
                    .data
                    .data
                    .insert("Difficulty", Tag::Byte(d as i8));
                for p in &self.players {
                    p.send(ClientboundPlay::ServerDifficulty {
                        difficulty: d as u8,
                    });
                }
                let shown = Arg::Component(t(difficulty_key(d), vec![]));
                self.notify(
                    sender,
                    name,
                    false,
                    "commands.difficulty.success",
                    vec![shown],
                );
                Ok(())
            }
            "defaultgamemode" => {
                let a = args.first().ok_or_else(|| usage(name))?;
                let m = parse_game_mode(a)?;
                self.config.gamemode = m;
                self.level.data.set_game_type(m);
                // TODO: with force-gamemode, vanilla also changes every player's mode.
                let shown = Arg::Component(t(game_mode_key(m), vec![]));
                self.notify(
                    sender,
                    name,
                    false,
                    "commands.defaultgamemode.success",
                    vec![shown],
                );
                Ok(())
            }
            "gamemode" => {
                let a = args.first().ok_or_else(|| usage(name))?;
                let mode = parse_game_mode(a)?;
                let target = self.require_player(sender, args.get(1))?;
                self.set_game_mode(target, mode);
                if self.game_rule_true("sendCommandFeedback") {
                    self.reply(Sender::Player(target), &t("gameMode.changed", vec![]));
                }
                let shown = Arg::Component(t(game_mode_key(mode), vec![]));
                if Sender::Player(target) == sender {
                    self.notify(
                        sender,
                        name,
                        true,
                        "commands.gamemode.success.self",
                        vec![shown],
                    );
                } else {
                    let target_name = s(&self.players[target].profile.name);
                    self.notify(
                        sender,
                        name,
                        true,
                        "commands.gamemode.success.other",
                        vec![target_name, shown],
                    );
                }
                Ok(())
            }
            "xp" => {
                let a = args.first().ok_or_else(|| usage(name))?;
                let (levels, number) = match a.strip_suffix(['l', 'L']) {
                    Some(n) => (true, n),
                    None => (false, *a),
                };
                let amount = if levels {
                    parse_int(number, i32::MIN, i32::MAX)?
                } else {
                    parse_int(number, 0, i32::MAX)?
                };
                let target = self.require_player(sender, args.get(1))?;
                let target_name = self.players[target].profile.name.clone();
                if levels {
                    self.players[target].add_experience_levels(amount);
                    let key = if amount < 0 {
                        "commands.xp.success.negative.levels"
                    } else {
                        "commands.xp.success.levels"
                    };
                    self.notify(
                        sender,
                        name,
                        false,
                        key,
                        vec![s(amount.abs()), s(target_name)],
                    );
                } else {
                    self.players[target].add_experience(amount);
                    self.notify(
                        sender,
                        name,
                        false,
                        "commands.xp.success",
                        vec![s(amount), s(target_name)],
                    );
                }
                Ok(())
            }
            "tp" => self.cmd_tp(sender, args),
            "kick" => {
                let target = args.first().ok_or_else(|| usage(name))?;
                let i = self.find_player(target)?;
                let player_name = self.players[i].profile.name.clone();
                let reason = Component::from_words(&args[1..]).unformatted();
                if args.len() < 2 {
                    self.kick(i, "Kicked by an operator.");
                    self.notify(
                        sender,
                        name,
                        false,
                        "commands.kick.success",
                        vec![s(player_name)],
                    );
                } else {
                    self.kick(i, &reason);
                    self.notify(
                        sender,
                        name,
                        false,
                        "commands.kick.success.reason",
                        vec![s(player_name), s(reason)],
                    );
                }
                Ok(())
            }
            "seed" => {
                let seed = self.level.data.seed();
                self.reply(sender, &t("commands.seed.success", vec![s(seed)]));
                Ok(())
            }
            "gamerule" => self.cmd_gamerule(sender, args),
            "setblock" => self.cmd_setblock(sender, args),
            "fill" => self.cmd_fill(sender, args),
            "effect" => self.cmd_effect(sender, args),
            "enchant" => self.cmd_enchant(sender, args),
            "give" => self.cmd_give(sender, args),
            "save-off" | "save-on" => {
                let enable = name == "save-on";
                if self.saving_enabled == enable {
                    let key = if enable {
                        "commands.save-on.alreadyOn"
                    } else {
                        "commands.save-off.alreadyOff"
                    };
                    return Err(err(key, vec![]));
                }
                self.saving_enabled = enable;
                let key = if enable {
                    "commands.save.enabled"
                } else {
                    "commands.save.disabled"
                };
                self.notify(sender, name, false, key, vec![]);
                Ok(())
            }
            "save-all" => {
                self.reply(sender, &t("commands.save.start", vec![]));
                for i in 0..self.players.len() {
                    self.save_player(i);
                }
                self.save_level(false);
                self.scoreboard.save(&self.level.dir);
                self.maps.save(&self.level.dir);
                self.notify(sender, name, false, "commands.save.success", vec![]);
                Ok(())
            }
            "banlist" => {
                let ips = args.first().is_some_and(|a| a.eq_ignore_ascii_case("ips"));
                let names: Vec<String> = {
                    let lists = self.shared.user_lists.lock().map_err(|_| err("", vec![]))?;
                    if ips {
                        lists.banned_ips.iter().map(|b| b.ip.clone()).collect()
                    } else {
                        lists
                            .banned_players
                            .iter()
                            .map(|b| b.name.clone())
                            .collect()
                    }
                };
                let key = if ips {
                    "commands.banlist.ips"
                } else {
                    "commands.banlist.players"
                };
                self.reply(sender, &t(key, vec![s(names.len())]));
                self.reply(sender, &Component::text(join_nice(&names)));
                Ok(())
            }
            "whitelist" => self.cmd_whitelist(sender, args),
            "op" | "deop" | "ban" | "pardon" | "ban-ip" | "pardon-ip" => {
                self.cmd_profile(sender, name, args)
            }
            "setidletimeout" => {
                let a = args.first().ok_or_else(|| usage(name))?;
                let minutes = parse_int(a, 0, i32::MAX)?;
                self.props.set("player-idle-timeout", minutes);
                self.props.save();
                self.notify(
                    sender,
                    name,
                    false,
                    "commands.setidletimeout.success",
                    vec![s(minutes)],
                );
                Ok(())
            }
            "setworldspawn" => {
                let pos = match (sender, args.len()) {
                    (_, 3) => (
                        parse_int(args[0], i32::MIN, i32::MAX)?,
                        parse_int(args[1], i32::MIN, i32::MAX)?,
                        parse_int(args[2], i32::MIN, i32::MAX)?,
                    ),
                    (Sender::Player(i), 0) => {
                        let p = &self.players[i];
                        (
                            krol_java::math::floor_f64(p.pos.0),
                            krol_java::math::floor_f64(p.pos.1),
                            krol_java::math::floor_f64(p.pos.2),
                        )
                    }
                    (Sender::Console, 0) => {
                        return Err(err("commands.generic.player.unspecified", vec![]));
                    }
                    _ => return Err(usage(name)),
                };
                self.level.data.set_spawn_x(pos.0);
                self.level.data.set_spawn_y(pos.1);
                self.level.data.set_spawn_z(pos.2);
                for p in &self.players {
                    p.send(ClientboundPlay::SpawnPosition { pos });
                }
                self.notify(
                    sender,
                    name,
                    false,
                    "commands.setworldspawn.success",
                    vec![s(pos.0), s(pos.1), s(pos.2)],
                );
                Ok(())
            }
            "stop" => {
                self.notify(sender, name, false, "commands.stop.start", vec![]);
                self.stopping = true;
                Ok(())
            }
            "kill" => self.cmd_kill(sender, args),
            "scoreboard" => self.cmd_scoreboard(sender, args),
            "trigger" => self.cmd_trigger(sender, args),
            "clear" => self.cmd_clear(sender, args),
            "spawnpoint" => self.cmd_spawnpoint(sender, args),
            _ => self.not_implemented(sender, name),
        }
    }

    /// `/kill [player|entity]` (`CommandKill`): `onKillCommand`, out-of-world damage of
    /// `Float.MAX_VALUE`.
    fn cmd_kill(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let target = match args.first() {
            Some(target) => self.get_entity(sender, target, false)?,
            None => Target::Player(self.require_player(sender, None)?),
        };
        let name = match target {
            Target::Player(i) => {
                let name = Arg::Component(self.display_name(&self.players[i].profile));
                self.attack_player(i, super::damage::DamageSource::OutOfWorld, f32::MAX);
                name
            }
            Target::Entity(id) => {
                // Entity.onKillCommand: setDead.
                let name = Arg::Component(self.entity_display_name(id));
                self.set_entity_dead(id);
                name
            }
        };
        self.notify(
            sender,
            "kill",
            false,
            "commands.kill.successful",
            vec![name],
        );
        Ok(())
    }

    /// `/clear [player] [item] [data] [maxCount] [dataTag]` (`CommandClearInventory`,
    /// `InventoryPlayer.clearMatchingItems`): the main inventory (hotbar first), the
    /// armor (boots first), then the item on the cursor.
    fn cmd_clear(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let i = self.require_player(sender, args.first())?;
        let item = match args.get(1) {
            Some(text) => Some(item_by_text(text)?),
            None => None,
        };
        let data = match args.get(2) {
            Some(a) => parse_int(a, -1, i32::MAX)?,
            None => -1,
        };
        let max = match args.get(3) {
            Some(a) => parse_int(a, -1, i32::MAX)?,
            None => -1,
        };
        let tag = if args.len() >= 5 {
            Some(krol_nbt::Tag::Compound(data_tag(
                &args[4..],
                "commands.clear.tagError",
            )?))
        } else {
            None
        };
        // Main inventory slots match lists partially, armor and the cursor exactly.
        let matches = |stack: &krol_protocol::packets::play::ItemStack, partial: bool| {
            item.is_none_or(|id| stack.id == id)
                && (data <= -1 || i32::from(stack.damage) == data)
                && tag.as_ref().is_none_or(|t| {
                    let target = stack.nbt.clone().map(krol_nbt::Tag::Compound);
                    krol_nbt::matches_pattern(t, target.as_ref(), partial)
                })
        };
        // mainInventory (0-8 hotbar, 9-35 main), then armorInventory (boots first).
        let slots: Vec<usize> = (36..45).chain(9..36).chain([8, 7, 6, 5]).collect();
        let mut removed = 0;
        let p = &mut self.players[i];
        let take = |slot: &mut Option<krol_protocol::packets::play::ItemStack>,
                    removed: &mut i32,
                    partial: bool|
         -> bool {
            let Some(stack) = slot.as_mut().filter(|s| matches(s, partial)) else {
                return false;
            };
            let count = i32::from(stack.count);
            let k = if max <= 0 {
                count
            } else {
                (max - *removed).min(count)
            };
            *removed += k;
            if max != 0 {
                stack.count -= k as i8;
                if stack.count == 0 {
                    *slot = None;
                }
                if max > 0 && *removed >= max {
                    return true;
                }
            }
            false
        };
        let mut done = false;
        for slot in slots {
            if take(
                &mut p.inventory[slot],
                &mut removed,
                slot >= MAIN_INVENTORY_FIRST,
            ) {
                done = true;
                break;
            }
        }
        if !done {
            take(&mut p.window.cursor, &mut removed, false);
        }
        self.detect_inventory_changes(i);
        let p = &self.players[i];
        if p.gamemode != CREATIVE {
            // updateHeldItem: the cursor.
            p.send(ClientboundPlay::SetSlot {
                window: -1,
                slot: -1,
                item: p.window.cursor.clone(),
            });
        }
        let name = p.profile.name.clone();
        if removed == 0 {
            return Err(err("commands.clear.failure", vec![s(name)]));
        }
        if max == 0 {
            self.reply(
                sender,
                &t("commands.clear.testing", vec![s(name), s(removed)]),
            );
        } else {
            self.notify(
                sender,
                "clear",
                false,
                "commands.clear.success",
                vec![s(name), s(removed)],
            );
        }
        Ok(())
    }

    /// `/spawnpoint [player] [<x> <y> <z>]` (`CommandSetSpawnpoint`): a forced spawn
    /// point at the coordinates or the player's block position.
    fn cmd_spawnpoint(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.len() > 1 && args.len() < 4 {
            return Err(usage("spawnpoint"));
        }
        let i = self.require_player(sender, args.first())?;
        let pos = if args.len() > 3 {
            self.parse_block_pos(sender, &args[1..4])?
        } else {
            // Entity.getPosition: the block at the feet, half a block up.
            let p = self.players[i].pos;
            (
                krol_java::math::floor_f64(p.0),
                krol_java::math::floor_f64(p.1 + 0.5),
                krol_java::math::floor_f64(p.2),
            )
        };
        super::beds::set_forced_spawn_point(&mut self.players[i], pos);
        let name = self.players[i].profile.name.clone();
        self.notify(
            sender,
            "spawnpoint",
            false,
            "commands.spawnpoint.success",
            vec![s(name), s(pos.0), s(pos.1), s(pos.2)],
        );
        Ok(())
    }

    fn not_implemented(&self, sender: Sender, name: &str) -> CommandResult {
        warn!("krol: /{name} is not implemented yet");
        self.reply(
            sender,
            &Component::text(format!("krol: /{name} is not implemented yet")).color("red"),
        );
        Ok(())
    }

    fn cmd_help(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        const PER_PAGE: usize = 7;
        let usable: Vec<&CommandSpec> = COMMANDS
            .iter()
            .filter(|c| self.can_use(sender, c))
            .collect();
        let pages = usable.len().div_ceil(PER_PAGE).max(1);
        let page = match args.first() {
            None => 1,
            Some(a) => match crate::config::parse_java_int(a) {
                Some(_) => parse_int(a, 1, pages as i32)? as usize,
                None => {
                    // `/help <command>` shows that command's usage as an error.
                    let name = ALIASES
                        .iter()
                        .find(|(al, _)| al == a)
                        .map_or(*a, |(_, n)| n);
                    return match spec(name) {
                        Some(c) => Err(CommandError::Usage(c.usage_key)),
                        None => Err(err("commands.generic.notFound", vec![])),
                    };
                }
            },
        };
        let header = t("commands.help.header", vec![s(page), s(pages)]).color("dark_green");
        self.reply(sender, &header);
        for c in usable.iter().skip((page - 1) * PER_PAGE).take(PER_PAGE) {
            let mut line = t(c.usage_key, vec![]);
            line.style.click = Some(("suggest_command".into(), format!("/{} ", c.name)));
            self.reply(sender, &line);
        }
        if page == 1 && matches!(sender, Sender::Player(_)) {
            self.reply(sender, &t("commands.help.footer", vec![]).color("green"));
        }
        Ok(())
    }

    fn cmd_time(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.len() < 2 {
            return Err(usage("time"));
        }
        match args[0] {
            "set" => {
                let v = match args[1] {
                    "day" => 1000,
                    "night" => 13000,
                    a => parse_int(a, 0, i32::MAX)?,
                };
                self.level.data.set_day_time(i64::from(v));
                self.notify(sender, "time", false, "commands.time.set", vec![s(v)]);
            }
            "add" => {
                let v = parse_int(args[1], 0, i32::MAX)?;
                let time = self.level.data.day_time() + i64::from(v);
                self.level.data.set_day_time(time);
                self.notify(sender, "time", false, "commands.time.added", vec![s(v)]);
            }
            "query" => {
                let v = match args[1] {
                    "daytime" => self.level.data.day_time() % 2_147_483_647,
                    "gametime" => self.level.data.time() % 2_147_483_647,
                    _ => return Err(usage("time")),
                };
                self.notify(sender, "time", false, "commands.time.query", vec![s(v)]);
            }
            _ => return Err(usage("time")),
        }
        Ok(())
    }

    fn cmd_weather(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.is_empty() || args.len() > 2 {
            return Err(usage("weather"));
        }
        let ticks = match args.get(1) {
            Some(a) => parse_int(a, 1, 1_000_000)? * 20,
            None => (300 + self.weather_rand.next_int_bounded(600)) * 20,
        };
        let (key, rain, thunder) = match args[0].to_ascii_lowercase().as_str() {
            "clear" => ("commands.weather.clear", false, false),
            "rain" => ("commands.weather.rain", true, false),
            "thunder" => ("commands.weather.thunder", true, true),
            _ => return Err(usage("weather")),
        };
        let data = &mut self.level.data;
        if rain {
            data.set_clear_weather_time(0);
            data.set_rain_time(ticks);
            data.set_thunder_time(ticks);
        } else {
            data.set_clear_weather_time(ticks);
            data.set_rain_time(0);
            data.set_thunder_time(0);
        }
        data.data.insert("raining", Tag::Byte(i8::from(rain)));
        data.data.insert("thundering", Tag::Byte(i8::from(thunder)));
        self.notify(sender, "weather", false, key, vec![]);
        Ok(())
    }

    /// `/tp [target] <destination>` and `/tp [target] <x> <y> <z> [<yaw> <pitch>]`
    /// (`CommandTeleport`). With 2, 4 or 6 arguments the first names the entity to move
    /// (`getEntity`), otherwise the sender moves.
    fn cmd_tp(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return Err(usage("tp"));
        }
        let (target, first) = if matches!(args.len(), 2 | 4 | 6) {
            (self.get_entity(sender, args[0], false)?, 1)
        } else {
            (Target::Player(self.require_player(sender, None)?), 0)
        };
        if args.len() <= 2 {
            let destination = self.get_entity(sender, args[args.len() - 1], false)?;
            let (pos, rot) = self.target_location(destination);
            match target {
                Target::Player(i) => {
                    Self::set_player_location(&mut self.players[i], pos, rot);
                }
                Target::Entity(id) => self.set_location_and_angles(id, pos, rot),
            }
            let names = vec![
                s(self.target_name(target)),
                s(self.target_name(destination)),
            ];
            self.notify(sender, "tp", false, "commands.tp.success", names);
            return Ok(());
        }
        if args.len() < first + 3 {
            return Err(usage("tp"));
        }
        let (pos, rot) = self.target_location(target);
        let x = parse_coordinate_arg(pos.0, args[first], Some(COORD_LIMIT), true)?;
        let y = parse_coordinate_arg(pos.1, args[first + 1], None, false)?;
        let z = parse_coordinate_arg(pos.2, args[first + 2], Some(COORD_LIMIT), true)?;
        let yaw_arg = args.get(first + 3).copied().unwrap_or("~");
        let pitch_arg = args.get(first + 4).copied().unwrap_or("~");
        let yaw = parse_coordinate_arg(f64::from(rot.0), yaw_arg, Some(COORD_LIMIT), false)?;
        let pitch = parse_coordinate_arg(f64::from(rot.1), pitch_arg, Some(COORD_LIMIT), false)?;
        use krol_java::math::{wrap_degrees_f32, wrap_degrees_f64};
        match target {
            Target::Player(i) => {
                // Relative arguments become relative flags of the position packet.
                let mut flags = 0;
                for (arg, flag) in [
                    (&x, TELEPORT_X),
                    (&y, TELEPORT_Y),
                    (&z, TELEPORT_Z),
                    (&pitch, TELEPORT_PITCH),
                    (&yaw, TELEPORT_YAW),
                ] {
                    if arg.relative {
                        flags |= flag;
                    }
                }
                let mut yaw_value = yaw.amount as f32;
                if !yaw.relative {
                    yaw_value = wrap_degrees_f32(yaw_value);
                }
                let mut pitch_value = pitch.amount as f32;
                if !pitch.relative {
                    pitch_value = wrap_degrees_f32(pitch_value);
                }
                if !(-90.0..=90.0).contains(&pitch_value) {
                    pitch_value = wrap_degrees_f32(180.0 - pitch_value);
                    yaw_value = wrap_degrees_f32(yaw_value + 180.0);
                }
                Self::set_player_location_relative(
                    &mut self.players[i],
                    (x.amount, y.amount, z.amount),
                    (yaw_value, pitch_value),
                    flags,
                );
                // TODO(M5): setRotationYawHead(yaw) (krol keeps no head yaw for
                // players yet).
            }
            Target::Entity(id) => {
                let mut yaw_value = wrap_degrees_f64(yaw.value) as f32;
                let mut pitch_value = wrap_degrees_f64(pitch.value) as f32;
                if !(-90.0..=90.0).contains(&pitch_value) {
                    pitch_value = wrap_degrees_f32(180.0 - pitch_value);
                    yaw_value = wrap_degrees_f32(yaw_value + 180.0);
                }
                self.set_location_and_angles(
                    id,
                    (x.value, y.value, z.value),
                    (yaw_value, pitch_value),
                );
            }
        }
        let f = krol_java::fmt::java_double;
        let name = self.target_name(target);
        self.notify(
            sender,
            "tp",
            false,
            "commands.tp.success.coordinates",
            vec![s(name), s(f(x.value)), s(f(y.value)), s(f(z.value))],
        );
        Ok(())
    }

    /// `CommandBase.parseBlockPos` (not centered): relative to the sender's block
    /// position (the origin for the console).
    fn parse_block_pos(
        &self,
        sender: Sender,
        args: &[&str],
    ) -> Result<(i32, i32, i32), CommandError> {
        let base = match sender {
            Sender::Console => (0, 0, 0),
            Sender::Player(i) => {
                let p = self.players[i].pos;
                (
                    krol_java::math::floor_f64(p.0),
                    krol_java::math::floor_f64(p.1 + 0.5),
                    krol_java::math::floor_f64(p.2),
                )
            }
        };
        let x = parse_coordinate_bounded(f64::from(base.0), args[0], COORD_LIMIT)?;
        let y = parse_coordinate_bounded(f64::from(base.1), args[1], (0, 256))?;
        let z = parse_coordinate_bounded(f64::from(base.2), args[2], COORD_LIMIT)?;
        Ok((
            krol_java::math::floor_f64(x),
            krol_java::math::floor_f64(y),
            krol_java::math::floor_f64(z),
        ))
    }

    /// Vanilla `CommandEffect` (on players).
    /// Vanilla `CommandEnchant`: enchants the held item of a player.
    fn cmd_enchant(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        use super::enchanting as ench;
        if args.len() < 2 {
            return Err(usage("enchant"));
        }
        let target = self.get_player(sender, args[0])?;
        let id = match parse_int(args[1], 0, i32::MAX) {
            Ok(id) => id,
            Err(e) => match ench::by_name(args[1]) {
                Some(id) => i32::from(id),
                None => return Err(e),
            },
        };
        let Some(stack) = self.players[target].held().cloned() else {
            return Err(err("commands.enchant.noItem", vec![]));
        };
        if !ench::exists(id) {
            return Err(err("commands.enchant.notFound", vec![s(id)]));
        }
        let id = id as i16;
        if !ench::can_apply(id, &stack) {
            return Err(err("commands.enchant.cantEnchant", vec![]));
        }
        let level = if args.len() >= 3 {
            parse_int(args[2], 1, ench::max_level_of(id))?
        } else {
            1
        };
        for (other, other_level) in super::item_data::enchantments(&stack) {
            if ench::exists(i32::from(other)) && !ench::can_apply_together(other, id) {
                return Err(err(
                    "commands.enchant.cantCombine",
                    vec![
                        s(ench::translated_name(id, level)),
                        s(ench::translated_name(other, other_level)),
                    ],
                ));
            }
        }
        let p = &mut self.players[target];
        let slot = 36 + p.selected;
        if let Some(held) = p.inventory[slot].as_mut() {
            ench::enchant_stack(held, id, level);
        }
        self.notify(sender, "enchant", false, "commands.enchant.success", vec![]);
        Ok(())
    }

    fn cmd_give(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        /// `EntityItem.setInfinitePickupDelay`.
        const NEVER_PICKED_UP: i32 = 32767;
        /// The age `EntityItem.func_174870_v` gives the fake item: it despawns (at
        /// 6000) on its next tick.
        const FAKE_ITEM_AGE: i32 = 5999;
        const POP_VOLUME: f32 = 0.2;
        if args.len() < 2 {
            return Err(usage("give"));
        }
        let target = self.get_player(sender, args[0])?;
        let id = item_by_text(args[1])?;
        let amount = match args.get(2) {
            Some(a) => parse_int(a, 1, i32::from(MAX_GIVE))?,
            None => 1,
        };
        let damage = match args.get(3) {
            Some(a) => parse_int(a, i32::MIN, i32::MAX)?,
            None => 0,
        };
        let mut stack = krol_protocol::packets::play::ItemStack {
            id,
            count: amount as i8,
            // `new ItemStack` clamps negative damage; NBT and packets keep a short.
            damage: damage.max(0) as i16,
            nbt: None,
        };
        if args.len() >= 5 {
            stack.nbt = Some(data_tag(&args[4..], "commands.give.tagError")?);
        }
        let p = &mut self.players[target];
        let added = super::item_entity::add_to_inventory(
            &mut p.inventory,
            &mut stack,
            p.gamemode == super::player::Player::CREATIVE,
        );
        if added {
            let p = &mut self.players[target];
            let r = &mut p.rand;
            let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
            let pos = p.pos;
            self.play_sound_near_except(None, "random.pop", pos, POP_VOLUME, pitch);
            self.detect_inventory_changes(target);
        }
        // TODO(M5): command result stats (AFFECTED_ITEMS, needs /stats).
        if added && stack.count <= 0 {
            // Vanilla shows a fake item that cannot be picked up and vanishes next tick.
            stack.count = 1;
            let item = self.toss(target, stack.clone(), false);
            if let Some(it) = self.items.get_mut(&item) {
                it.pickup_delay = NEVER_PICKED_UP;
                it.age = FAKE_ITEM_AGE;
            }
        } else {
            let item = self.toss(target, stack.clone(), false);
            let owner = self.players[target].profile.name.clone();
            if let Some(it) = self.items.get_mut(&item) {
                it.pickup_delay = 0;
                it.owner = Some(owner);
            }
        }
        let name = self.players[target].profile.name.clone();
        self.notify(
            sender,
            "give",
            false,
            "commands.give.success",
            vec![
                Arg::Component(super::item_data::chat_component(&stack)),
                s(amount),
                s(name),
            ],
        );
        Ok(())
    }

    fn cmd_effect(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        use super::potions::{Effect, effect_by_name, effect_type};
        if args.len() < 2 {
            return Err(usage("effect"));
        }
        // Players are the only living entities krol has, so the living filter of
        // getEntity leaves only them.
        let Target::Player(target) = self.get_entity(sender, args[0], true)? else {
            return Err(err("commands.generic.entity.notFound", vec![]));
        };
        let name = self.players[target].profile.name.clone();
        if args[1] == "clear" {
            if self.players[target].effects.is_empty() {
                return Err(err("commands.effect.failure.notActive.all", vec![s(name)]));
            }
            self.clear_effects(target);
            self.notify(
                sender,
                "effect",
                false,
                "commands.effect.success.removed.all",
                vec![s(name)],
            );
            return Ok(());
        }
        let id = match parse_int(args[1], 1, i32::MAX) {
            Ok(id) => id,
            Err(e) => match effect_by_name(args[1]) {
                Some(id) => i32::from(id),
                None => return Err(e),
            },
        };
        let Some(kind) = effect_type(id) else {
            return Err(err("commands.effect.notFound", vec![s(id)]));
        };
        let mut seconds = DEFAULT_EFFECT_SECONDS;
        let mut duration = DEFAULT_EFFECT_SECONDS * 20;
        if args.len() >= 3 {
            seconds = parse_int(args[2], 0, MAX_EFFECT_SECONDS)?;
            duration = if kind.instant { seconds } else { seconds * 20 };
        } else if kind.instant {
            duration = 1;
        }
        let amplifier = if args.len() >= 4 {
            parse_int(args[3], 0, 255)?
        } else {
            0
        };
        let particles = !(args.len() >= 5 && args[4].eq_ignore_ascii_case("true"));
        let effect_name = Arg::Component(Component::translate(kind.name, vec![]));
        if seconds > 0 {
            self.add_effect(
                target,
                Effect {
                    id: id as u8,
                    amplifier: amplifier as u8,
                    duration,
                    ambient: false,
                    particles,
                },
            );
            self.notify(
                sender,
                "effect",
                false,
                "commands.effect.success",
                vec![effect_name, s(id), s(amplifier), s(name), s(seconds)],
            );
        } else if self.remove_effect(target, id as u8) {
            self.notify(
                sender,
                "effect",
                false,
                "commands.effect.success.removed",
                vec![effect_name, s(name)],
            );
        } else {
            return Err(err(
                "commands.effect.failure.notActive",
                vec![effect_name, s(name)],
            ));
        }
        Ok(())
    }

    /// Vanilla `CommandSetBlock` (data tags are not supported yet).
    fn cmd_setblock(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.len() < 4 {
            return Err(usage("setblock"));
        }
        let pos = self.parse_block_pos(sender, args)?;
        let id = block_by_text(args[3])?;
        let meta = if args.len() >= 5 {
            parse_int(args[4], 0, 15)?
        } else {
            0
        };
        let loaded = (0..256).contains(&pos.1)
            && self.level.overworld.is_chunk_loaded(pos.0 >> 4, pos.2 >> 4);
        if !loaded {
            return Err(err("commands.setblock.outOfWorld", vec![]));
        }
        let tag = if args.len() >= 7 && super::world::has_tile_entity(id) {
            Some(data_tag(&args[6..], "commands.setblock.tagError")?)
        } else {
            None
        };
        if args.len() >= 6 {
            if args[5] == "destroy" {
                let old = self.level.overworld.block(pos.0, pos.1, pos.2);
                if old >> 4 != block::AIR {
                    self.destroy_block(pos, old);
                }
                if id == block::AIR {
                    self.notify(
                        sender,
                        "setblock",
                        false,
                        "commands.setblock.success",
                        vec![],
                    );
                    return Ok(());
                }
            } else if args[5] == "keep"
                && self.level.overworld.block(pos.0, pos.1, pos.2) >> 4 != block::AIR
            {
                return Err(err("commands.setblock.noChange", vec![]));
            }
        }
        // An old tile entity is emptied (no drops) and removed first.
        if let Some(te) = self.level.overworld.tile_entity_mut(pos) {
            te.remove("Items");
            let flags = if id == block::AIR {
                super::block_update::flags::SEND
            } else {
                0
            };
            self.set_block_flags(pos, 0, flags);
        }
        // Block.getStateFromMeta, which throws for some metadata.
        let Some(meta) = super::world::normalize_meta(id, meta as u8) else {
            return Err(CommandError::Exception);
        };
        let state = id << 4 | u16::from(meta);
        if self.level.overworld.block(pos.0, pos.1, pos.2) == state {
            return Err(err("commands.setblock.noChange", vec![]));
        }
        self.set_block_flags(pos, state, super::block_update::flags::SEND);
        if let Some(tag) = tag
            && let Some(fresh) = self.level.overworld.tile_entity(pos).cloned()
        {
            // TileEntityFurnace.readFromNBT derives the current fuel's burn time anew.
            self.furnace_burn.remove(&pos);
            let te = super::tile_data::read_tile_entity(&fresh, &tag);
            self.level.overworld.set_tile_entity(pos, te);
        }
        self.notify_neighbors(pos, id);
        self.notify(
            sender,
            "setblock",
            false,
            "commands.setblock.success",
            vec![],
        );
        Ok(())
    }

    /// Vanilla `CommandFill`: every block of a box, in z, y, x order, set without
    /// neighbor updates, which follow for every changed block once the box is done.
    fn cmd_fill(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.len() < 7 {
            return Err(usage("fill"));
        }
        let a = self.parse_block_pos(sender, args)?;
        let b = self.parse_block_pos(sender, &args[3..])?;
        let id = block_by_text(args[6])?;
        let meta = if args.len() >= 8 {
            parse_int(args[7], 0, 15)?
        } else {
            0
        };
        let min = (a.0.min(b.0), a.1.min(b.1), a.2.min(b.2));
        let max = (a.0.max(b.0), a.1.max(b.1), a.2.max(b.2));
        // Vanilla's int arithmetic; parsed coordinates keep it from overflowing.
        let volume =
            (max.0 - min.0 + 1) as i64 * (max.1 - min.1 + 1) as i64 * (max.2 - min.2 + 1) as i64;
        if volume > MAX_FILL {
            return Err(err(
                "commands.fill.tooManyBlocks",
                vec![s(volume), s(MAX_FILL)],
            ));
        }
        if min.1 < 0 || max.1 >= 256 {
            return Err(err("commands.fill.outOfWorld", vec![]));
        }
        // Vanilla quirk: steps of 16 from the low corner up to 15 past the high one, so
        // a chunk beyond the box can be checked too.
        let mut z = min.2;
        while z < max.2 + 16 {
            let mut x = min.0;
            while x < max.0 + 16 {
                if !self.level.overworld.is_chunk_loaded(x >> 4, z >> 4) {
                    return Err(err("commands.fill.outOfWorld", vec![]));
                }
                x += 16;
            }
            z += 16;
        }
        let has_te = super::world::has_tile_entity(id);
        let tag = if args.len() >= 10 && has_te {
            Some(data_tag(&args[9..], "commands.fill.tagError")?)
        } else {
            None
        };
        let mode = args.get(8).copied();
        let filter = match mode {
            // Vanilla quirk: the filter is ignored for blocks with a tile entity.
            Some("replace") if !has_te => {
                let block = args.get(9).map(|a| block_by_text(a)).transpose()?;
                let meta = args
                    .get(10)
                    .map(|a| parse_int(a, i32::MIN, i32::MAX))
                    .transpose()?;
                Some((block, meta))
            }
            _ => None,
        };
        // Block.getStateFromMeta, which throws for some metadata.
        let Some(meta) = super::world::normalize_meta(id, meta as u8) else {
            return Err(CommandError::Exception);
        };
        let state = id << 4 | u16::from(meta);
        let mut changed = Vec::new();
        let mut count = 0;
        for z in min.2..=max.2 {
            for y in min.1..=max.1 {
                for x in min.0..=max.0 {
                    let pos = (x, y, z);
                    let edge = x == min.0
                        || x == max.0
                        || y == min.1
                        || y == max.1
                        || z == min.2
                        || z == max.2;
                    let old = self.level.overworld.block(x, y, z);
                    match mode {
                        Some("outline" | "hollow") if !edge => {
                            if mode == Some("hollow") {
                                self.set_block_flags(pos, 0, super::block_update::flags::SEND);
                                changed.push(pos);
                            }
                            continue;
                        }
                        Some("destroy") => {
                            if old >> 4 != block::AIR {
                                self.destroy_block(pos, old);
                            }
                        }
                        Some("keep") if old >> 4 != block::AIR => continue,
                        _ => {}
                    }
                    if let Some((block, want)) = filter {
                        if block.is_some_and(|b| old >> 4 != b) {
                            continue;
                        }
                        if want.is_some_and(|m| i32::from(old & 15) != m) {
                            continue;
                        }
                    }
                    // An old tile entity is emptied (no drops) and swapped for a barrier.
                    if let Some(te) = self.level.overworld.tile_entity_mut(pos) {
                        te.remove("Items");
                        let flags = if id == block::BARRIER {
                            super::block_update::flags::SEND
                        } else {
                            0
                        };
                        self.set_block_flags(pos, block::BARRIER << 4, flags);
                    }
                    // Counted when set, even if the block changed itself right away
                    // (rails take their shape in onBlockAdded).
                    if !self.set_block_flags(pos, state, super::block_update::flags::SEND) {
                        continue;
                    }
                    changed.push(pos);
                    count += 1;
                    if let Some(tag) = &tag
                        && let Some(fresh) = self.level.overworld.tile_entity(pos).cloned()
                    {
                        self.furnace_burn.remove(&pos);
                        let te = super::tile_data::read_tile_entity(&fresh, tag);
                        self.level.overworld.set_tile_entity(pos, te);
                    }
                }
            }
        }
        for pos in changed {
            let id = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
            self.notify_neighbors(pos, id);
        }
        if count == 0 {
            return Err(err("commands.fill.failed", vec![]));
        }
        self.notify(
            sender,
            "fill",
            false,
            "commands.fill.success",
            vec![s(count)],
        );
        Ok(())
    }

    fn cmd_gamerule(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let rules = self.game_rules();
        match args.len() {
            0 => {
                let names: Vec<&String> = rules.iter().map(|(k, _)| k).collect();
                self.reply(sender, &Component::text(join_nice(&names)));
            }
            1 => match rules.iter().find(|(k, _)| k == args[0]) {
                Some((k, v)) => self.reply(sender, &Component::text(format!("{k} = {v}"))),
                None => return Err(err("commands.gamerule.norule", vec![s(args[0])])),
            },
            _ => {
                let value = args[1..].join(" ");
                self.set_game_rule(args[0], &value);
                self.notify(
                    sender,
                    "gamerule",
                    false,
                    "commands.gamerule.success",
                    vec![],
                );
            }
        }
        Ok(())
    }

    /// All game rules (defaults overlaid with `level.dat`), sorted by name like vanilla's
    /// `TreeMap`.
    /// `GameRules.setOrCreateGameRule`.
    pub(super) fn set_game_rule(&mut self, name: &str, value: &str) {
        let mut compound = self
            .level
            .data
            .data
            .get_compound("GameRules")
            .cloned()
            .unwrap_or_default();
        compound.insert(name, Tag::String(value.to_owned()));
        self.level
            .data
            .data
            .insert("GameRules", Tag::Compound(compound));
    }

    pub(super) fn game_rules(&self) -> Vec<(String, String)> {
        let mut rules: std::collections::BTreeMap<String, String> = DEFAULT_GAME_RULES
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        if let Some(c) = self.level.data.data.get_compound("GameRules") {
            for (k, v) in c {
                if let Tag::String(s) = v {
                    rules.insert(k.clone(), s.clone());
                }
            }
        }
        rules.into_iter().collect()
    }

    fn cmd_whitelist(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let a = args.first().ok_or_else(|| usage("whitelist"))?;
        match *a {
            "on" | "off" => {
                let on = *a == "on";
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    l.whitelist_enforced = on;
                }
                self.props.set("white-list", on);
                self.props.save();
                let key = if on {
                    "commands.whitelist.enabled"
                } else {
                    "commands.whitelist.disabled"
                };
                self.notify(sender, "whitelist", false, key, vec![]);
            }
            "list" => {
                let names: Vec<String> = self
                    .shared
                    .user_lists
                    .lock()
                    .map(|l| l.whitelist.iter().map(|e| e.name.clone()).collect())
                    .unwrap_or_default();
                let seen = std::fs::read_dir(self.level.dir.join("playerdata"))
                    .map(|d| {
                        d.filter_map(Result::ok)
                            .filter(|e| e.file_name().to_string_lossy().ends_with(".dat"))
                            .count()
                    })
                    .unwrap_or(0);
                self.reply(
                    sender,
                    &t("commands.whitelist.list", vec![s(names.len()), s(seen)]),
                );
                self.reply(sender, &Component::text(join_nice(&names)));
            }
            "reload" => {
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    l.reload_whitelist();
                }
                self.notify(
                    sender,
                    "whitelist",
                    false,
                    "commands.whitelist.reloaded",
                    vec![],
                );
            }
            "add" => {
                let name = args.get(1).ok_or_else(|| usage("whitelist"))?;
                let Some((uuid, real)) = self.profile_for_name(name) else {
                    return Err(err("commands.whitelist.add.failed", vec![s(name)]));
                };
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    l.whitelist.add(WhitelistEntry { uuid, name: real });
                    l.whitelist.save();
                }
                self.notify(
                    sender,
                    "whitelist",
                    false,
                    "commands.whitelist.add.success",
                    vec![s(name)],
                );
            }
            "remove" => {
                let name = args.get(1).ok_or_else(|| usage("whitelist"))?;
                let removed = self.shared.user_lists.lock().ok().and_then(|mut l| {
                    let key = l
                        .whitelist
                        .iter()
                        .find(|e| e.name.eq_ignore_ascii_case(name))
                        .map(|e| java_uuid_string(e.uuid))?;
                    l.whitelist.remove(&key);
                    l.whitelist.save();
                    Some(())
                });
                if removed.is_none() {
                    return Err(err("commands.whitelist.remove.failed", vec![s(name)]));
                }
                self.notify(
                    sender,
                    "whitelist",
                    false,
                    "commands.whitelist.remove.success",
                    vec![s(name)],
                );
            }
            _ => return Err(usage("whitelist")),
        }
        Ok(())
    }

    /// `/op`, `/deop`, `/ban`, `/pardon`, `/ban-ip`, `/pardon-ip`.
    fn cmd_profile(&mut self, sender: Sender, name: &str, args: &[&str]) -> CommandResult {
        let first = *args
            .first()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| usage(name))?;
        let source = self.sender_name(sender);
        match name {
            "op" => {
                let Some((uuid, real)) = self.profile_for_name(first) else {
                    return Err(err("commands.op.failed", vec![s(first)]));
                };
                let level = self.config.op_permission_level;
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    let key = java_uuid_string(uuid);
                    let bypass = l.ops.get(&key).is_some_and(|o| o.bypasses_player_limit);
                    l.ops.add(OpEntry {
                        uuid,
                        name: real,
                        level,
                        bypasses_player_limit: bypass,
                    });
                    l.ops.save();
                }
                self.notify(sender, name, false, "commands.op.success", vec![s(first)]);
            }
            "deop" => {
                let removed = self.shared.user_lists.lock().ok().and_then(|mut l| {
                    let key = l
                        .ops
                        .iter()
                        .find(|o| o.name.eq_ignore_ascii_case(first))
                        .map(|o| java_uuid_string(o.uuid))?;
                    l.ops.remove(&key);
                    l.ops.save();
                    Some(())
                });
                if removed.is_none() {
                    return Err(err("commands.deop.failed", vec![s(first)]));
                }
                self.notify(sender, name, false, "commands.deop.success", vec![s(first)]);
            }
            "ban" => {
                let Some((uuid, real)) = self.profile_for_name(first) else {
                    return Err(err("commands.ban.failed", vec![s(first)]));
                };
                let reason =
                    (args.len() > 1).then(|| Component::from_words(&args[1..]).unformatted());
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    l.banned_players.add(PlayerBan {
                        uuid,
                        name: real,
                        info: new_ban(&source, reason),
                    });
                    l.banned_players.save();
                }
                if let Ok(i) = self.find_player(first) {
                    self.kick(i, "You are banned from this server.");
                }
                self.notify(sender, name, false, "commands.ban.success", vec![s(first)]);
            }
            "pardon" => {
                let removed = self.shared.user_lists.lock().ok().and_then(|mut l| {
                    let key = l
                        .banned_players
                        .iter()
                        .find(|b| b.name.eq_ignore_ascii_case(first))
                        .map(|b| java_uuid_string(b.uuid))?;
                    l.banned_players.remove(&key);
                    l.banned_players.save();
                    Some(())
                });
                if removed.is_none() {
                    return Err(err("commands.unban.failed", vec![s(first)]));
                }
                self.notify(
                    sender,
                    name,
                    false,
                    "commands.unban.success",
                    vec![s(first)],
                );
            }
            "ban-ip" => {
                let ip = if is_ipv4(first) {
                    first.to_owned()
                } else if let Ok(i) = self.find_player(first) {
                    self.players[i].peer.ip().to_string()
                } else {
                    return Err(err("commands.banip.invalid", vec![]));
                };
                let reason =
                    (args.len() > 1).then(|| Component::from_words(&args[1..]).unformatted());
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    l.banned_ips.add(IpBan {
                        ip: ip.clone(),
                        info: new_ban(&source, reason),
                    });
                    l.banned_ips.save();
                }
                let affected: Vec<usize> = (0..self.players.len())
                    .filter(|&i| crate::userlist::ip_key(self.players[i].peer) == ip)
                    .collect();
                let names: Vec<String> = affected
                    .iter()
                    .map(|&i| self.players[i].profile.name.clone())
                    .collect();
                for &i in &affected {
                    self.kick(i, "You have been IP banned.");
                }
                if names.is_empty() {
                    self.notify(sender, name, false, "commands.banip.success", vec![s(&ip)]);
                } else {
                    self.notify(
                        sender,
                        name,
                        false,
                        "commands.banip.success.players",
                        vec![s(&ip), s(join_nice(&names))],
                    );
                }
            }
            "pardon-ip" => {
                if !is_ipv4(first) {
                    return Err(err("commands.unbanip.invalid", vec![]));
                }
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    l.banned_ips.remove(first);
                    l.banned_ips.save();
                }
                self.notify(
                    sender,
                    name,
                    false,
                    "commands.unbanip.success",
                    vec![s(first)],
                );
            }
            _ => unreachable!("dispatched above"),
        }
        Ok(())
    }
}

fn new_ban(source: &str, reason: Option<String>) -> BanInfo {
    BanInfo {
        created: chrono::Local::now().fixed_offset(),
        source: source.to_owned(),
        expires: None,
        reason: reason.unwrap_or_else(|| crate::userlist::DEFAULT_REASON.to_owned()),
    }
}

/// Vanilla `CommandBase.parseCoordinate`: `~` for relative, integer coordinates centered
/// on the block (+0.5) when `center` is set.
fn parse_coordinate(base: f64, input: &str, center: bool) -> Result<f64, CommandError> {
    let relative = input.starts_with('~');
    let rest = if relative { &input[1..] } else { input };
    if relative && rest.is_empty() {
        return Ok(base);
    }
    let mut value: f64 = rest
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite())
        .ok_or_else(|| err("commands.generic.num.invalid", vec![s(rest)]))?;
    if !relative && center && !rest.contains('.') {
        value += 0.5;
    }
    Ok(if relative { base + value } else { value })
}

/// `CommandBase.CoordinateArg`: the resulting coordinate, the number typed (the
/// offset when relative), and whether it was relative (`~`).
struct CoordinateArg {
    value: f64,
    amount: f64,
    relative: bool,
}

/// Vanilla `CommandBase.parseCoordinate(base, input, min, max, center)`; `bounds` of
/// `None` stands for vanilla's `0, 0` (unbounded). The bounds apply to the typed number.
fn parse_coordinate_arg(
    base: f64,
    input: &str,
    bounds: Option<(i32, i32)>,
    center: bool,
) -> Result<CoordinateArg, CommandError> {
    let relative = input.starts_with('~');
    if relative && base.is_nan() {
        return Err(err("commands.generic.num.invalid", vec![s(base)]));
    }
    let mut amount = 0.0;
    if !relative || input.len() > 1 {
        let rest = if relative { &input[1..] } else { input };
        amount += parse_double(rest)?;
        if !rest.contains('.') && !relative && center {
            amount += 0.5;
        }
    }
    if let Some((min, max)) = bounds {
        let f = krol_java::fmt::java_double;
        if amount < f64::from(min) {
            return Err(err(
                "commands.generic.double.tooSmall",
                vec![s(f(amount)), s(min)],
            ));
        }
        if amount > f64::from(max) {
            return Err(err(
                "commands.generic.double.tooBig",
                vec![s(f(amount)), s(max)],
            ));
        }
    }
    Ok(CoordinateArg {
        value: amount + if relative { base } else { 0.0 },
        amount,
        relative,
    })
}

/// Vanilla `CommandBase.parseDouble(text)`: Java's `Double.parseDouble` (surrounding
/// whitespace and a trailing `d`/`f` type suffix allowed), finite values only.
///
/// TODO(M3): Java's hexadecimal floating-point literals.
fn parse_double(text: &str) -> Result<f64, CommandError> {
    let t = text.trim_matches(|c: char| c <= ' ');
    let t = t
        .strip_suffix(['d', 'D', 'f', 'F'])
        .filter(|r| r.ends_with(|c: char| c.is_ascii_digit() || c == '.'))
        .unwrap_or(t);
    t.parse::<f64>()
        .ok()
        .filter(|v| {
            v.is_finite() && !t.contains(|c: char| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
        })
        .ok_or_else(|| err("commands.generic.num.invalid", vec![s(text)]))
}

/// `S08PacketPlayerPosLook.EnumFlags` bits.
const TELEPORT_X: u8 = 0x01;
const TELEPORT_Y: u8 = 0x02;
const TELEPORT_Z: u8 = 0x04;
const TELEPORT_YAW: u8 = 0x08;
const TELEPORT_PITCH: u8 = 0x10;

/// `/effect` without a duration: 30 seconds.
const DEFAULT_EFFECT_SECONDS: i32 = 30;
/// The longest `/effect` duration.
const MAX_EFFECT_SECONDS: i32 = 1_000_000;

/// The horizontal coordinate bounds of `parseBlockPos`.
const COORD_LIMIT: (i32, i32) = (-30_000_000, 30_000_000);

/// Vanilla `CommandBase.parseDouble(base, input, min, max, false)`.
fn parse_coordinate_bounded(
    base: f64,
    input: &str,
    (min, max): (i32, i32),
) -> Result<f64, CommandError> {
    let v = parse_coordinate(base, input, false)?;
    let f = krol_java::fmt::java_double;
    if v < f64::from(min) {
        return Err(err(
            "commands.generic.double.tooSmall",
            vec![s(f(v)), s(min)],
        ));
    }
    if v > f64::from(max) {
        return Err(err("commands.generic.double.tooBig", vec![s(f(v)), s(max)]));
    }
    Ok(v)
}

/// Vanilla `CommandBase.getBlockByText`: a block registry name (`minecraft:` is the
/// default domain; only the domain is case-insensitive).
fn block_by_text(text: &str) -> Result<u16, CommandError> {
    let (domain, path) = match text.find(':') {
        Some(i) if i > 1 => (text[..i].to_lowercase(), &text[i + 1..]),
        Some(i) => ("minecraft".to_owned(), &text[i + 1..]),
        None => ("minecraft".to_owned(), text),
    };
    let found = (domain == "minecraft")
        .then(|| super::block_data::BLOCKS.iter().position(|b| b.0 == path))
        .flatten()
        .and_then(|id| u16::try_from(id).ok());
    found.ok_or_else(|| {
        err(
            "commands.give.block.notFound",
            vec![s(format!("{domain}:{path}"))],
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(join_nice(&["a"]), "a");
        assert_eq!(join_nice(&["a", "b", "c"]), "a, b and c");
        assert_eq!(join_nice::<&str>(&[]), "");
        assert_eq!(COMMANDS.len().div_ceil(7), 9);
        assert!(
            COMMANDS.windows(2).all(|w| w[0].name < w[1].name),
            "sorted for /help"
        );
        assert!(is_ipv4("1.2.3.4") && !is_ipv4("1.2.3.256") && !is_ipv4("nonsense"));
        assert_eq!(
            usage_text("commands.message.usage"),
            Some("/tell <player> <private message ...>")
        );
        assert!(matches!(
            parse_int("9", 0, 3),
            Err(CommandError::Translated("commands.generic.num.tooBig", _))
        ));
    }
}
