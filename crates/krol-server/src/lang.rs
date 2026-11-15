//! The en_US strings the server renders itself (console log lines). Clients receive
//! translation keys and render them in their own language.

/// The en_US text for `key`, or `None` if krol does not know it.
pub fn en_us(key: &str) -> Option<&'static str> {
    Some(match key {
        // Chat and connection messages.
        "chat.type.text" => "<%s> %s",
        "chat.type.emote" => "* %s %s",
        "chat.type.announcement" => "[%s] %s",
        "chat.type.admin" => "[%s: %s]",
        "multiplayer.player.joined" => "%s joined the game",
        "multiplayer.player.left" => "%s left the game",
        "death.attack.generic" => "%1$s died",
        "death.attack.fall" => "%1$s hit the ground too hard",
        "death.attack.drown" => "%1$s drowned",
        "death.attack.onFire" => "%1$s burned to death",
        "death.attack.inFire" => "%1$s went up in flames",
        "death.attack.lava" => "%1$s tried to swim in lava",
        "death.attack.magic" => "%1$s was killed by magic",
        "death.attack.wither" => "%1$s withered away",
        "death.attack.cactus" => "%1$s was pricked to death",
        "death.attack.inWall" => "%1$s suffocated in a wall",
        "death.attack.explosion" => "%1$s blew up",
        "death.attack.outOfWorld" => "%1$s fell out of the world",
        "commands.kill.successful" => "Killed %s",
        "commands.clear.success" => "Cleared the inventory of %s, removing %d items",
        "commands.clear.testing" => "%s has %d items that match the criteria",
        "commands.clear.failure" => "Could not clear the inventory of %s, no items to remove",
        "commands.spawnpoint.success" => "Set %s's spawn point to (%d, %d, %d)",
        "commands.give.item.notFound" => "There is no such item with name %d",
        "death.attack.explosion.player" => "%1$s was blown up by %2$s",
        "death.fell.accident.generic" => "%1$s fell from a high place",
        "death.fell.accident.ladder" => "%1$s fell off a ladder",
        "death.fell.accident.vines" => "%1$s fell off some vines",
        "death.fell.accident.water" => "%1$s fell out of the water",
        "disconnect.timeout" => "Timed out",
        "disconnect.endOfStream" => "End of stream",
        "disconnect.genericReason" => "%s",
        "disconnect.spam" => "Kicked for spamming",
        "multiplayer.disconnect.server_shutdown" => "Server closed",
        "gameMode.changed" => "Your game mode has been updated",
        "build.tooHigh" => "Height limit for building is %s blocks",
        "gameMode.survival" => "Survival Mode",
        "gameMode.creative" => "Creative Mode",
        "gameMode.adventure" => "Adventure Mode",
        "gameMode.spectator" => "Spectator Mode",
        "options.difficulty.peaceful" => "Peaceful",
        "options.difficulty.easy" => "Easy",
        "options.difficulty.normal" => "Normal",
        "options.difficulty.hard" => "Hard",
        // Generic command messages.
        "commands.generic.usage" => "Usage: %s",
        "commands.generic.notFound" => "Unknown command. Try /help for a list of commands",
        "commands.generic.permission" => "You do not have permission to use this command",
        "commands.generic.num.invalid" => "'%s' is not a valid number",
        "commands.generic.num.tooBig" => {
            "The number you have entered (%s) is too big, it must be at most %s"
        }
        "commands.generic.num.tooSmall" => {
            "The number you have entered (%s) is too small, it must be at least %s"
        }
        "commands.generic.double.tooBig" => {
            "The number you have entered (%s) is too big, it must be at most %s"
        }
        "commands.generic.double.tooSmall" => {
            "The number you have entered (%s) is too small, it must be at least %s"
        }
        "commands.generic.player.notFound" => "That player cannot be found",
        "commands.generic.player.unspecified" => {
            "You must specify which player you wish to perform this action on."
        }
        "commands.generic.entity.invalidUuid" => "The entity UUID provided is in an invalid format",
        "commands.generic.entity.notFound" => "Entity '%s' cannot be found",
        // Command results.
        "commands.help.header" => "--- Showing help page %s of %s (/help <page>) ---",
        "commands.help.footer" => {
            "Tip: Use the <tab> key while typing a command to auto-complete the command or its arguments"
        }
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
        "commands.whitelist.add.success" => "Added %s to the whitelist",
        "commands.whitelist.add.failed" => "Could not add %s to the whitelist",
        "commands.whitelist.remove.success" => "Removed %s from the whitelist",
        "commands.whitelist.remove.failed" => "Could not remove %s from the whitelist",
        "commands.op.success" => "Opped %s",
        "commands.op.failed" => "Could not op %s",
        "commands.deop.success" => "De-opped %s",
        "commands.deop.failed" => "Could not de-op %s",
        "commands.ban.success" => "Banned player %s",
        "commands.ban.failed" => "Could not ban player %s",
        "commands.unban.success" => "Unbanned player %s",
        "commands.unban.failed" => "Could not unban player %s",
        "commands.banip.success" => "Banned IP address %s",
        "commands.banip.success.players" => "Banned IP address %s belonging to %s",
        "commands.banip.invalid" => {
            "You have entered an invalid IP address or a player that is not online"
        }
        "commands.unbanip.success" => "Unbanned IP address %s",
        "commands.unbanip.invalid" => "You have entered an invalid IP address",
        "commands.setidletimeout.success" => "Successfully set the idle timeout to %s minutes.",
        "commands.setworldspawn.success" => "Set the world spawn point to (%s, %s, %s)",
        "commands.kick.success" => "Kicked %s from the game",
        "commands.kick.success.reason" => "Kicked %s from the game: '%s'",
        "commands.stop.start" => "Stopping the server",
        "commands.message.sameTarget" => "You can't send a private message to yourself!",
        "commands.message.display.outgoing" => "You whisper to %s: %s",
        "commands.message.display.incoming" => "%s whispers to you: %s",
        "commands.gamemode.success.self" => "Set own game mode to %s",
        "commands.gamemode.success.other" => "Set %s's game mode to %s",
        "commands.xp.success" => "Given %s experience to %s",
        "commands.xp.success.levels" => "Given %s levels to %s",
        "commands.xp.success.negative.levels" => "Taken %s levels from %s",
        "commands.tp.success.coordinates" => "Teleported %s to %s, %s, %s",
        "commands.setblock.success" => "Block placed",
        // Status effect names.
        "potion.moveSpeed" => "Speed",
        "potion.moveSlowdown" => "Slowness",
        "potion.digSpeed" => "Haste",
        "potion.digSlowDown" => "Mining Fatigue",
        "potion.damageBoost" => "Strength",
        "potion.heal" => "Instant Health",
        "potion.harm" => "Instant Damage",
        "potion.jump" => "Jump Boost",
        "potion.confusion" => "Nausea",
        "potion.regeneration" => "Regeneration",
        "potion.resistance" => "Resistance",
        "potion.fireResistance" => "Fire Resistance",
        "potion.waterBreathing" => "Water Breathing",
        "potion.invisibility" => "Invisibility",
        "potion.blindness" => "Blindness",
        "potion.nightVision" => "Night Vision",
        "potion.hunger" => "Hunger",
        "potion.weakness" => "Weakness",
        "potion.poison" => "Poison",
        "potion.wither" => "Wither",
        "potion.healthBoost" => "Health Boost",
        "potion.absorption" => "Absorption",
        "potion.saturation" => "Saturation",
        "commands.effect.success" => "Given %1$s (ID %2$d) * %3$d to %4$s for %5$d seconds",
        "commands.enchant.notFound" => "There is no such enchantment with ID %d",
        "commands.enchant.noItem" => "The target doesn't hold an item",
        "commands.enchant.cantEnchant" => {
            "The selected enchantment can't be added to the target item"
        }
        "commands.enchant.cantCombine" => "%1$s can't be combined with %2$s",
        "commands.enchant.success" => "Enchanting succeeded",
        "commands.effect.success.removed" => "Took %1$s from %2$s",
        "commands.effect.success.removed.all" => "Took all effects from %s",
        "commands.effect.failure.notActive" => {
            "Couldn't take %1$s from %2$s as they do not have the effect"
        }
        "commands.effect.failure.notActive.all" => {
            "Couldn't take any effects from %s as they do not have any"
        }
        "commands.effect.notFound" => "There is no such mob effect with ID %d",
        "commands.setblock.noChange" => "The block couldn't be placed",
        "commands.setblock.outOfWorld" => "Cannot place block outside of the world",
        "commands.give.block.notFound" => "There is no such block with name %s",
        _ => return crate::game::commands::usage_text(key),
    })
}

/// Formats `key` with `args` like Java's `String.format` with `%s` placeholders (and
/// `%1$s`-style positional ones); unknown keys render as the key itself, as vanilla
/// does.
pub fn format(key: &str, args: &[String]) -> String {
    let Some(template) = en_us(key) else {
        return key.to_owned();
    };
    // ChatComponentTranslation: `%[<index>$]<letter>` takes an argument whatever the
    // letter (`%s`, `%d`), `%%` is a percent sign.
    let mut out = String::new();
    let mut next = 0;
    let mut rest = template;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(r) = after.strip_prefix('%') {
            out.push('%');
            rest = r;
            continue;
        }
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let (index, spec) = match after[digits..].strip_prefix('$') {
            Some(spec) if digits > 0 => (after[..digits].parse::<usize>().ok(), spec),
            _ => (None, after),
        };
        match spec.chars().next() {
            Some(c) if c.is_ascii_alphabetic() => {
                let k = match index {
                    Some(n) => n.saturating_sub(1),
                    None => {
                        next += 1;
                        next - 1
                    }
                };
                out.push_str(args.get(k).map_or("", String::as_str));
                rest = &spec[c.len_utf8()..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format("chat.type.text", &["a".into(), "b".into()]), "<a> b");
        assert_eq!(
            format("commands.time.usage", &[]),
            "/time <set|add|query> <value>"
        );
        assert_eq!(format("no.such.key", &[]), "no.such.key");
    }
}
