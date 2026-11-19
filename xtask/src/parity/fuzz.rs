//! `parity fuzz`: random bot sessions run against vanilla and krol side by side.
//!
//! Each case is a player (an operator, starting in creative mode) in the flat fixture
//! world doing a random sequence of steps: walk, look, set a hotbar slot, select a slot,
//! place or use on a block, use in the air, dig (in survival: finishing or aborting after
//! a while), drop items, click in a window, close it, sneak, switch between creative and
//! survival, summon armor stands and minecarts, and attack or interact with recently
//! spawned entities. Steps get one second each and actions happen while the
//! player stands still, so the bot never races its own movement and each action's
//! effects land before the next one (see `parity/README.md`, "Writing race-free
//! scenarios"). Everything the case can change is observed: the bot's packets, every
//! block, light level and tile entity in and around the area, the saved entities of the
//! chunk, the player file and the log.
//!
//! When the servers disagree, vanilla is run a second time: if vanilla also disagrees with
//! itself, the case is reported as `noisy` (the harness, not krol, has to change).
//! Otherwise the case is shrunk by removing steps (delta debugging) while it still fails,
//! checked once more, and written to `parity/scenarios/fuzz/seed-<N>.toml`, ready for
//! `parity record` and the usual loop.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use super::diff::Difference;
use super::scenario::{Scenario, Step};
use super::server::Target;
use super::{DiffEntry, build_krol, compare, flag_value, observe, parity_root, play, work_dir};

/// The player's feet when it joins (from `fixtures/playerdata/notch_creative.dat`).
const ORIGIN: [f64; 3] = [-471.5, 4.0, -434.5];
/// Notch's offline UUID, which names the player file.
const PLAYER_FILE: &str = "world/playerdata/b50ad385-829d-3141-a216-7e7d7539ba7f.dat";
/// Blocks the bot targets: x, y, z ranges (inclusive), all within chunk (-30, -28).
const TARGET_X: (i32, i32) = (-475, -468);
const TARGET_Y: (i32, i32) = (1, 6);
const TARGET_Z: (i32, i32) = (-438, -434);
/// Where the player's feet may walk to (blocks; it flies in creative).
const WALK_X: (f64, f64) = (-474.5, -468.5);
const WALK_Z: (f64, f64) = (-437.5, -433.5);
const WALK_Y: [f64; 3] = [4.0, 5.0, 6.0];
/// Heights a survival player walks to: mostly the ground, sometimes up high to fall
/// back down (4 blocks: one point of damage).
const SURVIVAL_WALK_Y: [f64; 4] = [4.0, 4.0, 4.0, 8.0];
/// The ground the player stands on.
const GROUND_Y: f64 = 4.0;
/// Observed blocks: the target area grown by one block (beds and doors reach out).
const OBSERVE_X: (i32, i32) = (-476, -467);
const OBSERVE_Y: (i32, i32) = (0, 7);
const OBSERVE_Z: (i32, i32) = (-439, -433);
/// The chunks whose saved entities are compared: the area's, and the one south of it
/// (the area reaches its edge).
const CHUNKS: [[i32; 2]; 2] = [[-30, -28], [-30, -27]];

/// Bot ticks before the first step (the join settles first).
const LEAD_TICKS: usize = 20;
/// Bot ticks per step: actions are a second apart, like hand-written scenarios.
const STEP_TICKS: usize = 20;
/// The tick within a step at which its action is sent (the player stands still).
const ACTION_TICK: usize = 10;
/// Ticks after a survival dig starts that it is finished: no block mined by hand needs
/// within one tick of that (dirt needs 10, wool 16, logs 42), so the jitter between bot
/// and server ticks never decides whether it breaks.
const DIG_FINISH_TICKS: usize = 18;
/// Ticks after a survival dig starts that an aborted one is aborted.
const DIG_ABORT_TICKS: usize = 5;
/// Player Digging statuses.
const DIG_STATUS_ABORT: i8 = 1;
const DIG_STATUS_FINISH: i8 = 2;
const DIG_STATUS_DROP_STACK: i8 = 3;
const DIG_STATUS_DROP_ITEM: i8 = 4;
/// How far back `#<n>` entity references reach (the n-th most recently spawned entity).
const ENTITY_BACK: u64 = 3;
/// Entities the bot summons with `/summon`.
const SUMMONED: &[&str] = &["ArmorStand", "MinecartRideable"];
/// Seconds the session runs on after the last step.
const TAIL_SECONDS: u64 = 3;
/// Milliseconds per tick.
const TICK_MS: usize = 50;

/// First hotbar slot in the player's inventory window.
const HOTBAR_SLOT_0: u8 = 36;
/// Slots of the player's inventory window (crafting, armor, main, hotbar).
const PLAYER_WINDOW_SLOTS: i16 = 45;

/// Items the bot takes from the creative inventory: id and damage. Left out on purpose:
/// what draws from unseeded randoms or lives on its own clock (falling blocks, TNT,
/// liquids, spawn eggs, throwables, bone meal: its growth rolls on the world random),
/// which would make vanilla disagree with itself.
const ITEMS: &[(i16, i16)] = &[
    (1, 0),   // stone
    (1, 3),   // diorite
    (3, 0),   // dirt
    (4, 0),   // cobblestone
    (5, 0),   // oak planks
    (5, 2),   // birch planks
    (17, 0),  // oak log
    (17, 2),  // birch log
    (20, 0),  // glass
    (35, 14), // red wool
    (44, 0),  // stone slab
    (44, 3),  // cobblestone slab
    (126, 1), // spruce slab
    (53, 0),  // oak stairs
    (67, 0),  // cobblestone stairs
    (50, 0),  // torch
    (76, 0),  // redstone torch
    (69, 0),  // lever
    (77, 0),  // stone button
    (143, 0), // wooden button
    (70, 0),  // stone pressure plate
    (324, 0), // oak door
    (330, 0), // iron door
    (96, 0),  // trapdoor
    (107, 0), // fence gate
    (85, 0),  // fence
    (139, 0), // cobblestone wall
    (101, 0), // iron bars
    (102, 0), // glass pane
    (65, 0),  // ladder
    (106, 0), // vine
    (54, 0),  // chest
    (146, 0), // trapped chest
    (61, 0),  // furnace
    (58, 0),  // crafting table
    (331, 0), // redstone
    (356, 0), // repeater
    (404, 0), // comparator
    (123, 0), // redstone lamp
    (152, 0), // block of redstone
    (33, 0),  // piston
    (29, 0),  // sticky piston
    (154, 0), // hopper
    (23, 0),  // dispenser
    (158, 0), // dropper
    (25, 0),  // note block
    (89, 0),  // glowstone
    (66, 0),  // rail
    (27, 0),  // powered rail
    (355, 0), // bed
    (323, 0), // sign
    (390, 0), // flower pot
    (38, 0),  // poppy
    (31, 1),  // tall grass
    (78, 0),  // snow layer
    (171, 4), // yellow carpet
    (397, 3), // player head
    (120, 0), // end portal frame
    (290, 0), // wooden hoe
    (280, 0), // stick
    (268, 0), // wooden sword
    (416, 0), // armor stand
    (389, 0), // item frame
    (328, 0), // minecart
    (322, 0), // golden apple
    (335, 0), // milk bucket
    (298, 0), // leather cap
    (307, 0), // iron chestplate
];

/// One step of a case. Positions are absolute.
#[derive(Debug, Clone, PartialEq)]
enum FuzzStep {
    Walk {
        to: [f64; 3],
    },
    Look {
        yaw: f32,
        pitch: f32,
    },
    Creative {
        slot: u8,
        item: (i16, i16),
        count: i8,
    },
    Hold {
        slot: u8,
    },
    Place {
        pos: [i32; 3],
        face: i8,
        cursor: [i8; 3],
    },
    Dig {
        pos: [i32; 3],
        face: i8,
        end: DigEnd,
    },
    /// Use the held item in the air (eat, drink, put on armor).
    Use,
    /// Q (one item) or Ctrl+Q (the stack).
    Drop {
        stack: bool,
    },
    /// `/gamemode` 0 or 1.
    Gamemode(u8),
    /// `/summon` an entity on top of a block.
    Summon {
        kind: &'static str,
        pos: [i32; 3],
    },
    /// Left click on the `back`-th most recently spawned entity.
    Attack {
        back: u8,
    },
    /// Right click on it; with `at`, a 1.8 client's click at that point of it first.
    Interact {
        back: u8,
        at: Option<[f32; 3]>,
    },
    Click {
        window: i8,
        slot: i16,
        button: i8,
        mode: i8,
    },
    Close,
    Sneak(bool),
}

/// How a dig goes on after it started.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DigEnd {
    /// Nothing more: creative players break blocks at once.
    None,
    /// Survival: the finish packet, [`DIG_FINISH_TICKS`] later.
    Finish,
    /// Survival: the abort packet, [`DIG_ABORT_TICKS`] later.
    Abort,
}

/// SplitMix64: a small, well-mixed generator; cases only need to be reproducible from
/// their seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// Uniform on a 1/16 grid in `lo..=hi`.
    fn grid(&mut self, lo: f64, hi: f64) -> f64 {
        let steps = ((hi - lo) * 16.0) as u64;
        lo + self.below(steps + 1) as f64 / 16.0
    }
}

/// Generates `count` steps from `seed`.
fn generate(seed: u64, count: usize) -> Vec<FuzzStep> {
    let mut rng = Rng(seed);
    let mut steps = Vec::with_capacity(count);
    let mut sneaking = false;
    let mut survival = false;
    let mut here = ORIGIN;
    // Start with something in hand, so placements mean something from the first step.
    if count > 0 {
        steps.push(creative(&mut rng, 0));
    }
    // Weights: walk, look, creative, hold, place, dig, click, close, sneak, gamemode,
    // use, drop, summon, attack, interact.
    const WEIGHTS: [u64; 15] = [3, 2, 3, 2, 7, 3, 2, 1, 1, 1, 1, 1, 1, 2, 2];
    let total: u64 = WEIGHTS.iter().sum();
    while steps.len() < count {
        let mut roll = rng.below(total);
        let kind = WEIGHTS
            .iter()
            .position(|w| {
                if roll < *w {
                    true
                } else {
                    roll -= w;
                    false
                }
            })
            .unwrap_or(0);
        let step = match kind {
            0 => {
                let ys: &[f64] = if survival { &SURVIVAL_WALK_Y } else { &WALK_Y };
                let to = [
                    rng.grid(WALK_X.0, WALK_X.1),
                    ys[rng.below(ys.len() as u64) as usize],
                    rng.grid(WALK_Z.0, WALK_Z.1),
                ];
                here = to;
                FuzzStep::Walk { to }
            }
            1 => FuzzStep::Look {
                yaw: (rng.below(8) * 45) as f32 + rng.range(-20, 20) as f32,
                pitch: rng.range(-30, 90) as f32,
            },
            2 => {
                let slot = rng.below(9) as u8;
                creative(&mut rng, slot)
            }
            3 => FuzzStep::Hold {
                slot: rng.below(9) as u8,
            },
            4 => FuzzStep::Place {
                pos: target(&mut rng),
                face: rng.below(6) as i8,
                cursor: [
                    rng.below(16) as i8,
                    rng.below(16) as i8,
                    rng.below(16) as i8,
                ],
            },
            5 => FuzzStep::Dig {
                pos: target(&mut rng),
                face: rng.below(6) as i8,
                end: match (survival, rng.chance(75)) {
                    (false, _) => DigEnd::None,
                    (true, true) => DigEnd::Finish,
                    (true, false) => DigEnd::Abort,
                },
            },
            6 => {
                let mode = [0, 0, 1, 2][rng.below(4) as usize];
                FuzzStep::Click {
                    // Window 1 is the first container a placement may have opened.
                    window: if rng.chance(25) { 1 } else { 0 },
                    slot: rng.below(PLAYER_WINDOW_SLOTS as u64) as i16,
                    button: match mode {
                        2 => rng.below(9) as i8,
                        _ => rng.below(2) as i8,
                    },
                    mode,
                }
            }
            7 => FuzzStep::Close,
            8 => {
                sneaking = !sneaking;
                FuzzStep::Sneak(sneaking)
            }
            9 => {
                survival = !survival;
                if survival && here[1] > GROUND_Y {
                    // Down to the ground first, or the player floats (and gets kicked).
                    here[1] = GROUND_Y;
                    steps.push(FuzzStep::Walk { to: here });
                    if steps.len() == count {
                        break;
                    }
                }
                FuzzStep::Gamemode(u8::from(!survival))
            }
            10 => FuzzStep::Use,
            11 => FuzzStep::Drop {
                stack: rng.chance(30),
            },
            12 => FuzzStep::Summon {
                kind: SUMMONED[rng.below(SUMMONED.len() as u64) as usize],
                pos: target(&mut rng),
            },
            13 => FuzzStep::Attack {
                back: 1 + rng.below(ENTITY_BACK) as u8,
            },
            _ => FuzzStep::Interact {
                back: 1 + rng.below(ENTITY_BACK) as u8,
                // A point within an armor stand's box (0.5 wide, 1.975 high).
                at: rng.chance(50).then(|| {
                    [
                        rng.range(-4, 4) as f32 / 16.0,
                        rng.range(0, 31) as f32 / 16.0,
                        rng.range(-4, 4) as f32 / 16.0,
                    ]
                }),
            },
        };
        steps.push(step);
    }
    steps
}

fn creative(rng: &mut Rng, slot: u8) -> FuzzStep {
    FuzzStep::Creative {
        slot,
        item: ITEMS[rng.below(ITEMS.len() as u64) as usize],
        count: if rng.chance(70) {
            64
        } else {
            rng.range(1, 16) as i8
        },
    }
}

/// A target block, biased towards the grass surface (y 3) where most placements stick.
fn target(rng: &mut Rng) -> [i32; 3] {
    let y = if rng.chance(40) {
        3
    } else {
        rng.range(TARGET_Y.0, TARGET_Y.1)
    };
    [
        rng.range(TARGET_X.0, TARGET_X.1),
        y,
        rng.range(TARGET_Z.0, TARGET_Z.1),
    ]
}

/// Renders a case as scenario TOML. `header` lines become comments at the top;
/// `noisy` paths (where two vanilla runs disagreed) are ignored.
fn render(steps: &[FuzzStep], header: &[String], noisy: &[String]) -> String {
    let mut chat = Vec::new();
    // A position for a walk's tick; `None` stands still (a Player packet without a
    // position, so a corrected position stays corrected).
    let mut moves: Vec<Option<[f64; 3]>> = Vec::new();
    let mut action_number = 0i16;
    // Lines timed after their step's action (the end of a survival dig).
    let mut later: Vec<(usize, String)> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let start = LEAD_TICKS + i * STEP_TICKS;
        while moves.len() < start {
            moves.push(None);
        }
        let at = (start + ACTION_TICK) * TICK_MS;
        let line = match step {
            // One packet: a walk the server rejects (into a block) then gets exactly one
            // correction. Several packets would race the correction, since the moves
            // already on their way when it is sent hit the block again.
            FuzzStep::Walk { to } => {
                moves.push(Some(*to));
                None
            }
            FuzzStep::Look { yaw, pitch } => Some(format!("!look {yaw} {pitch}")),
            FuzzStep::Creative { slot, item, count } => Some(format!(
                "!creative {} {} {count} {}",
                HOTBAR_SLOT_0 + slot,
                item.0,
                item.1
            )),
            FuzzStep::Hold { slot } => Some(format!("!hold {slot}")),
            FuzzStep::Place { pos, face, cursor } => Some(format!(
                "!place {} {} {} {face} {} {} {}",
                pos[0], pos[1], pos[2], cursor[0], cursor[1], cursor[2]
            )),
            FuzzStep::Dig { pos, face, end } => {
                let ending = match end {
                    DigEnd::None => None,
                    DigEnd::Finish => Some((DIG_FINISH_TICKS, DIG_STATUS_FINISH)),
                    DigEnd::Abort => Some((DIG_ABORT_TICKS, DIG_STATUS_ABORT)),
                };
                if let Some((ticks, status)) = ending {
                    later.push((
                        at + ticks * TICK_MS,
                        format!("!dig {} {} {} {face} {status}", pos[0], pos[1], pos[2]),
                    ));
                }
                Some(format!("!dig {} {} {} {face}", pos[0], pos[1], pos[2]))
            }
            FuzzStep::Use => Some("!use".to_owned()),
            FuzzStep::Drop { stack } => Some(format!(
                "!dig 0 0 0 0 {}",
                if *stack {
                    DIG_STATUS_DROP_STACK
                } else {
                    DIG_STATUS_DROP_ITEM
                }
            )),
            FuzzStep::Gamemode(mode) => Some(format!("/gamemode {mode}")),
            FuzzStep::Summon { kind, pos } => Some(format!(
                "/summon {kind} {} {} {}",
                f64::from(pos[0]) + 0.5,
                pos[1] + 1,
                f64::from(pos[2]) + 0.5
            )),
            FuzzStep::Attack { back } => Some(format!("!attack #{back}")),
            FuzzStep::Interact { back, at: None } => Some(format!("!interact #{back}")),
            FuzzStep::Interact {
                back,
                at: Some([x, y, z]),
            } => Some(format!("!interactat #{back} {x} {y} {z}")),
            FuzzStep::Click {
                window,
                slot,
                button,
                mode,
            } => {
                action_number += 1;
                Some(format!(
                    "!click {window} {slot} {button} {action_number} {mode}"
                ))
            }
            FuzzStep::Close => Some("!close 0".to_owned()),
            FuzzStep::Sneak(true) => Some("!sneak".to_owned()),
            FuzzStep::Sneak(false) => Some("!unsneak".to_owned()),
        };
        if let Some(line) = line {
            chat.push(format!("@{at} {line}"));
        }
        for (ms, line) in later.drain(..) {
            chat.push(format!("@{ms} {line}"));
        }
    }
    let end = LEAD_TICKS + steps.len() * STEP_TICKS;
    // Close the window in the tail: a stack left on the cursor (or in a crafting grid)
    // drops now, on the bot's schedule, instead of at logout, where the item may or may
    // not update once before the harness's `stop` (`play/logout_cursor_drop` covers
    // that).
    chat.push(format!("@{} !close 0", (end + ACTION_TICK) * TICK_MS));
    while moves.len() < end {
        moves.push(None);
    }
    let seconds = (end * TICK_MS).div_ceil(1000) as u64 + TAIL_SECONDS;

    let mut t = String::new();
    for line in header {
        let _ = writeln!(t, "# {line}");
    }
    let _ = writeln!(
        t,
        "description = \"Fuzz case: {} random steps (cargo xtask parity fuzz)\"",
        steps.len()
    );
    t.push_str(
        r#"subsystem = "fuzz"

[setup]
world = "fixtures/worlds/flat"
properties = { online-mode = "false", level-type = "FLAT", generate-structures = "false", spawn-animals = "false", spawn-monsters = "false", spawn-npcs = "false", allow-nether = "false", gamemode = "1" }

# A fixed position (creative players get a random spawn offset otherwise).
[setup.files]
"#,
    );
    let _ = writeln!(
        t,
        "\"{PLAYER_FILE}\" = \"fixtures/playerdata/notch_creative.dat\""
    );
    // An operator (by the offline UUID: `op Notch` would look the name up at Mojang), to
    // switch game modes and summon entities.
    t.push_str("\"ops.json\" = \"fixtures/ops_notch_offline.json\"\n");
    t.push_str(
        r#"
# Random block ticks change grass at random; drops fly off with unseeded motion.
[[steps]]
action = "console"
command = "gamerule randomTickSpeed 0"

[[steps]]
action = "console"
command = "gamerule doTileDrops false"

[[steps]]
id = "bot"
action = "play"
protocol = 47
name = "Notch"
"#,
    );
    let _ = writeln!(t, "seconds = {seconds}");
    // The tail then lasts the same on both servers, however long each takes to place
    // the player (vanilla is a few ticks slower), so items dropped in it age the same.
    t.push_str("seconds_from_spawn = true\n");
    t.push_str("chat = [\n");
    for line in &chat {
        let _ = writeln!(t, "    \"{line}\",");
    }
    t.push_str("]\nmoves = [\n");
    for m in &moves {
        match m {
            Some(m) => {
                let _ = writeln!(
                    t,
                    "    [{}, {}, {}],",
                    num(m[0] - ORIGIN[0]),
                    num(m[1] - ORIGIN[1]),
                    num(m[2] - ORIGIN[2])
                );
            }
            None => t.push_str("    [1.0],\n"),
        }
    }
    t.push_str("]\n\n[observe]\nlog = true\n");
    let _ = writeln!(t, "files = [\"{PLAYER_FILE}\"]");
    let chunks: Vec<String> = CHUNKS
        .iter()
        .map(|c| format!("[{}, {}]", c[0], c[1]))
        .collect();
    let _ = writeln!(t, "entities = [{}]", chunks.join(", "));
    let area = observed_area();
    for key in ["blocks", "light", "tile_entities"] {
        let _ = writeln!(t, "{key} = [{area}]");
    }
    t.push_str("ignore = [\n");
    let _ = writeln!(
        t,
        "    # Vanilla replaces a stored XpSeed of 0 with an unseeded random value on load.\n    \"file:{PLAYER_FILE}.XpSeed\","
    );
    let _ = writeln!(
        t,
        "    # A flying player's motion keeps changing with each server tick until it logs out.\n    \"file:{PLAYER_FILE}.Motion\","
    );
    let entities: Vec<String> = CHUNKS
        .iter()
        .map(|c| format!("entities.{},{}[*]", c[0], c[1]))
        .collect();
    t.push_str(
        "    # Tossed and dropped items: a random yaw (Math.random) and spread (the\n    # dropping entity's random). Placed armor stands: a random pose (the world's random).\n",
    );
    for e in &entities {
        let _ = writeln!(
            t,
            "    \"{e}.Motion\",\n    \"{e}.Rotation\",\n    \"{e}.Pose\","
        );
    }
    if !noisy.is_empty() {
        t.push_str(
            "    # Two vanilla runs of this case disagreed here (unseeded randoms or timing).\n",
        );
        for path in noisy {
            let _ = writeln!(t, "    \"{path}\",");
        }
    }
    t.push_str("]\n");
    // How far a tossed item's random spread carries it, and the tick or two the bot's
    // clock may drift against the server's between its drop and the save (an item
    // dropped right before it may have updated once more: Fire, FallDistance).
    t.push_str("\n[observe.tolerance]\n");
    for e in &entities {
        let _ = writeln!(t, "\"{e}.Pos\" = 1.0");
        let _ = writeln!(t, "\"{e}.Age\" = 2");
        let _ = writeln!(t, "\"{e}.PickupDelay\" = 2");
        let _ = writeln!(t, "\"{e}.Fire\" = 1");
        let _ = writeln!(t, "\"{e}.FallDistance\" = 1.0");
    }
    t
}

/// A number as TOML float text, without float noise from the interpolation.
fn num(v: f64) -> String {
    let rounded = (v * 1024.0).round() / 1024.0;
    let s = format!("{rounded}");
    if s.contains('.') { s } else { format!("{s}.0") }
}

fn observed_area() -> String {
    let mut out = Vec::new();
    for x in OBSERVE_X.0..=OBSERVE_X.1 {
        for y in OBSERVE_Y.0..=OBSERVE_Y.1 {
            for z in OBSERVE_Z.0..=OBSERVE_Z.1 {
                out.push(format!("[{x}, {y}, {z}]"));
            }
        }
    }
    out.join(", ")
}

fn scenario(steps: &[FuzzStep], name: &str, noisy: &[String]) -> Result<(Scenario, String)> {
    let text = render(steps, &[], noisy);
    let mut sc: Scenario = toml::from_str(&text).context("parsing a generated fuzz case")?;
    sc.name = name.to_owned();
    Ok((sc, text))
}

/// Runs one generated case on both servers (at once) and returns the observations in
/// the form they are compared in: vanilla with protocol 47, krol with the requested one.
struct Pair<'a> {
    krol: &'a Target,
    vanilla: &'a Target,
    v5: bool,
}

impl Pair<'_> {
    fn run(&self, sc: &Scenario) -> Result<(Value, Value)> {
        let mut krol_sc = sc.clone();
        if self.v5 {
            for step in &mut krol_sc.steps {
                if let Step::Play { protocol, .. } = step {
                    *protocol = 5;
                }
            }
        }
        let root = parity_root();
        let (vanilla, krol) = std::thread::scope(|s| {
            let v =
                s.spawn(|| observe::run(sc, self.vanilla, &work_dir(&sc.name, "vanilla"), &root));
            let k =
                s.spawn(|| observe::run(&krol_sc, self.krol, &work_dir(&sc.name, "krol"), &root));
            (join(v), join(k))
        });
        let (mut vanilla, mut krol) = (vanilla?, krol?);
        if self.v5 {
            vanilla = play::v5_view_all(&vanilla, false);
            krol = play::v5_view_all(&krol, true);
        }
        Ok((vanilla, krol))
    }

    fn vanilla_again(&self, sc: &Scenario) -> Result<Value> {
        let v = observe::run(
            sc,
            self.vanilla,
            &work_dir(&sc.name, "vanilla-2"),
            &parity_root(),
        )?;
        Ok(if self.v5 {
            play::v5_view_all(&v, false)
        } else {
            v
        })
    }

    /// The differences of a case.
    fn diffs(&self, steps: &[FuzzStep], name: &str, noisy: &[String]) -> Result<Vec<Difference>> {
        let (sc, _) = scenario(steps, name, noisy)?;
        let (vanilla, krol) = self.run(&sc)?;
        Ok(compare(&sc, vanilla, krol))
    }
}

fn join<T>(handle: std::thread::ScopedJoinHandle<'_, Result<T>>) -> Result<T> {
    handle
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("a server thread panicked")))
}

#[derive(Debug, Serialize)]
struct CaseReport {
    seed: u64,
    /// `pass`, `fail`, `noisy` (vanilla disagreed with itself) or `error`.
    status: String,
    steps: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    shrunk_steps: Option<usize>,
    /// Whether the shrunk case failed again when run once more.
    #[serde(skip_serializing_if = "Option::is_none")]
    reproduced: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scenario: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_divergence: Option<String>,
    diff_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diffs: Vec<DiffEntry>,
    /// Paths where two vanilla runs disagreed, ignored for the case (a `noisy` case
    /// differs nowhere else).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    noisy_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Differences kept per case in the report.
const MAX_REPORTED: usize = 30;
/// Default number of shrink attempts (each runs both servers once).
const DEFAULT_SHRINK_RUNS: usize = 40;

pub fn main(args: &[String]) -> Result<ExitCode> {
    let parse = |flag: &str| -> Result<Option<u64>> {
        flag_value(args, flag)
            .map(|v| v.parse::<u64>())
            .transpose()
            .with_context(|| format!("{flag} takes a number"))
    };
    let seed = match parse("--seed")? {
        Some(s) => s,
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let steps = parse("--steps")?.unwrap_or(20) as usize;
    let cases = parse("--cases")?.unwrap_or(1).max(1);
    let shrink_runs = parse("--shrink-runs")?.map_or(DEFAULT_SHRINK_RUNS, |n| n as usize);
    let v5 = match parse("--protocol")? {
        None | Some(47) => false,
        Some(5) => true,
        Some(p) => bail!("--protocol takes 47 or 5, not {p}"),
    };
    if steps == 0 {
        bail!("--steps must be at least 1");
    }
    let vanilla = super::vanilla_target().context(
        "parity fuzz compares against live vanilla: set KROL_VANILLA_JAR (and KROL_JAVA)",
    )?;
    let krol = build_krol()?;
    let pair = Pair {
        krol: &krol,
        vanilla: &vanilla,
        v5,
    };

    let mut reports = Vec::new();
    for seed in seed..seed + cases {
        eprintln!("fuzz: seed {seed}, {steps} steps ...");
        let report = fuzz_case(&pair, seed, steps, shrink_runs).unwrap_or_else(|e| CaseReport {
            seed,
            status: "error".into(),
            steps,
            shrunk_steps: None,
            reproduced: None,
            scenario: None,
            first_divergence: None,
            diff_count: 0,
            diffs: Vec::new(),
            noisy_paths: Vec::new(),
            error: Some(format!("{e:#}")),
        });
        let extra = match report.status.as_str() {
            "fail" => format!(
                " ({} diffs, first at {}; shrunk to {} steps{}: {})",
                report.diff_count,
                report.first_divergence.as_deref().unwrap_or("?"),
                report.shrunk_steps.unwrap_or(steps),
                if report.reproduced == Some(false) {
                    ", did NOT reproduce"
                } else {
                    ""
                },
                report.scenario.as_deref().unwrap_or("?")
            ),
            "noisy" => format!(
                " (vanilla disagreed with itself at {} paths, first {})",
                report.noisy_paths.len(),
                report.noisy_paths.first().map_or("?", String::as_str)
            ),
            "error" => format!(" ({})", report.error.as_deref().unwrap_or("?")),
            _ => String::new(),
        };
        println!("{:<6} seed {seed}{extra}", report.status.to_uppercase());
        reports.push(report);
    }

    let count = |s: &str| reports.iter().filter(|r| r.status == s).count();
    let summary = format!(
        "{} cases: {} pass, {} fail, {} noisy, {} error",
        reports.len(),
        count("pass"),
        count("fail"),
        count("noisy"),
        count("error")
    );
    let path = parity_root().join(if v5 {
        "fuzz-report-v5.json"
    } else {
        "fuzz-report.json"
    });
    let report = serde_json::json!({ "summary": summary, "cases": reports });
    std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;
    println!("\n{summary}\nreport: {}", path.display());
    Ok(if count("fail") + count("error") > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn fuzz_case(pair: &Pair, seed: u64, count: usize, shrink_runs: usize) -> Result<CaseReport> {
    let name = format!("fuzz/seed-{seed}{}", if pair.v5 { "-v5" } else { "" });
    let steps = generate(seed, count);
    let (sc, _) = scenario(&steps, &name, &[])?;
    let (vanilla, krol) = pair.run(&sc)?;
    let diffs = compare(&sc, vanilla.clone(), krol);
    let mut report = CaseReport {
        seed,
        status: "pass".into(),
        steps: count,
        shrunk_steps: None,
        reproduced: None,
        scenario: None,
        first_divergence: None,
        diff_count: 0,
        diffs: Vec::new(),
        noisy_paths: Vec::new(),
        error: None,
    };
    if diffs.is_empty() {
        return Ok(report);
    }

    // A second vanilla run tells krol's bugs apart from what the harness cannot pin:
    // where vanilla disagrees with itself is ignored from here on.
    let noise = compare(&sc, vanilla, pair.vanilla_again(&sc)?);
    let noisy = noisy_bases(&noise);
    let diffs: Vec<Difference> = diffs
        .into_iter()
        .filter(|d| !noisy.iter().any(|n| super::diff::under(&d.path, n)))
        .collect();
    report.noisy_paths = noisy.clone();
    if diffs.is_empty() {
        report.status = "noisy".into();
        return Ok(report);
    }

    eprintln!("fuzz: seed {seed} fails ({} diffs); shrinking", diffs.len());
    let shrunk = shrink(steps, shrink_runs, |candidate| {
        eprintln!("fuzz:   trying {} steps", candidate.len());
        // A candidate that cannot be run counts as passing: it proves nothing.
        pair.diffs(candidate, &name, &noisy)
            .map(|d| !d.is_empty())
            .unwrap_or(false)
    });

    // Run the result once more, so a flaky failure is not written up as a bug.
    let final_diffs = pair.diffs(&shrunk, &name, &noisy)?;
    let reproduced = !final_diffs.is_empty();
    report.status = "fail".into();
    report.shrunk_steps = Some(shrunk.len());
    report.reproduced = Some(reproduced);
    let shown = if reproduced { final_diffs } else { diffs };
    let file = write_scenario(seed, pair.v5, count, &shrunk, &shown, &noisy)?;
    report.scenario = Some(file.display().to_string());
    fill_diffs(&mut report, shown);
    Ok(report)
}

/// The paths to ignore for differences between two vanilla runs: list element diffs
/// (`Motion[+1]`, `Motion[0]`) ignore the whole list.
fn noisy_bases(noise: &[Difference]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in noise {
        let mut base = d.path.as_str();
        while base.ends_with(']')
            && let Some(open) = base.rfind('[')
        {
            base = &base[..open];
        }
        if !base.is_empty() && !out.iter().any(|o| o == base) {
            out.push(base.to_owned());
        }
    }
    out
}

fn fill_diffs(report: &mut CaseReport, diffs: Vec<Difference>) {
    report.first_divergence = diffs.first().map(|d| d.path.clone());
    report.diff_count = diffs.len();
    report.diffs = diffs
        .into_iter()
        .take(MAX_REPORTED)
        .map(DiffEntry::from)
        .collect();
}

/// Delta debugging (ddmin): removes chunks of steps while `fails` still holds, trying at
/// most `budget` candidates. Returns the smallest failing sequence found.
fn shrink(
    mut steps: Vec<FuzzStep>,
    mut budget: usize,
    mut fails: impl FnMut(&[FuzzStep]) -> bool,
) -> Vec<FuzzStep> {
    let mut n = 2usize;
    while steps.len() >= 2 && budget > 0 {
        let chunk = steps.len().div_ceil(n);
        let mut reduced = false;
        let mut start = 0;
        while start < steps.len() && budget > 0 {
            let end = (start + chunk).min(steps.len());
            let candidate: Vec<FuzzStep> = steps[..start]
                .iter()
                .chain(&steps[end..])
                .cloned()
                .collect();
            budget -= 1;
            if !candidate.is_empty() && fails(&candidate) {
                steps = candidate;
                n = n.saturating_sub(1).max(2);
                reduced = true;
                break;
            }
            start = end;
        }
        if !reduced {
            if n >= steps.len() {
                break;
            }
            n = (n * 2).min(steps.len());
        }
    }
    steps
}

fn write_scenario(
    seed: u64,
    v5: bool,
    original: usize,
    steps: &[FuzzStep],
    diffs: &[Difference],
    noisy: &[String],
) -> Result<PathBuf> {
    let mut header = vec![
        format!(
            "Found by `cargo xtask parity fuzz --seed {seed} --steps {original}{}`, shrunk to {} steps.",
            if v5 { " --protocol 5" } else { "" },
            steps.len()
        ),
        "Record it (`cargo xtask parity record --filter fuzz/`), fix krol, and keep it as a".into(),
        "regression scenario; rename it after what it covers once understood.".into(),
        format!("First differences ({} in all):", diffs.len()),
    ];
    for d in diffs.iter().take(5) {
        let mut line = format!("  {}: vanilla {} / krol {}", d.path, d.expected, d.actual);
        if line.len() > 200 {
            let mut cut = 197;
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            line.truncate(cut);
            line.push_str("...");
        }
        header.push(line.replace('\n', " "));
    }
    let text = render(steps, &header, noisy);
    let dir = parity_root().join("scenarios").join("fuzz");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("seed-{seed}{}.toml", if v5 { "-v5" } else { "" }));
    std::fs::write(&path, text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_reproducible() {
        assert_eq!(generate(7, 30), generate(7, 30));
        assert_ne!(generate(7, 30), generate(8, 30));
    }

    #[test]
    fn rendered_cases_parse_as_scenarios() {
        for seed in 0..20 {
            let steps = generate(seed, 25);
            let noisy = vec!["entities.-30,-28[0].Motion".to_owned()];
            let (sc, _) = scenario(&steps, "fuzz/test", &noisy).expect("valid scenario");
            let Some(Step::Play { moves, chat, .. }) =
                sc.steps.iter().find(|s| matches!(s, Step::Play { .. }))
            else {
                panic!("no play step");
            };
            assert_eq!(moves.len(), LEAD_TICKS + steps.len() * STEP_TICKS);
            // One line per step at most (two for a survival dig), and the closing of the
            // window after the last.
            let digs = steps
                .iter()
                .filter(|s| matches!(s, FuzzStep::Dig { end, .. } if *end != DigEnd::None))
                .count();
            assert!(chat.len() <= steps.len() + digs + 1);
        }
    }

    #[test]
    fn walks_stay_in_bounds_and_within_one_packet() {
        /// The longest walk, squared: one position packet may move up to 10 blocks before
        /// "moved too quickly" (the walk area's diagonal with a 4 block drop is about 8.2).
        const MAX_WALK_SQ: f64 = 100.0;
        let steps = generate(3, 200);
        let mut here = ORIGIN;
        for s in &steps {
            if let FuzzStep::Walk { to } = s {
                assert!((WALK_X.0..=WALK_X.1).contains(&to[0]));
                assert!((WALK_Z.0..=WALK_Z.1).contains(&to[2]));
                let d: f64 = (0..3).map(|k| (to[k] - here[k]).powi(2)).sum();
                assert!(d < MAX_WALK_SQ);
                here = *to;
            }
        }
    }

    #[test]
    fn survival_players_stay_grounded_and_dig_slowly() {
        const { assert!(ACTION_TICK + DIG_FINISH_TICKS < STEP_TICKS + ACTION_TICK) };
        for seed in 0..20 {
            let mut survival = false;
            let mut y = ORIGIN[1];
            for s in generate(seed, 200) {
                match s {
                    FuzzStep::Gamemode(mode) => {
                        survival = mode == 0;
                        if survival {
                            assert_eq!(y, GROUND_Y, "seed {seed}: switched in the air");
                        }
                    }
                    FuzzStep::Walk { to } => {
                        y = to[1];
                        if survival {
                            assert!(SURVIVAL_WALK_Y.contains(&y));
                        }
                    }
                    FuzzStep::Dig { end, .. } => assert_eq!(end == DigEnd::None, !survival),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn shrinking_finds_the_failing_pair() {
        let steps: Vec<FuzzStep> = (0..16).map(|slot| FuzzStep::Hold { slot }).collect();
        let culprit = |s: &[FuzzStep]| {
            s.contains(&FuzzStep::Hold { slot: 3 }) && s.contains(&FuzzStep::Hold { slot: 11 })
        };
        let shrunk = shrink(steps, 100, culprit);
        assert_eq!(
            shrunk,
            vec![FuzzStep::Hold { slot: 3 }, FuzzStep::Hold { slot: 11 }]
        );
    }
}
