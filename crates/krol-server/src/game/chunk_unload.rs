//! Unloading chunks nobody watches (vanilla `ChunkProviderServer.dropChunk` and
//! `unloadQueuedChunks`).
//!
//! A chunk is dropped when its last watching player leaves it (moving away, logging out,
//! respawning elsewhere) and, at every save, when no player watches it at all. Dropped
//! chunks within 128 blocks of the spawn point stay. Each tick, before the scheduled
//! block ticks, up to 100 dropped chunks unload in the drop set's order (a Java
//! `ConcurrentHashMap` key set, see `krol_java::JavaConcurrentLongSet`): the chunk is
//! saved with its entities, which leave the game, and forgotten.

use super::Game;

/// Most chunks `unloadQueuedChunks` unloads per tick.
const UNLOADS_PER_TICK: usize = 100;

/// Half the side, in blocks, of the area around an entity whose chunks must all be
/// loaded for it to update (`World.updateEntityWithOptionalForce`).
const ENTITY_UPDATE_RANGE: i32 = 32;

/// Ticks entities and tile entities keep updating once no player is in the world
/// (`WorldServer.updateEntities`).
pub(super) const IDLE_UPDATE_TICKS: u32 = 1200;

impl Game {
    /// `ChunkProviderServer.unloadQueuedChunks`, skipped while saving is off
    /// (`disableLevelSaving`).
    pub(super) fn unload_queued_chunks(&mut self) {
        if !self.saving_enabled {
            return;
        }
        // Entities of chunks loaded this tick exist from their load in vanilla.
        self.load_chunk_entities();
        let mut unloaded = false;
        for _ in 0..UNLOADS_PER_TICK {
            let Some((cx, cz)) = self.level.overworld.next_dropped() else {
                break;
            };
            if self.level.overworld.is_chunk_loaded(cx, cz) {
                self.unload_chunk(cx, cz);
                unloaded = true;
            }
        }
        if unloaded {
            self.level.overworld.flush_regions();
        }
    }

    /// Unloads one chunk: saved with its entities, which leave the game along with the
    /// state its tile entities only had while loaded.
    fn unload_chunk(&mut self, cx: i32, cz: i32) {
        let (ids, entities) = self.chunk_saved_entities((cx, cz));
        for id in ids {
            self.forget_entity(id);
        }
        let in_chunk = |p: &(i32, i32, i32)| (p.0 >> 4, p.2 >> 4) == (cx, cz);
        self.chests.retain(|c| !in_chunk(&c.pos));
        self.note_powered.retain(|p, _| !in_chunk(p));
        self.moving_progress.retain(|p, _| !in_chunk(p));
        self.sign_editors.retain(|p, _| !in_chunk(p));
        self.furnace_burn.retain(|p, _| !in_chunk(p));
        self.brewing_ingredient.retain(|p, _| !in_chunk(p));
        let time = self.level.data.time();
        self.level.overworld.unload_chunk(cx, cz, entities, time);
    }

    fn spawn_column(&self) -> (i32, i32) {
        (self.level.data.spawn_x(), self.level.data.spawn_z())
    }

    /// `PlayerInstance.removePlayer` for chunks player `i` stops watching: a chunk no
    /// other player watches is dropped.
    pub(super) fn drop_unwatched(&mut self, i: usize, chunks: &[(i32, i32)]) {
        let spawn = self.spawn_column();
        for &c in chunks {
            let watched = self
                .players
                .iter()
                .enumerate()
                .any(|(k, p)| k != i && p.watching.contains(&c));
            if !watched {
                self.level.overworld.drop_chunk(c.0, c.1, spawn);
            }
        }
    }

    /// `PlayerManager.removePlayer`: player `i` stops watching all its chunks.
    pub(super) fn stop_watching_all(&mut self, i: usize) -> Vec<(i32, i32)> {
        let radius = self.config.view_distance;
        let chunks = self.players[i].watched_in_order(radius);
        self.players[i].watching.clear();
        self.players[i].chunk_queue.clear();
        self.drop_unwatched(i, &chunks);
        chunks
    }

    /// The end of `WorldServer.saveAllChunks`: every loaded chunk no player watches is
    /// dropped, in load order.
    pub(super) fn drop_unwatched_chunks(&mut self) {
        let spawn = self.spawn_column();
        for (cx, cz) in self.level.overworld.loaded_in_order() {
            if !self.players.iter().any(|p| p.watching.contains(&(cx, cz))) {
                self.level.overworld.drop_chunk(cx, cz, spawn);
            }
        }
    }

    /// `World.isAreaLoaded` around an entity: whether it updates this tick.
    pub(super) fn entity_area_loaded(&self, pos: (f64, f64, f64)) -> bool {
        let x = krol_java::math::floor_f64(pos.0);
        let z = krol_java::math::floor_f64(pos.2);
        let w = &self.level.overworld;
        let (x0, x1) = (
            (x - ENTITY_UPDATE_RANGE) >> 4,
            (x + ENTITY_UPDATE_RANGE) >> 4,
        );
        let (z0, z1) = (
            (z - ENTITY_UPDATE_RANGE) >> 4,
            (z + ENTITY_UPDATE_RANGE) >> 4,
        );
        (x0..=x1).all(|cx| (z0..=z1).all(|cz| w.is_chunk_loaded(cx, cz)))
    }

    /// Whether a non-player entity is marked dead.
    pub(super) fn entity_dead(&self, id: i32) -> bool {
        self.items.get(&id).is_some_and(|e| e.dead)
            || self.orbs.get(&id).is_some_and(|e| e.dead)
            || self.falling.get(&id).is_some_and(|e| e.dead)
            || self.tnt.get(&id).is_some_and(|e| e.dead)
            || self.thrown.get(&id).is_some_and(|e| e.dead)
            || self.arrows.get(&id).is_some_and(|e| e.dead)
            || self.hanging.get(&id).is_some_and(|e| e.dead)
            || self.armor_stands.get(&id).is_some_and(|e| e.dead)
            || self.minecarts.get(&id).is_some_and(|e| e.dead)
            || self.boats.get(&id).is_some_and(|e| e.dead)
            || self.mobs.get(&id).is_some_and(|e| e.dead)
    }
}
