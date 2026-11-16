//! `/scoreboard` (`CommandScoreboard`) and `/trigger` (`CommandTrigger`).

use super::super::scoreboard::{
    self as sb, DISPLAY_SLOTS, MAX_DISPLAY, MAX_HOLDER, MAX_NAME, RESET, TRIGGER, VISIBILITIES,
};
use super::{
    CommandError, CommandResult, Component, Game, Sender, Target, data_tag, err, join_nice,
    parse_int, s, selector, t, usage,
};

/// Java's `String.length` (UTF-16 units).
fn java_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn usage_of(key: &'static str) -> CommandError {
    CommandError::Usage(key)
}

/// The team options `setTeamOption` accepts.
const TEAM_OPTIONS: [&str; 5] = [
    "color",
    "friendlyfire",
    "seeFriendlyInvisibles",
    "nametagVisibility",
    "deathMessageVisibility",
];

impl Game {
    /// `CommandBase.getEntityName`: a player's name, another entity's UUID, or the text
    /// itself unless it is a selector.
    fn holder_name(&self, sender: Sender, text: &str) -> Result<String, CommandError> {
        if let Ok(i) = self.get_player(sender, text) {
            return Ok(self.players[i].profile.name.clone());
        }
        match self.get_entity(sender, text, false) {
            Ok(target) => Ok(crate::status::java_uuid_string(self.target_uuid(target))),
            Err(e) if selector::is_selector(text) => Err(e),
            Err(_) => Ok(text.to_owned()),
        }
    }

    /// `CommandBase.getPlayerName`: a player's name, or the text unless a selector.
    fn player_name_arg(&self, sender: Sender, text: &str) -> Result<String, CommandError> {
        match self.get_player(sender, text) {
            Ok(i) => Ok(self.players[i].profile.name.clone()),
            Err(e) if selector::is_selector(text) => Err(e),
            Err(_) => Ok(text.to_owned()),
        }
    }

    /// `CommandScoreboard.getObjective`.
    fn objective_arg(&self, name: &str, writable: bool) -> Result<String, CommandError> {
        let Some(o) = self.scoreboard.objectives.get(name) else {
            return Err(err("commands.scoreboard.objectiveNotFound", vec![s(name)]));
        };
        if writable && sb::read_only(&o.criteria) {
            return Err(err("commands.scoreboard.objectiveReadOnly", vec![s(name)]));
        }
        Ok(name.to_owned())
    }

    /// `CommandScoreboard.getTeam`.
    fn team_arg(&self, name: &str) -> Result<String, CommandError> {
        if self.scoreboard.teams.get(name).is_none() {
            return Err(err("commands.scoreboard.teamNotFound", vec![s(name)]));
        }
        Ok(name.to_owned())
    }

    fn holder_length(name: &str) -> CommandResult {
        if java_len(name) > MAX_HOLDER {
            return Err(err(
                "commands.scoreboard.players.name.tooLong",
                vec![s(name), s(MAX_HOLDER)],
            ));
        }
        Ok(())
    }

    pub(super) fn cmd_scoreboard(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if self.scoreboard_wildcard(sender, args)? {
            return Ok(());
        }
        self.scoreboard_once(sender, args)
    }

    /// `func_175780_b`: a `*` in a player position runs the command for every score
    /// holder; failures are reported one by one.
    fn scoreboard_wildcard(&mut self, sender: Sender, args: &[&str]) -> Result<bool, CommandError> {
        let mut wildcard = None;
        for (k, a) in args.iter().enumerate() {
            if *a == "*" && super::scoreboard_is_username_index(args, k) {
                if wildcard.is_some() {
                    return Err(err("commands.scoreboard.noMultiWildcard", vec![]));
                }
                wildcard = Some(k);
            }
        }
        let Some(k) = wildcard else {
            return Ok(false);
        };
        let holders: Vec<String> = self
            .scoreboard
            .scores
            .iter()
            .map(|(h, _)| h.clone())
            .collect();
        let mut succeeded = 0;
        for holder in holders {
            let mut expanded: Vec<&str> = args.to_vec();
            expanded[k] = &holder;
            match self.cmd_scoreboard(sender, &expanded) {
                Ok(()) => succeeded += 1,
                Err(e) => {
                    let message = match e {
                        CommandError::Usage(key) => t(key, vec![]),
                        CommandError::UsageWith(key, a) | CommandError::Translated(key, a) => {
                            t(key, a)
                        }
                        CommandError::Exception => return Err(CommandError::Exception),
                    };
                    self.reply(sender, &message.color("red"));
                }
            }
        }
        if succeeded == 0 {
            return Err(usage_of("commands.scoreboard.allMatchesFailed"));
        }
        Ok(true)
    }

    fn scoreboard_once(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let Some(&group) = args.first() else {
            return Err(usage("scoreboard"));
        };
        let n = args.len();
        let sub = args.get(1).copied().unwrap_or_default();
        let is = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        if is(group, "objectives") {
            if n == 1 {
                return Err(usage_of("commands.scoreboard.objectives.usage"));
            }
            if is(sub, "list") {
                self.sb_list_objectives(sender)
            } else if is(sub, "add") {
                if n < 4 {
                    return Err(usage_of("commands.scoreboard.objectives.add.usage"));
                }
                self.sb_cmd_add_objective(sender, args)
            } else if is(sub, "remove") {
                if n != 3 {
                    return Err(usage_of("commands.scoreboard.objectives.remove.usage"));
                }
                let name = self.objective_arg(args[2], false)?;
                self.sb_remove_objective(&name);
                self.notify(
                    sender,
                    "scoreboard",
                    false,
                    "commands.scoreboard.objectives.remove.success",
                    vec![s(args[2])],
                );
                Ok(())
            } else if is(sub, "setdisplay") {
                if n != 3 && n != 4 {
                    return Err(usage_of("commands.scoreboard.objectives.setdisplay.usage"));
                }
                self.sb_cmd_set_display(sender, args)
            } else {
                Err(usage_of("commands.scoreboard.objectives.usage"))
            }
        } else if is(group, "players") {
            if n == 1 {
                return Err(usage_of("commands.scoreboard.players.usage"));
            }
            if is(sub, "list") {
                if n > 3 {
                    return Err(usage_of("commands.scoreboard.players.list.usage"));
                }
                self.sb_list_players(sender, args)
            } else if is(sub, "add") || is(sub, "remove") || is(sub, "set") {
                if n < 5 {
                    return Err(usage_of(if is(sub, "add") {
                        "commands.scoreboard.players.add.usage"
                    } else if is(sub, "remove") {
                        "commands.scoreboard.players.remove.usage"
                    } else {
                        "commands.scoreboard.players.set.usage"
                    }));
                }
                self.sb_cmd_set_player(sender, args)
            } else if is(sub, "reset") {
                if n != 3 && n != 4 {
                    return Err(usage_of("commands.scoreboard.players.reset.usage"));
                }
                self.sb_cmd_reset(sender, args)
            } else if is(sub, "enable") {
                if n != 4 {
                    return Err(usage_of("commands.scoreboard.players.enable.usage"));
                }
                self.sb_cmd_enable(sender, args)
            } else if is(sub, "test") {
                if n != 5 && n != 6 {
                    return Err(usage_of("commands.scoreboard.players.test.usage"));
                }
                self.sb_cmd_test(sender, args)
            } else if is(sub, "operation") {
                if n != 7 {
                    return Err(usage_of("commands.scoreboard.players.operation.usage"));
                }
                self.sb_cmd_operation(sender, args)
            } else {
                Err(usage_of("commands.scoreboard.players.usage"))
            }
        } else if is(group, "teams") {
            if n == 1 {
                return Err(usage_of("commands.scoreboard.teams.usage"));
            }
            let player = matches!(sender, Sender::Player(_));
            if is(sub, "list") {
                if n > 3 {
                    return Err(usage_of("commands.scoreboard.teams.list.usage"));
                }
                self.sb_list_teams(sender, args)
            } else if is(sub, "add") {
                if n < 3 {
                    return Err(usage_of("commands.scoreboard.teams.add.usage"));
                }
                self.sb_cmd_add_team(sender, args)
            } else if is(sub, "remove") {
                if n != 3 {
                    return Err(usage_of("commands.scoreboard.teams.remove.usage"));
                }
                let team = self.team_arg(args[2])?;
                self.sb_remove_team(&team);
                self.notify(
                    sender,
                    "scoreboard",
                    false,
                    "commands.scoreboard.teams.remove.success",
                    vec![s(&team)],
                );
                Ok(())
            } else if is(sub, "empty") {
                if n != 3 {
                    return Err(usage_of("commands.scoreboard.teams.empty.usage"));
                }
                self.sb_cmd_empty_team(sender, args)
            } else if is(sub, "join") {
                if n < 4 && (n != 3 || !player) {
                    return Err(usage_of("commands.scoreboard.teams.join.usage"));
                }
                self.sb_cmd_join(sender, args)
            } else if is(sub, "leave") {
                if n < 3 && !player {
                    return Err(usage_of("commands.scoreboard.teams.leave.usage"));
                }
                self.sb_cmd_leave(sender, args)
            } else if is(sub, "option") {
                if n != 4 && n != 5 {
                    return Err(usage_of("commands.scoreboard.teams.option.usage"));
                }
                self.sb_cmd_team_option(sender, args)
            } else {
                Err(usage_of("commands.scoreboard.teams.usage"))
            }
        } else {
            Err(usage("scoreboard"))
        }
    }

    fn sb_cmd_add_objective(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let (name, criteria) = (args[2], args[3]);
        if !sb::criteria_exists(criteria) {
            return Err(CommandError::UsageWith(
                "commands.scoreboard.objectives.add.wrongType",
                vec![s(criteria)],
            ));
        }
        if self.scoreboard.objectives.get(name).is_some() {
            return Err(err(
                "commands.scoreboard.objectives.add.alreadyExists",
                vec![s(name)],
            ));
        }
        if java_len(name) > MAX_NAME {
            return Err(err(
                "commands.scoreboard.objectives.add.tooLong",
                vec![s(name), s(MAX_NAME)],
            ));
        }
        if name.is_empty() {
            return Err(usage_of("commands.scoreboard.objectives.add.usage"));
        }
        let display = args[4..].join(" ");
        if java_len(&display) > MAX_DISPLAY {
            return Err(err(
                "commands.scoreboard.objectives.add.displayTooLong",
                vec![s(&display), s(MAX_DISPLAY)],
            ));
        }
        self.sb_add_objective(name, criteria);
        if !display.is_empty() {
            let render = self
                .scoreboard
                .objectives
                .get(name)
                .map_or("integer", |o| o.render);
            self.sb_set_objective(name, display, render);
        }
        self.notify(
            sender,
            "scoreboard",
            false,
            "commands.scoreboard.objectives.add.success",
            vec![s(name)],
        );
        Ok(())
    }

    fn sb_list_objectives(&mut self, sender: Sender) -> CommandResult {
        let objectives: Vec<(String, String, String)> = self
            .scoreboard
            .objectives
            .iter()
            .map(|(n, o)| (n.clone(), o.display.clone(), o.criteria.clone()))
            .collect();
        if objectives.is_empty() {
            return Err(err("commands.scoreboard.objectives.list.empty", vec![]));
        }
        self.reply(
            sender,
            &t(
                "commands.scoreboard.objectives.list.count",
                vec![s(objectives.len())],
            )
            .color("dark_green"),
        );
        for (name, display, criteria) in objectives {
            self.reply(
                sender,
                &t(
                    "commands.scoreboard.objectives.list.entry",
                    vec![s(name), s(display), s(criteria)],
                ),
            );
        }
        Ok(())
    }

    fn sb_cmd_set_display(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let slot_text = args[2];
        let slot = sb::slot_by_name(slot_text);
        let objective = match args.get(3) {
            Some(name) if args.len() == 4 => Some(self.objective_arg(name, false)?),
            _ => None,
        };
        let Some(slot) = slot.filter(|&k| k < DISPLAY_SLOTS) else {
            return Err(err(
                "commands.scoreboard.objectives.setdisplay.invalidSlot",
                vec![s(slot_text)],
            ));
        };
        self.sb_set_display_slot(slot, objective.as_deref());
        let slot_name = sb::slot_name(slot).unwrap_or_default();
        match objective {
            Some(name) => self.notify(
                sender,
                "scoreboard",
                false,
                "commands.scoreboard.objectives.setdisplay.successSet",
                vec![s(slot_name), s(name)],
            ),
            None => self.notify(
                sender,
                "scoreboard",
                false,
                "commands.scoreboard.objectives.setdisplay.successCleared",
                vec![s(slot_name)],
            ),
        }
        Ok(())
    }

    fn sb_list_players(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if let Some(arg) = args.get(2) {
            let holder = self.holder_name(sender, arg)?;
            let entries: Vec<(i32, String, String)> = self
                .scoreboard
                .scores
                .get(&holder)
                .map(|l| {
                    l.iter()
                        .map(|(o, sc)| {
                            let display = self
                                .scoreboard
                                .objectives
                                .get(o)
                                .map(|o| o.display.clone())
                                .unwrap_or_default();
                            (sc.points, display, o.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if entries.is_empty() {
                return Err(err(
                    "commands.scoreboard.players.list.player.empty",
                    vec![s(&holder)],
                ));
            }
            self.reply(
                sender,
                &t(
                    "commands.scoreboard.players.list.player.count",
                    vec![s(entries.len()), s(&holder)],
                )
                .color("dark_green"),
            );
            for (points, display, name) in entries {
                self.reply(
                    sender,
                    &t(
                        "commands.scoreboard.players.list.player.entry",
                        vec![s(points), s(display), s(name)],
                    ),
                );
            }
        } else {
            let holders: Vec<String> = self
                .scoreboard
                .scores
                .iter()
                .map(|(h, _)| h.clone())
                .collect();
            if holders.is_empty() {
                return Err(err("commands.scoreboard.players.list.empty", vec![]));
            }
            self.reply(
                sender,
                &t(
                    "commands.scoreboard.players.list.count",
                    vec![s(holders.len())],
                )
                .color("dark_green"),
            );
            self.reply(sender, &Component::text(join_nice(&holders)));
        }
        Ok(())
    }

    fn sb_cmd_set_player(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let mode = args[1];
        let holder = self.holder_name(sender, args[2])?;
        Self::holder_length(&holder)?;
        let objective = self.objective_arg(args[3], true)?;
        let amount = if mode.eq_ignore_ascii_case("set") {
            parse_int(args[4], i32::MIN, i32::MAX)?
        } else {
            parse_int(args[4], 0, i32::MAX)?
        };
        if args.len() > 5 {
            let target = self.get_entity(sender, args[2], false)?;
            let tag = data_tag(&args[5..], "commands.scoreboard.players.set.tagError")?;
            let nbt = match target {
                Target::Player(i) => {
                    self.players[i].store_state();
                    self.players[i].data.clone()
                }
                // TODO(M5): other entities' NBT.
                Target::Entity(_) => krol_nbt::Compound::new(),
            };
            if !krol_nbt::matches_pattern(
                &krol_nbt::Tag::Compound(tag),
                Some(&krol_nbt::Tag::Compound(nbt)),
                true,
            ) {
                return Err(err(
                    "commands.scoreboard.players.set.tagMismatch",
                    vec![s(&holder)],
                ));
            }
        }
        if mode.eq_ignore_ascii_case("set") {
            self.sb_set_score(&holder, &objective, amount);
        } else if mode.eq_ignore_ascii_case("add") {
            self.sb_add_score(&holder, &objective, amount);
        } else {
            self.sb_add_score(&holder, &objective, amount.wrapping_neg());
        }
        let points = self
            .scoreboard
            .score(&holder, &objective)
            .map_or(0, |sc| sc.points);
        self.notify(
            sender,
            "scoreboard",
            false,
            "commands.scoreboard.players.set.success",
            vec![s(&objective), s(&holder), s(points)],
        );
        Ok(())
    }

    fn sb_cmd_reset(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let holder = self.holder_name(sender, args[2])?;
        if let Some(name) = args.get(3) {
            let objective = self.objective_arg(name, false)?;
            self.sb_reset(&holder, Some(&objective));
            self.notify(
                sender,
                "scoreboard",
                false,
                "commands.scoreboard.players.resetscore.success",
                vec![s(&objective), s(&holder)],
            );
        } else {
            self.sb_reset(&holder, None);
            self.notify(
                sender,
                "scoreboard",
                false,
                "commands.scoreboard.players.reset.success",
                vec![s(&holder)],
            );
        }
        Ok(())
    }

    fn sb_cmd_enable(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let holder = self.player_name_arg(sender, args[2])?;
        Self::holder_length(&holder)?;
        let objective = self.objective_arg(args[3], false)?;
        let is_trigger = self
            .scoreboard
            .objectives
            .get(&objective)
            .is_some_and(|o| o.criteria == TRIGGER);
        if !is_trigger {
            return Err(err(
                "commands.scoreboard.players.enable.noTrigger",
                vec![s(&objective)],
            ));
        }
        self.sb_set_locked(&holder, &objective, false);
        self.notify(
            sender,
            "scoreboard",
            false,
            "commands.scoreboard.players.enable.success",
            vec![s(&objective), s(&holder)],
        );
        Ok(())
    }

    fn sb_cmd_test(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let holder = self.holder_name(sender, args[2])?;
        Self::holder_length(&holder)?;
        let objective = self.objective_arg(args[3], false)?;
        let Some(points) = self
            .scoreboard
            .score(&holder, &objective)
            .map(|sc| sc.points)
        else {
            return Err(err(
                "commands.scoreboard.players.test.notFound",
                vec![s(&objective), s(&holder)],
            ));
        };
        let min = if args[4] == "*" {
            i32::MIN
        } else {
            parse_int(args[4], i32::MIN, i32::MAX)?
        };
        let max = match args.get(5) {
            Some(a) if *a != "*" => parse_int(a, min, i32::MAX)?,
            _ => i32::MAX,
        };
        if points >= min && points <= max {
            self.notify(
                sender,
                "scoreboard",
                false,
                "commands.scoreboard.players.test.success",
                vec![s(points), s(min), s(max)],
            );
            Ok(())
        } else {
            Err(err(
                "commands.scoreboard.players.test.failed",
                vec![s(points), s(min), s(max)],
            ))
        }
    }

    fn sb_cmd_operation(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let target = self.holder_name(sender, args[2])?;
        let target_objective = self.objective_arg(args[3], true)?;
        let op = args[4];
        let source = self.holder_name(sender, args[5])?;
        let source_objective = self.objective_arg(args[6], false)?;
        Self::holder_length(&target)?;
        Self::holder_length(&source)?;
        let current = self
            .scoreboard
            .get_or_create(&target, &target_objective)
            .points;
        let Some(other) = self
            .scoreboard
            .score(&source, &source_objective)
            .map(|sc| sc.points)
        else {
            return Err(err(
                "commands.scoreboard.players.operation.notFound",
                vec![s(&source_objective), s(&source)],
            ));
        };
        let result = match op {
            "+=" => Some(current.wrapping_add(other)),
            "-=" => Some(current.wrapping_sub(other)),
            "*=" => Some(current.wrapping_mul(other)),
            "/=" => (other != 0).then(|| current.wrapping_div(other)),
            "%=" => (other != 0).then(|| current.wrapping_rem(other)),
            "=" => Some(other),
            "<" => Some(current.min(other)),
            ">" => Some(current.max(other)),
            "><" => {
                self.sb_set_score(&target, &target_objective, other);
                self.sb_set_score(&source, &source_objective, current);
                None
            }
            _ => {
                return Err(err(
                    "commands.scoreboard.players.operation.invalidOperation",
                    vec![s(op)],
                ));
            }
        };
        if let Some(value) = result {
            self.sb_set_score(&target, &target_objective, value);
        }
        self.notify(
            sender,
            "scoreboard",
            false,
            "commands.scoreboard.players.operation.success",
            vec![],
        );
        Ok(())
    }

    fn sb_list_teams(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if let Some(name) = args.get(2) {
            let team = self.team_arg(name)?;
            let members: Vec<String> = self
                .scoreboard
                .teams
                .get(&team)
                .map(|t| t.members.iter().map(|(m, _)| m.clone()).collect())
                .unwrap_or_default();
            if members.is_empty() {
                return Err(err(
                    "commands.scoreboard.teams.list.player.empty",
                    vec![s(&team)],
                ));
            }
            self.reply(
                sender,
                &t(
                    "commands.scoreboard.teams.list.player.count",
                    vec![s(members.len()), s(&team)],
                )
                .color("dark_green"),
            );
            self.reply(sender, &Component::text(join_nice(&members)));
        } else {
            let teams: Vec<(String, String, usize)> = self
                .scoreboard
                .teams
                .iter()
                .map(|(n, t)| (n.clone(), t.display.clone(), t.members.len()))
                .collect();
            if teams.is_empty() {
                return Err(err("commands.scoreboard.teams.list.empty", vec![]));
            }
            self.reply(
                sender,
                &t("commands.scoreboard.teams.list.count", vec![s(teams.len())])
                    .color("dark_green"),
            );
            for (name, display, count) in teams {
                self.reply(
                    sender,
                    &t(
                        "commands.scoreboard.teams.list.entry",
                        vec![s(name), s(display), s(count)],
                    ),
                );
            }
        }
        Ok(())
    }

    fn sb_cmd_add_team(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let name = args[2];
        if self.scoreboard.teams.get(name).is_some() {
            return Err(err(
                "commands.scoreboard.teams.add.alreadyExists",
                vec![s(name)],
            ));
        }
        if java_len(name) > MAX_NAME {
            return Err(err(
                "commands.scoreboard.teams.add.tooLong",
                vec![s(name), s(MAX_NAME)],
            ));
        }
        if name.is_empty() {
            return Err(usage_of("commands.scoreboard.teams.add.usage"));
        }
        let display = args[3..].join(" ");
        if java_len(&display) > MAX_DISPLAY {
            return Err(err(
                "commands.scoreboard.teams.add.displayTooLong",
                vec![s(&display), s(MAX_DISPLAY)],
            ));
        }
        self.sb_create_team(name);
        if !display.is_empty() {
            self.sb_update_team(name, |t| t.display = display);
        }
        self.notify(
            sender,
            "scoreboard",
            false,
            "commands.scoreboard.teams.add.success",
            vec![s(name)],
        );
        Ok(())
    }

    fn sb_cmd_empty_team(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let team = self.team_arg(args[2])?;
        let members: Vec<String> = self
            .scoreboard
            .teams
            .get(&team)
            .map(|t| t.members.iter().map(|(m, _)| m.clone()).collect())
            .unwrap_or_default();
        if members.is_empty() {
            return Err(err(
                "commands.scoreboard.teams.empty.alreadyEmpty",
                vec![s(&team)],
            ));
        }
        for m in &members {
            self.sb_leave_teams(m);
        }
        self.notify(
            sender,
            "scoreboard",
            false,
            "commands.scoreboard.teams.empty.success",
            vec![s(members.len()), s(&team)],
        );
        Ok(())
    }

    /// The holders named from `args` on, selectors expanded like `func_175763_c`.
    fn sb_holders(&self, sender: Sender, args: &[&str]) -> Result<Vec<String>, CommandError> {
        let mut names = Vec::new();
        for arg in args {
            if arg.starts_with('@') {
                let targets = if selector::is_selector(arg) {
                    self.match_entities(sender, arg, false)
                } else {
                    vec![self.get_entity(sender, arg, false)?]
                };
                for target in targets {
                    let uuid = crate::status::java_uuid_string(self.target_uuid(target));
                    names.push(self.holder_name(sender, &uuid)?);
                }
            } else {
                names.push(self.holder_name(sender, arg)?);
            }
        }
        Ok(names)
    }

    fn sb_cmd_join(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let team = args[2].to_owned();
        let holders = match sender {
            Sender::Player(i) if args.len() == 3 => vec![self.players[i].profile.name.clone()],
            _ => self.sb_holders(sender, &args[3..])?,
        };
        let mut added = krol_java::JavaHashMap::new();
        let mut failed = krol_java::JavaHashMap::new();
        for h in holders {
            if self.sb_join_team(&h, &team) {
                added.insert(h, ());
            } else {
                failed.insert(h, ());
            }
        }
        let names = |m: &krol_java::JavaHashMap<()>| -> Vec<String> {
            m.iter().map(|(k, _)| k.clone()).collect()
        };
        if !added.is_empty() {
            self.notify(
                sender,
                "scoreboard",
                false,
                "commands.scoreboard.teams.join.success",
                vec![s(added.len()), s(&team), s(join_nice(&names(&added)))],
            );
        }
        if !failed.is_empty() {
            return Err(err(
                "commands.scoreboard.teams.join.failure",
                vec![s(failed.len()), s(&team), s(join_nice(&names(&failed)))],
            ));
        }
        Ok(())
    }

    fn sb_cmd_leave(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let holders = match sender {
            Sender::Player(i) if args.len() == 2 => vec![self.players[i].profile.name.clone()],
            _ => self.sb_holders(sender, &args[2..])?,
        };
        let mut removed = krol_java::JavaHashMap::new();
        let mut failed = krol_java::JavaHashMap::new();
        for h in holders {
            if self.sb_leave_teams(&h) {
                removed.insert(h, ());
            } else {
                failed.insert(h, ());
            }
        }
        let names = |m: &krol_java::JavaHashMap<()>| -> Vec<String> {
            m.iter().map(|(k, _)| k.clone()).collect()
        };
        if !removed.is_empty() {
            self.notify(
                sender,
                "scoreboard",
                false,
                "commands.scoreboard.teams.leave.success",
                vec![s(removed.len()), s(join_nice(&names(&removed)))],
            );
        }
        if !failed.is_empty() {
            return Err(err(
                "commands.scoreboard.teams.leave.failure",
                vec![s(failed.len()), s(join_nice(&names(&failed)))],
            ));
        }
        Ok(())
    }

    fn sb_cmd_team_option(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        let team = self.team_arg(args[2])?;
        let option = args[3].to_lowercase();
        if !TEAM_OPTIONS.iter().any(|o| o.eq_ignore_ascii_case(&option)) {
            return Err(usage_of("commands.scoreboard.teams.option.usage"));
        }
        let is = |o: &str| option.eq_ignore_ascii_case(o);
        let no_value = |values: String| {
            CommandError::UsageWith(
                "commands.scoreboard.teams.option.noValue",
                vec![s(&option), s(values)],
            )
        };
        let colors = || join_nice(&sb::color_names());
        let booleans = || join_nice(&["true", "false"]);
        let visibilities = || join_nice(&sb::visibility_names());
        let Some(&value) = args.get(4) else {
            return Err(if is("color") {
                no_value(colors())
            } else if is("friendlyfire") || is("seeFriendlyInvisibles") {
                no_value(booleans())
            } else {
                no_value(visibilities())
            });
        };
        let boolean = || -> Result<bool, CommandError> {
            if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
                Ok(value.eq_ignore_ascii_case("true"))
            } else {
                Err(no_value(booleans()))
            }
        };
        let visibility = || -> Result<&'static str, CommandError> {
            VISIBILITIES
                .iter()
                .find(|v| **v == value)
                .copied()
                .ok_or_else(|| no_value(visibilities()))
        };
        if is("color") {
            let Some(color) = sb::formatting_by_name(value).filter(|&f| !sb::is_fancy(f)) else {
                return Err(no_value(colors()));
            };
            if let Some(t) = self.scoreboard.teams.get_mut(&team) {
                t.color = color;
            }
            self.sb_update_team(&team, |t| t.prefix = sb::formatting_code(color));
            self.sb_update_team(&team, |t| t.suffix = sb::formatting_code(RESET));
        } else if is("friendlyfire") {
            let v = boolean()?;
            self.sb_update_team(&team, |t| t.friendly_fire = v);
        } else if is("seeFriendlyInvisibles") {
            let v = boolean()?;
            self.sb_update_team(&team, |t| t.see_invisibles = v);
        } else if is("nametagVisibility") {
            let v = visibility()?;
            self.sb_update_team(&team, |t| t.nametag = v);
        } else {
            let v = visibility()?;
            self.sb_update_team(&team, |t| t.death_messages = v);
        }
        self.notify(
            sender,
            "scoreboard",
            false,
            "commands.scoreboard.teams.option.success",
            vec![s(&option), s(&team), s(value)],
        );
        Ok(())
    }

    /// `/trigger <objective> <add|set> <value>`.
    pub(super) fn cmd_trigger(&mut self, sender: Sender, args: &[&str]) -> CommandResult {
        if args.len() < 3 {
            return Err(usage("trigger"));
        }
        let Sender::Player(i) = sender else {
            return Err(err("commands.trigger.invalidPlayer", vec![]));
        };
        let is_trigger = self
            .scoreboard
            .objectives
            .get(args[0])
            .is_some_and(|o| o.criteria == TRIGGER);
        if !is_trigger {
            return Err(err("commands.trigger.invalidObjective", vec![s(args[0])]));
        }
        let value = parse_int(args[2], i32::MIN, i32::MAX)?;
        let name = self.players[i].profile.name.clone();
        let Some(locked) = self.scoreboard.score(&name, args[0]).map(|sc| sc.locked) else {
            return Err(err("commands.trigger.invalidObjective", vec![s(args[0])]));
        };
        if locked {
            return Err(err("commands.trigger.disabled", vec![s(args[0])]));
        }
        match args[1] {
            "set" => self.sb_set_score(&name, args[0], value),
            "add" => self.sb_add_score(&name, args[0], value),
            other => return Err(err("commands.trigger.invalidMode", vec![s(other)])),
        }
        self.sb_set_locked(&name, args[0], true);
        if self.players[i].is_creative() {
            self.notify(
                sender,
                "trigger",
                false,
                "commands.trigger.success",
                vec![s(args[0]), s(args[1]), s(args[2])],
            );
        }
        Ok(())
    }
}
