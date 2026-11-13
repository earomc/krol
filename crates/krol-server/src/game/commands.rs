//! Commands (vanilla: `ServerCommandManager`, `CommandBase` and the `Command*` classes).
//!
//! Messages are vanilla's en_US strings; parsing follows `CommandBase` (e.g. `parseInt`
//! bounds and their error texts). Commands vanilla has but krol does not implement yet
//! are still listed by `/help` and answer with a krol notice.

use krol_nbt::Tag;
use tracing::{info, warn};

use super::{Game, Outgoing};
use crate::chat::{Arg, Component};
use crate::status::java_uuid_string;

/// Every vanilla 1.8.8 dedicated-server command with its usage, in `/help` order.
pub const COMMANDS: &[(&str, &str)] = &[
    (
        "achievement",
        "/achievement <give|take> <stat_name|*> [player]",
    ),
    ("ban", "/ban <name> [reason ...]"),
    ("ban-ip", "/ban-ip <address|name> [reason ...]"),
    ("banlist", "/banlist [ips|players]"),
    ("blockdata", "/blockdata <x> <y> <z> <dataTag>"),
    (
        "clear",
        "/clear [player] [item] [data] [maxCount] [dataTag]",
    ),
    (
        "clone",
        "/clone <x1> <y1> <z1> <x2> <y2> <z2> <x> <y> <z> [maskMode] [cloneMode]",
    ),
    ("debug", "/debug <start|stop>"),
    ("defaultgamemode", "/defaultgamemode <mode>"),
    ("deop", "/deop <player>"),
    ("difficulty", "/difficulty <new difficulty>"),
    (
        "effect",
        "/effect <player> <effect> [seconds] [amplifier] [hideParticles] OR /effect <player> clear",
    ),
    ("enchant", "/enchant <player> <enchantment ID> [level]"),
    ("entitydata", "/entitydata <entity> <dataTag>"),
    (
        "execute",
        "/execute <entity> <x> <y> <z> <command> OR /execute <entity> <x> <y> <z> detect <x> <y> <z> <block> <data> <command>",
    ),
    (
        "fill",
        "/fill <x1> <y1> <z1> <x2> <y2> <z2> <TileName> [dataValue] [oldBlockHandling] [dataTag]",
    ),
    ("gamemode", "/gamemode <mode> [player]"),
    ("gamerule", "/gamerule <rule name> [value]"),
    ("give", "/give <player> <item> [amount] [data] [dataTag]"),
    ("help", "/help [page|command name]"),
    ("kick", "/kick <player> [reason ...]"),
    ("kill", "/kill [player|entity]"),
    ("list", "/list"),
    ("me", "/me <action ...>"),
    ("op", "/op <player>"),
    ("pardon", "/pardon <name>"),
    ("pardon-ip", "/pardon-ip <address>"),
    (
        "particle",
        "/particle <name> <x> <y> <z> <xd> <yd> <zd> <speed> [count] [mode]",
    ),
    (
        "playsound",
        "/playsound <sound> <player> [x] [y] [z] [volume] [pitch] [minimumVolume]",
    ),
    ("replaceitem", "/replaceitem <entity|block> ..."),
    ("save-all", "/save-all"),
    ("save-off", "/save-off"),
    ("save-on", "/save-on"),
    ("say", "/say <message ...>"),
    ("scoreboard", "/scoreboard <objectives|players|teams> ..."),
    ("seed", "/seed"),
    (
        "setblock",
        "/setblock <x> <y> <z> <TileName> [dataValue] [oldBlockHandling] [dataTag]",
    ),
    ("setidletimeout", "/setidletimeout <Minutes until kick>"),
    ("setworldspawn", "/setworldspawn [<x> <y> <z>]"),
    ("spawnpoint", "/spawnpoint [player] [<x> <y> <z>]"),
    (
        "spreadplayers",
        "/spreadplayers <x> <z> <spreadDistance> <maxRange> <respectTeams true|false> <player ...>",
    ),
    ("stats", "/stats <entity|block> ..."),
    ("stop", "/stop"),
    ("summon", "/summon <EntityName> [x] [y] [z] [dataTag]"),
    ("tell", "/tell <player> <private message ...>"),
    ("tellraw", "/tellraw <player> <raw json message>"),
    ("testfor", "/testfor <player> [dataTag]"),
    (
        "testforblock",
        "/testforblock <x> <y> <z> <TileName> [dataValue] [dataTag]",
    ),
    (
        "testforblocks",
        "/testforblocks <x1> <y1> <z1> <x2> <y2> <z2> <x> <y> <z> [mode]",
    ),
    ("time", "/time <set|add|query> <value>"),
    (
        "title",
        "/title <player> <title|subtitle|clear|reset|times> ...",
    ),
    ("toggledownfall", "/toggledownfall"),
    (
        "tp",
        "/tp [target player] <destination player> OR /tp [target player] <x> <y> <z> [<y-rot> <x-rot>]",
    ),
    ("trigger", "/trigger <objective> <add|set> <value>"),
    (
        "weather",
        "/weather <clear|rain|thunder> [duration in seconds]",
    ),
    ("whitelist", "/whitelist <on|off|list|add|remove|reload>"),
    (
        "worldborder",
        "/worldborder <set|center|damage|warning|get|add> ...",
    ),
    ("xp", "/xp <amount> [player] OR /xp <amount>L [player]"),
];

/// Aliases vanilla registers (`ICommand.getCommandAliases`).
const ALIASES: &[(&str, &str)] = &[("?", "help"), ("msg", "tell"), ("w", "tell"), ("tp", "tp")];

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
    /// `WrongUsageException`.
    Usage(&'static str),
    /// A translated error.
    Translated(&'static str, Vec<String>),
}

type CommandResult = Result<(), CommandError>;

fn usage(name: &str) -> &'static str {
    COMMANDS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or("", |(_, u)| u)
}

fn err(key: &'static str, args: &[&str]) -> CommandError {
    CommandError::Translated(key, args.iter().map(|s| (*s).to_owned()).collect())
}

/// en_US strings for command messages.
fn translate(key: &str) -> &'static str {
    match key {
        "commands.generic.usage" => "Usage: %s",
        "commands.generic.notFound" => "Unknown command. Try /help for a list of commands",
        "commands.generic.num.invalid" => "'%s' is not a valid number",
        "commands.generic.num.tooBig" => {
            "The number you have entered (%s) is too big, it must be at most %s"
        }
        "commands.generic.num.tooSmall" => {
            "The number you have entered (%s) is too small, it must be at least %s"
        }
        "commands.generic.player.notFound" => "That player cannot be found",
        "commands.generic.player.unspecified" => {
            "You must specify which player you wish to perform this action on."
        }
        "commands.generic.entity.invalidUuid" => "The entity UUID provided is in an invalid format",
        "commands.generic.entity.notFound" => "Entity '%s' cannot be found",
        "commands.help.header" => "--- Showing help page %s of %s (/help <page>) ---",
        "commands.players.list" => "There are %s/%s players online:",
        "commands.time.set" => "Set the time to %s",
        "commands.time.added" => "Added %s to the time",
        "commands.time.query" => "Time is %s",
        "commands.weather.clear" => "Changing to clear weather",
        "commands.weather.rain" => "Changing to rainy weather",
        "commands.weather.thunder" => "Changing to rain and thunder",
        "commands.downfall.success" => "Toggled downfall",
        "commands.difficulty.success" => "Set game difficulty to %s",
        "commands.defaultgamemode.success" => "The world's default game mode is now %s",
        "commands.seed.success" => "Seed: %s",
        "commands.gamerule.success" => "Game rule has been updated",
        "commands.gamerule.norule" => "No game rule called '%s' is available",
        "commands.save.disabled" => "Turned off world auto-saving",
        "commands.save.enabled" => "Turned on world auto-saving",
        "commands.save-off.alreadyOff" => "Saving is already turned off",
        "commands.save-on.alreadyOn" => "Saving is already turned on",
        "commands.save.start" => "Saving...",
        "commands.save.success" => "Saved the world",
        "commands.banlist.players" => "There are %s total banned players:",
        "commands.banlist.ips" => "There are %s total banned IP addresses:",
        "commands.whitelist.list" => "There are %s (out of %s seen) whitelisted players:",
        "commands.whitelist.enabled" => "Turned on the whitelist",
        "commands.whitelist.disabled" => "Turned off the whitelist",
        "commands.whitelist.reloaded" => "Reloaded the whitelist",
        "commands.setidletimeout.success" => "Successfully set the idle timeout to %s minutes.",
        "commands.setworldspawn.success" => "Set the world spawn point to (%s, %s, %s)",
        "commands.kick.success" => "Kicked %s from the game",
        "commands.kick.success.reason" => "Kicked %s from the game: '%s'",
        "commands.stop.start" => "Stopping the server",
        "options.difficulty.peaceful" => "Peaceful",
        "options.difficulty.easy" => "Easy",
        "options.difficulty.normal" => "Normal",
        "options.difficulty.hard" => "Hard",
        "gameMode.survival" => "Survival Mode",
        "gameMode.creative" => "Creative Mode",
        "gameMode.adventure" => "Adventure Mode",
        "gameMode.spectator" => "Spectator Mode",
        _ => "",
    }
}

fn render(key: &str, args: &[String]) -> String {
    let template = translate(key);
    if template.is_empty() {
        return key.to_owned();
    }
    let mut out = String::new();
    let mut rest = template;
    let mut args = args.iter();
    while let Some(i) = rest.find("%s") {
        out.push_str(&rest[..i]);
        out.push_str(args.next().map_or("", String::as_str));
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// Vanilla `CommandBase.parseInt(input, min, max)`.
fn parse_int(input: &str, min: i32, max: i32) -> Result<i32, CommandError> {
    let v = crate::config::parse_java_int(input)
        .ok_or_else(|| err("commands.generic.num.invalid", &[input]))?;
    if v < min {
        return Err(err(
            "commands.generic.num.tooSmall",
            &[&v.to_string(), &min.to_string()],
        ));
    }
    if v > max {
        return Err(err(
            "commands.generic.num.tooBig",
            &[&v.to_string(), &max.to_string()],
        ));
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

fn difficulty_name(d: i32) -> &'static str {
    match d {
        0 => "options.difficulty.peaceful",
        1 => "options.difficulty.easy",
        2 => "options.difficulty.normal",
        _ => "options.difficulty.hard",
    }
}

fn game_mode_name(m: i32) -> &'static str {
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

impl Game {
    /// Sends `message` to the sender: logged for the console, chat for players.
    fn reply(&self, sender: Sender, message: &Component) {
        match sender {
            Sender::Console => info!("{}", message.unformatted()),
            Sender::Player(i) => {
                self.players[i].send(krol_protocol::packets::play::ClientboundPlay::Chat {
                    json: message.to_json_string(),
                    position: 0,
                })
            }
        }
    }

    fn reply_key(&self, sender: Sender, key: &'static str, args: &[&str]) {
        self.reply(
            sender,
            &Component::text(render(
                key,
                &args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            )),
        );
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
            Sender::Player(i) => Self::display_name(&self.players[i].profile),
        }
    }

    /// Vanilla `getPlayer` by name (case-insensitive).
    fn find_player(&self, name: &str) -> Result<usize, CommandError> {
        self.players
            .iter()
            .position(|p| p.profile.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| err("commands.generic.player.notFound", &[]))
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
        if !COMMANDS.iter().any(|(n, _)| *n == name) {
            self.reply_error(sender, &err("commands.generic.notFound", &[]));
            return;
        }
        if let Err(e) = self.run_command(sender, name, &args) {
            self.reply_error(sender, &e);
        }
    }

    fn reply_error(&self, sender: Sender, e: &CommandError) {
        let text = match e {
            CommandError::Usage(u) => render("commands.generic.usage", &[(*u).to_owned()]),
            CommandError::Translated(key, args) => render(key, args),
        };
        self.reply(sender, &Component::text(text).color("red"));
    }

    fn run_command(&mut self, sender: Sender, name: &str, args: &[&str]) -> CommandResult {
        match name {
            "help" => self.cmd_help(sender, args),
            "list" => {
                let names: Vec<String> = self
                    .players
                    .iter()
                    .map(|p| {
                        if args
                            .first()
                            .is_some_and(|a| a.eq_ignore_ascii_case("uuids"))
                        {
                            format!("{} ({})", p.profile.name, java_uuid_string(p.profile.id))
                        } else {
                            p.profile.name.clone()
                        }
                    })
                    .collect();
                self.reply_key(
                    sender,
                    "commands.players.list",
                    &[
                        &self.players.len().to_string(),
                        &self.config.max_players.to_string(),
                    ],
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
                    return Err(CommandError::Usage(usage(name)));
                }
                let key = if name == "say" {
                    "chat.type.announcement"
                } else {
                    "chat.type.emote"
                };
                let msg = Component::translate(
                    key,
                    vec![
                        Arg::Component(self.sender_component(sender)),
                        Arg::Component(Component::text(args.join(" "))),
                    ],
                );
                self.broadcast_chat(&msg, None);
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
                self.reply_key(sender, "commands.downfall.success", &[]);
                Ok(())
            }
            "difficulty" => {
                let a = args.first().ok_or(CommandError::Usage(usage(name)))?;
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
                    p.send(
                        krol_protocol::packets::play::ClientboundPlay::ServerDifficulty {
                            difficulty: d as u8,
                        },
                    );
                }
                let shown = render(difficulty_name(d), &[]);
                self.reply_key(sender, "commands.difficulty.success", &[&shown]);
                Ok(())
            }
            "defaultgamemode" => {
                let a = args.first().ok_or(CommandError::Usage(usage(name)))?;
                let m = parse_game_mode(a)?;
                self.config.gamemode = m;
                self.level.data.set_game_type(m);
                // TODO: with force-gamemode, vanilla also changes every player's mode.
                let shown = render(game_mode_name(m), &[]);
                self.reply_key(sender, "commands.defaultgamemode.success", &[&shown]);
                Ok(())
            }
            "seed" => {
                let seed = self.level.data.seed().to_string();
                self.reply_key(sender, "commands.seed.success", &[&seed]);
                Ok(())
            }
            "gamerule" => self.cmd_gamerule(sender, args),
            "save-off" | "save-on" => {
                let enable = name == "save-on";
                if self.saving_enabled == enable {
                    let key = if enable {
                        "commands.save-on.alreadyOn"
                    } else {
                        "commands.save-off.alreadyOff"
                    };
                    return Err(err(key, &[]));
                }
                self.saving_enabled = enable;
                let key = if enable {
                    "commands.save.enabled"
                } else {
                    "commands.save.disabled"
                };
                self.reply_key(sender, key, &[]);
                Ok(())
            }
            "save-all" => {
                self.reply_key(sender, "commands.save.start", &[]);
                let dir = self.level.dir.clone();
                for p in &mut self.players {
                    p.save(&dir);
                }
                self.level.save_quietly();
                self.reply_key(sender, "commands.save.success", &[]);
                Ok(())
            }
            "banlist" => {
                let ips = args.first().is_some_and(|a| a.eq_ignore_ascii_case("ips"));
                let lists = self.shared.user_lists.lock().map_err(|_| err("", &[]))?;
                let names: Vec<String> = if ips {
                    lists.banned_ips.iter().map(|b| b.ip.clone()).collect()
                } else {
                    lists
                        .banned_players
                        .iter()
                        .map(|b| b.name.clone())
                        .collect()
                };
                drop(lists);
                let key = if ips {
                    "commands.banlist.ips"
                } else {
                    "commands.banlist.players"
                };
                self.reply_key(sender, key, &[&names.len().to_string()]);
                self.reply(sender, &Component::text(join_nice(&names)));
                Ok(())
            }
            "whitelist" => self.cmd_whitelist(sender, args),
            "setidletimeout" => {
                let a = args.first().ok_or(CommandError::Usage(usage(name)))?;
                let minutes = parse_int(a, 0, i32::MAX)?;
                self.props.set("player-idle-timeout", minutes);
                self.props.save();
                self.reply_key(
                    sender,
                    "commands.setidletimeout.success",
                    &[&minutes.to_string()],
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
                        return Err(err("commands.generic.player.unspecified", &[]));
                    }
                    _ => return Err(CommandError::Usage(usage(name))),
                };
                self.level.data.set_spawn_x(pos.0);
                self.level.data.set_spawn_y(pos.1);
                self.level.data.set_spawn_z(pos.2);
                for p in &self.players {
                    p.send(krol_protocol::packets::play::ClientboundPlay::SpawnPosition { pos });
                }
                self.reply_key(
                    sender,
                    "commands.setworldspawn.success",
                    &[&pos.0.to_string(), &pos.1.to_string(), &pos.2.to_string()],
                );
                Ok(())
            }
            "kick" => {
                let target = args.first().ok_or(CommandError::Usage(usage(name)))?;
                let i = self.find_player(target)?;
                let player_name = self.players[i].profile.name.clone();
                let reason = args[1..].join(" ");
                if reason.is_empty() {
                    self.kick(i, "Kicked by an operator.");
                    self.reply_key(sender, "commands.kick.success", &[&player_name]);
                } else {
                    self.kick(i, &reason);
                    self.reply_key(
                        sender,
                        "commands.kick.success.reason",
                        &[&player_name, &reason],
                    );
                }
                Ok(())
            }
            "tp" => {
                if args.is_empty() {
                    return Err(CommandError::Usage(usage(name)));
                }
                // Vanilla resolves the target with getEntity: a player name, else a UUID.
                let target = args[0];
                if self.find_player(target).is_err() {
                    return Err(if parse_uuid_like_java(target) {
                        err("commands.generic.entity.notFound", &[target])
                    } else {
                        err("commands.generic.entity.invalidUuid", &[])
                    });
                }
                self.not_implemented(sender, name)
            }
            "tell" | "xp" | "gamemode" => {
                let player_arg = match name {
                    "tell" => args.first(),
                    _ => args.get(1),
                };
                if args.is_empty() {
                    return Err(CommandError::Usage(usage(name)));
                }
                match player_arg {
                    Some(p) => {
                        self.find_player(p)?;
                    }
                    None if sender == Sender::Console => {
                        return Err(err("commands.generic.player.unspecified", &[]));
                    }
                    None => {}
                }
                self.not_implemented(sender, name)
            }
            "stop" => {
                self.reply_key(sender, "commands.stop.start", &[]);
                self.stopping = true;
                Ok(())
            }
            _ => self.not_implemented(sender, name),
        }
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
        let pages = COMMANDS.len().div_ceil(PER_PAGE);
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
                    return match COMMANDS.iter().find(|(n, _)| *n == name) {
                        Some((_, u)) => Err(CommandError::Usage(u)),
                        None => Err(err("commands.generic.notFound", &[])),
                    };
                }
            },
        };
        let header = Component::text(render(
            "commands.help.header",
            &[page.to_string(), pages.to_string()],
        ))
        .color("dark_green");
        self.reply(sender, &header);
        for (_, u) in COMMANDS.iter().skip((page - 1) * PER_PAGE).take(PER_PAGE) {
            self.reply(sender, &Component::text(*u));
        }
        Ok(())
    }

    fn cmd_time(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.len() < 2 {
            return Err(CommandError::Usage(usage("time")));
        }
        match args[0] {
            "set" => {
                let v = match args[1] {
                    "day" => 1000,
                    "night" => 13000,
                    a => parse_int(a, 0, i32::MAX)?,
                };
                self.level.data.set_day_time(i64::from(v));
                self.reply_key(sender, "commands.time.set", &[&v.to_string()]);
            }
            "add" => {
                let v = parse_int(args[1], 0, i32::MAX)?;
                let t = self.level.data.day_time() + i64::from(v);
                self.level.data.set_day_time(t);
                self.reply_key(sender, "commands.time.added", &[&v.to_string()]);
            }
            "query" => {
                let v = match args[1] {
                    "daytime" => self.level.data.day_time() % 2_147_483_647,
                    "gametime" => self.level.data.time() % 2_147_483_647,
                    _ => return Err(CommandError::Usage(usage("time"))),
                };
                self.reply_key(sender, "commands.time.query", &[&v.to_string()]);
            }
            _ => return Err(CommandError::Usage(usage("time"))),
        }
        Ok(())
    }

    fn cmd_weather(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.is_empty() || args.len() > 2 {
            return Err(CommandError::Usage(usage("weather")));
        }
        let ticks = match args.get(1) {
            Some(a) => parse_int(a, 1, 1_000_000)? * 20,
            None => (300 + self.weather_rand.next_int_bounded(600)) * 20,
        };
        let data = &mut self.level.data;
        let (key, rain, thunder) = match args[0].to_ascii_lowercase().as_str() {
            "clear" => ("commands.weather.clear", false, false),
            "rain" => ("commands.weather.rain", true, false),
            "thunder" => ("commands.weather.thunder", true, true),
            _ => return Err(CommandError::Usage(usage("weather"))),
        };
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
        self.reply_key(sender, key, &[]);
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
                None => return Err(err("commands.gamerule.norule", &[args[0]])),
            },
            _ => {
                let value = args[1..].join(" ");
                let mut compound = self
                    .level
                    .data
                    .data
                    .get_compound("GameRules")
                    .cloned()
                    .unwrap_or_default();
                compound.insert(args[0], Tag::String(value));
                self.level
                    .data
                    .data
                    .insert("GameRules", Tag::Compound(compound));
                self.reply_key(sender, "commands.gamerule.success", &[]);
            }
        }
        Ok(())
    }

    /// All game rules (defaults overlaid with `level.dat`), sorted by name like vanilla's
    /// `TreeMap`.
    fn game_rules(&self) -> Vec<(String, String)> {
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
        let a = args
            .first()
            .ok_or(CommandError::Usage(usage("whitelist")))?;
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
                self.reply_key(sender, key, &[]);
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
                self.reply_key(
                    sender,
                    "commands.whitelist.list",
                    &[&names.len().to_string(), &seen.to_string()],
                );
                self.reply(sender, &Component::text(join_nice(&names)));
            }
            "reload" => {
                if let Ok(mut l) = self.shared.user_lists.lock() {
                    l.reload_whitelist();
                }
                self.reply_key(sender, "commands.whitelist.reloaded", &[]);
            }
            "add" | "remove" => return self.not_implemented(sender, "whitelist add/remove"),
            _ => return Err(CommandError::Usage(usage("whitelist"))),
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn sender_is_console(&self, sender: Sender) -> bool {
        let _ = self.sender_name(sender);
        sender == Sender::Console
    }
}

/// Whether `s` would parse with Java's `UUID.fromString`.
fn parse_uuid_like_java(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 16 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

#[allow(dead_code)]
fn _outgoing_used(_: Outgoing) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(join_nice(&["a"]), "a");
        assert_eq!(join_nice(&["a", "b", "c"]), "a, b and c");
        assert_eq!(join_nice::<&str>(&[]), "");
        assert_eq!(COMMANDS.len().div_ceil(7), 9);
        assert!(matches!(
            parse_int("9", 0, 3),
            Err(CommandError::Translated("commands.generic.num.tooBig", _))
        ));
        assert_eq!(
            render("commands.generic.num.tooBig", &["9".into(), "3".into()]),
            "The number you have entered (9) is too big, it must be at most 3"
        );
        assert!(
            COMMANDS.windows(2).all(|w| w[0].0 < w[1].0),
            "sorted for /help"
        );
    }
}
