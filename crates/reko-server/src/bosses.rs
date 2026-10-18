//! One natural rare-boss instance per zone; each server start resets death timers.
use crate::{movement::Position, population::Spawn};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(include_str!("bosses.sql"))
}
#[derive(Clone)]
struct State {
    entity: u16,
    position: Position,
    due: u64,
}
struct Pool {
    name: String,
    zone: u8,
    delay: u64,
    candidates: Vec<Spawn>,
    state: State,
}
pub(super) struct Scheduler {
    pools: HashMap<String, Pool>,
    members: HashMap<u16, String>,
    rng: u64,
}
fn random(rng: &mut u64) -> u64 {
    *rng = rng
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *rng
}
fn choose(candidates: &[Spawn], rng: &mut u64) -> State {
    // Multiple source ordinals in one rectangle must not weight that location.
    let locations: Vec<_> = candidates
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            !candidates[..*i]
                .iter()
                .any(|prior| prior.bounds == s.bounds)
        })
        .map(|(_, s)| s)
        .collect();
    let s = locations[random(rng) as usize % locations.len()];
    let mut p = s.position;
    p.x = crate::population::coordinate(s.bounds[0], s.bounds[2], random(rng));
    p.z = crate::population::coordinate(s.bounds[1], s.bounds[3], random(rng));
    State {
        entity: s.id,
        position: p,
        due: 0,
    }
}
impl Scheduler {
    pub fn load(db: &Connection, definitions: &[Spawn], now: u64) -> anyhow::Result<Self> {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='world_boss_pools')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(Self {
                pools: HashMap::new(),
                members: HashMap::new(),
                rng: now.max(1),
            });
        }
        let mut result = Self {
            pools: HashMap::new(),
            members: HashMap::new(),
            rng: now.max(1),
        };
        let by_id: HashMap<_, _> = definitions
            .iter()
            .filter(|s| s.monster)
            .map(|s| (s.id, s))
            .collect();
        let mut q = db.prepare(
            "SELECT pool_id,zone,name,respawn_ms FROM world_boss_pools ORDER BY pool_id",
        )?;
        let configurations = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u8>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(q);
        for (key, zone, name, delay) in configurations {
            let ids = db
                .prepare(
                    "SELECT entity_id FROM world_boss_members WHERE pool_id=?1 ORDER BY entity_id",
                )?
                .query_map([&key], |r| r.get::<_, u16>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let candidates: Vec<_> = ids
                .iter()
                .filter_map(|id| by_id.get(id).copied().cloned())
                .collect();
            if candidates.is_empty() {
                continue;
            }
            anyhow::ensure!(
                delay > 0 && candidates.iter().all(|s| s.position.zone == zone),
                "invalid boss pool {key}"
            );
            let state = choose(&candidates, &mut result.rng);
            for candidate in &candidates {
                anyhow::ensure!(
                    result.members.insert(candidate.id, key.clone()).is_none(),
                    "duplicate boss membership"
                );
            }
            result.pools.insert(
                key,
                Pool {
                    name,
                    zone,
                    delay,
                    candidates,
                    state,
                },
            );
        }
        tracing::info!(
            pools = result.pools.len(),
            candidates = result.members.len(),
            "rare boss pools loaded"
        );
        Ok(result)
    }
    pub fn members(&self) -> HashSet<u16> {
        self.members.keys().copied().collect()
    }
    pub fn active(&self) -> Vec<Spawn> {
        self.pools
            .values()
            .filter(|p| p.state.due == 0)
            .map(|p| {
                let mut s = p
                    .candidates
                    .iter()
                    .find(|s| s.id == p.state.entity)
                    .unwrap()
                    .clone();
                s.position = p.state.position;
                s
            })
            .collect()
    }
    pub fn death(&mut self, id: u16, now: u64) -> anyhow::Result<bool> {
        let Some(key) = self.members.get(&id) else {
            return Ok(false);
        };
        let pool = self.pools.get_mut(key).unwrap();
        anyhow::ensure!(
            pool.state.entity == id && pool.state.due == 0,
            "inactive boss cannot die twice"
        );
        let state = State {
            due: now.saturating_add(pool.delay),
            ..pool.state.clone()
        };
        pool.state = state;
        tracing::info!(
            pool = key,
            entity = id,
            respawn_at = pool.state.due,
            "natural boss death scheduled"
        );
        Ok(true)
    }
    pub fn tick(&mut self, now: u64) -> Vec<Spawn> {
        let mut spawned = Vec::new();
        for (key, pool) in &mut self.pools {
            if pool.state.due == 0 || now < pool.state.due {
                continue;
            }
            let state = choose(&pool.candidates, &mut self.rng);
            pool.state = state;
            let mut s = pool
                .candidates
                .iter()
                .find(|s| s.id == pool.state.entity)
                .unwrap()
                .clone();
            s.position = pool.state.position;
            spawned.push(s);
            tracing::info!(
                pool = key,
                entity = pool.state.entity,
                "natural boss respawned"
            );
        }
        spawned
    }
    pub fn status(&self, zone: u8, query: &str, now: u64) -> Vec<String> {
        let query = query.to_ascii_lowercase();
        let mut pools: Vec<_> = self
            .pools
            .values()
            .filter(|p| p.zone == zone && p.name.to_ascii_lowercase().contains(&query))
            .collect();
        pools.sort_by_key(|p| &p.name);
        pools
            .into_iter()
            .map(|p| {
                if p.state.due == 0 {
                    format!(
                        "{}: alive at {},{} (entity {}).",
                        p.name,
                        p.state.position.x / 10,
                        p.state.position.z / 10,
                        p.state.entity
                    )
                } else {
                    let seconds = p.state.due.saturating_sub(now).div_ceil(1000);
                    format!(
                        "{}: respawn in {:02}:{:02}:{:02}.",
                        p.name,
                        seconds / 3600,
                        seconds / 60 % 60,
                        seconds % 60
                    )
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    #[test]
    fn one_per_zone_six_hour_death_timer_and_restart_resets_all_pools() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("reko-boss-{}-{unique}.db", std::process::id()));
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "PRAGMA foreign_keys=ON; CREATE TABLE world_spawns(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        initialize(&db).unwrap();
        let mut definitions = Vec::new();
        for zone in [11u8, 12] {
            let key = format!("{zone}:talos");
            db.execute(
                "INSERT INTO world_boss_pools VALUES (?1,?2,'Talos',21600000)",
                params![key, zone],
            )
            .unwrap();
            for (index, x) in [(0, 1000), (1, 2000)] {
                let id = 12000 + u16::from(zone) * 10 + index;
                db.execute("INSERT INTO world_spawns VALUES (?1)", [id])
                    .unwrap();
                db.execute(
                    "INSERT INTO world_boss_members VALUES (?1,?2)",
                    params![id, key],
                )
                .unwrap();
                definitions.push(Spawn {
                    id,
                    monster: true,
                    position: Position {
                        zone,
                        x,
                        z: 1000,
                        y: 0,
                    },
                    prototype: 2405,
                    appearance: 2400,
                    name: "Talos".into(),
                    kind: 0,
                    group: 0,
                    nation: 0,
                    level: 80,
                    size: 100,
                    weapons: [0; 2],
                    direction: 0,
                    bounds: [x, 1000, x + 100, 1100],
                    hp: 100,
                    armor: 0,
                    resistance: [0; 6],
                    respawn_ms: 1000,
                    regen_type: 0,
                    move_type: 1,
                });
            }
        }
        let mut scheduler = Scheduler::load(&db, &definitions, 1000).unwrap();
        assert_eq!(scheduler.active().len(), 2);
        let first = scheduler
            .active()
            .into_iter()
            .find(|s| s.position.zone == 11)
            .unwrap();
        assert!(scheduler.death(first.id, 2000).unwrap());
        assert!(scheduler.death(first.id, 3000).is_err());
        assert_eq!(scheduler.active().len(), 1);
        let due = 2000 + 21_600_000;
        assert!(scheduler.tick(due - 1).is_empty());
        assert_eq!(scheduler.tick(due).len(), 1);
        let active = scheduler
            .active()
            .into_iter()
            .find(|s| s.position.zone == 11)
            .unwrap();
        scheduler.death(active.id, due).unwrap();
        drop(scheduler);
        let mut scheduler = Scheduler::load(&db, &definitions, due + 1).unwrap();
        assert_eq!(
            scheduler.active().len(),
            2,
            "restart resets the six-hour cooldown"
        );
        assert!(
            scheduler.tick(due + 2).is_empty(),
            "restart must not duplicate the living bosses"
        );
        let mut visited = HashSet::new();
        let mut now = due;
        for _ in 0..20 {
            let active = scheduler
                .active()
                .into_iter()
                .find(|s| s.position.zone == 11)
                .unwrap();
            let source = definitions.iter().find(|s| s.id == active.id).unwrap();
            assert!((source.bounds[0]..=source.bounds[2]).contains(&active.position.x));
            visited.insert(active.id);
            scheduler.death(active.id, now).unwrap();
            now += 21_600_000;
            assert_eq!(scheduler.tick(now).len(), 1);
        }
        assert_eq!(
            visited.len(),
            2,
            "different candidate locations must be selectable"
        );
        assert_eq!(scheduler.active().len(), 2);
        let active = scheduler
            .active()
            .into_iter()
            .find(|s| s.position.zone == 11)
            .unwrap();
        scheduler.death(active.id, now).unwrap();
        drop(scheduler);
        let scheduler = Scheduler::load(&db, &definitions, now + 21_600_001).unwrap();
        assert_eq!(
            scheduler.active().len(),
            2,
            "an overdue restart creates only one replacement"
        );
        drop(scheduler);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
