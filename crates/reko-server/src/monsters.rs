//! Combat fixtures and imported historical spawn rectangles/respawn timers.

use crate::{movement::Position, skills};
use reko_assets::{CellValue, Table};
use std::{collections::HashMap, path::Path, sync::Mutex};
#[path = "npc_ai.rs"]
mod ai;

#[derive(Clone)]
pub(crate) struct Event {
    pub zone: u8,
    pub packet: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct ExperienceCredit {
    pub prototype: u16,
    pub level: u8,
    pub position: Position,
    pub damage: u32,
    pub total_damage: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_monsters_use_database_timers_and_rectangles_for_hits_and_dots() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        let world = World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        assert_eq!(
            world.spawns(21).len(),
            4,
            "three training fixtures must remain"
        );
        let position = world.inner.lock().unwrap().mobs[&12000].position;
        let attack = || Attack {
            position,
            range: 2.0,
            attack: 0,
            percent: 100,
            extra: 0,
            skill: true,
            drain_percent: 100,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        world
            .hit_owned(
                12000,
                Attack {
                    drain_percent: 25,
                    ..attack()
                },
                Some("OtherOwner"),
                || Ok(()),
            )
            .unwrap();
        assert!(
            world
                .hit_owned(12000, attack(), Some("QuestOwner"), || Ok(()))
                .unwrap()
                .dead
        );
        assert!(world.take_quest_kills("OtherOwner").is_empty());
        assert_eq!(world.take_quest_kills("QuestOwner"), [100]);
        assert!(world.take_quest_kills("QuestOwner").is_empty());
        let credits = world.take_experience("QuestOwner");
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].prototype, 100);
        assert_eq!((credits[0].damage, credits[0].total_damage), (75, 100));
        let other = world.take_experience("OtherOwner");
        assert_eq!((other[0].damage, other[0].total_damage), (25, 100));
        assert!(world.take_experience("QuestOwner").is_empty());
        assert!(
            world
                .hit_owned(12000, attack(), Some("QuestOwner"), || Ok(()))
                .is_err()
        );
        assert!(world.take_experience("QuestOwner").is_empty());
        let deadline = world.inner.lock().unwrap().mobs[&12000].respawn;
        assert!(deadline >= skills::now_ms() + 19900);
        world.tick(deadline - 1);
        assert_eq!(world.spawns(21).len(), 3);
        let events = world.tick(deadline);
        assert_eq!(
            events
                .iter()
                .filter(|e| e.packet.starts_with(&[0x0a, 1]))
                .count(),
            1
        );
        let inner = world.inner.lock().unwrap();
        let mob = &inner.mobs[&12000];
        assert_eq!(mob.hp, 100);
        assert!((3200..=3300).contains(&mob.position.x) && (3900..=4000).contains(&mob.position.z));
        let position = mob.position;
        drop(inner);
        let now = skills::now_ms();
        world
            .hit_owned(
                12000,
                Attack {
                    position,
                    range: 2.0,
                    attack: 0,
                    percent: 100,
                    extra: 0,
                    skill: true,
                    drain_percent: 0,
                    arrow: false,
                    penetrate: false,
                    magic: Some(MagicDamage {
                        element: 4,
                        first: 0,
                        over_time: 100,
                        duration: 4,
                        charisma: 60,
                        bonus: 0,
                        raw_over_time: true,
                        raw_first: false,
                    }),
                },
                Some("DotOwner"),
                || Ok(()),
            )
            .unwrap();
        world.tick(now + 2100);
        world.tick(now + 4200);
        assert_eq!(world.inner.lock().unwrap().mobs[&12000].hp, 0);
        assert_eq!(world.take_experience("DotOwner").len(), 1);
        assert!(world.take_experience("DotOwner").is_empty());
        assert_eq!(
            world.inner.lock().unwrap().mobs[&12000].respawn,
            now + 24200
        );
        let spell = MagicDamage {
            element: 1,
            first: 1000,
            over_time: 0,
            duration: 0,
            charisma: 200,
            bonus: 0,
            raw_first: false,
            raw_over_time: false,
        };
        assert!(spell.sample(1000, 7, 100) < spell.sample(1000, 7, 0));
    }
    #[test]
    fn original_client_spawn_layout_and_damage_authority() {
        let mob = Mob {
            id: 10000,
            resource: 100,
            name: "Target".into(),
            position: Position {
                zone: 21,
                x: 3250,
                z: 4000,
                y: 0,
            },
            level: 80,
            hp: 25000,
            max_hp: 25000,
            armor: 0,
            respawn: 0,
            next_regen: skills::now_ms() + 10000,
            dots: Vec::new(),
            speed_percent: 100,
            slow_until: 0,
            modifiers: HashMap::new(),
            definition: None,
            damage_by: HashMap::new(),
        };
        let packet = mob.spawn();
        assert_eq!(packet.len(), 48);
        assert_eq!(&packet[..9], &[0x0a, 1, 0x10, 0x27, 150, 0, 100, 0, 0]);
        assert_eq!(packet[23], 6);
        assert_eq!(&packet[24..30], b"Target");
        let world = World {
            inner: Mutex::new(Inner {
                mobs: HashMap::from([(mob.id, mob)]),
                rng: 1,
            }),
            events: tokio::sync::broadcast::channel(16).0,
            loot: None,
            ai: Mutex::new(ai::Brain::default()),
            quest_kills: Mutex::new(HashMap::new()),
            experience: Mutex::new(HashMap::new()),
        };
        let attack = || Attack {
            position: Position {
                zone: 21,
                x: 3250,
                z: 4000,
                y: 0,
            },
            range: 1.0,
            attack: 131,
            percent: 600,
            extra: 100,
            skill: true,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        assert!(world.hit(10000, attack(), || Err("No mana")).is_err());
        let hp = world.health(10000, 21).unwrap();
        assert_eq!(&hp[8..12], &25000u32.to_le_bytes());
        let hit = world.hit(10000, attack(), || Ok(())).unwrap();
        assert!((754..=950).contains(&hit.damage));
        let mut wrong_zone = attack();
        wrong_zone.position.zone = 1;
        assert!(
            world
                .hit(10000, wrong_zone, || panic!(
                    "must not debit invalid target"
                ))
                .is_err()
        );
        let mut approached = attack();
        approached.position.x = 3229;
        approached.position.z = 4005;
        assert!(world.hit(10000, approached, || Ok(())).is_ok());
        let mut distant = attack();
        distant.position.x = 3200;
        assert!(
            world
                .hit(10000, distant, || panic!(
                    "out-of-range attack must not spend resources"
                ))
                .is_err()
        );
    }
    #[test]
    fn one_life_source_spawns_do_not_reappear_on_normal_regen_ticks() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        db.execute("UPDATE world_spawns SET regen_type=1", [])
            .unwrap();
        let world = World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let position = world.inner.lock().unwrap().mobs[&12000].position;
        let hit = world
            .hit(
                12000,
                Attack {
                    position,
                    range: 5.0,
                    attack: 1000,
                    percent: 100,
                    extra: 1000,
                    skill: true,
                    drain_percent: 0,
                    arrow: false,
                    penetrate: false,
                    magic: None,
                },
                || Ok(()),
            )
            .unwrap();
        assert!(hit.dead);
        let _ = world.tick(skills::now_ms() + 1000000000);
        assert_eq!(world.inner.lock().unwrap().mobs[&12000].hp, 0);
        assert!(
            !world
                .spawns(21)
                .iter()
                .any(|p| u16::from_le_bytes([p[2], p[3]]) == 12000)
        );
    }
}

#[derive(Debug, Clone)]
struct Mob {
    id: u16,
    resource: u16,
    name: String,
    position: Position,
    level: u8,
    hp: u32,
    max_hp: u32,
    armor: u16,
    respawn: u64,
    next_regen: u64,
    dots: Vec<Dot>,
    speed_percent: u16,
    slow_until: u64,
    modifiers: HashMap<u8, Modifier>,
    definition: Option<crate::population::Spawn>,
    damage_by: HashMap<String, u32>,
}

#[derive(Debug, Clone)]
struct Modifier {
    percent: u16,
    until: u64,
}

impl Mob {
    fn maximum(&self) -> u32 {
        self.modifiers
            .get(&1)
            .filter(|m| skills::now_ms() < m.until)
            .map_or(self.max_hp, |m| {
                (self.max_hp * u32::from(m.percent) / 100).max(1)
            })
    }
    fn defence(&self) -> u32 {
        self.modifiers
            .get(&2)
            .filter(|m| skills::now_ms() < m.until)
            .map_or(u32::from(self.armor), |m| {
                u32::from(self.armor) * u32::from(m.percent) / 100
            })
    }
    fn spawn(&self) -> Vec<u8> {
        let mut p = vec![0x0a, 1];
        p.extend(self.id.to_le_bytes());
        let definition = self.definition.as_ref();
        p.extend(definition.map_or(150, |s| s.prototype).to_le_bytes());
        p.extend(self.resource.to_le_bytes());
        p.push(definition.map_or(0, |s| s.kind));
        p.extend(definition.map_or(0, |s| s.group).to_le_bytes());
        p.extend(definition.map_or(100, |s| s.size).to_le_bytes());
        for weapon in definition.map_or([0; 2], |s| s.weapons) {
            p.extend(weapon.to_le_bytes());
        }
        let name: Vec<u8> = self.name.chars().map(|c| c as u8).collect();
        p.push(name.len() as u8);
        p.extend(name);
        p.extend([definition.map_or(0, |s| s.nation), self.level]);
        p.extend(self.position.x.to_le_bytes());
        p.extend(self.position.z.to_le_bytes());
        p.extend(self.position.y.to_le_bytes());
        p.extend([0; 9]);
        p.push(definition.map_or(0, |s| s.direction));
        p
    }
    fn respawn_delay(&self) -> u64 {
        self.definition.as_ref().map_or(1000, |s| s.respawn_ms)
    }
    fn respawn_at(&self, now: u64) -> u64 {
        if self.definition.as_ref().is_some_and(|s| s.regen_type == 1) {
            u64::MAX
        } else {
            now.saturating_add(self.respawn_delay())
        }
    }
    fn resistance(&self, element: u8) -> u16 {
        self.definition
            .as_ref()
            .and_then(|s| {
                element
                    .checked_sub(1)
                    .and_then(|e| s.resistance.get(usize::from(e)).copied())
            })
            .unwrap_or(0)
    }
    fn health(&self, delta: i16, immediate: bool) -> Vec<u8> {
        let mut p = vec![0x22];
        p.extend(self.id.to_le_bytes());
        p.push(u8::from(immediate));
        p.extend(self.maximum().to_le_bytes());
        p.extend(self.hp.to_le_bytes());
        p.extend(delta.to_le_bytes());
        p
    }
}

struct Inner {
    mobs: HashMap<u16, Mob>,
    rng: u64,
}

pub(crate) struct World {
    quest_kills: Mutex<HashMap<String, Vec<u16>>>,
    experience: Mutex<HashMap<String, Vec<ExperienceCredit>>>,
    ai: Mutex<ai::Brain>,
    pub loot: Option<std::sync::Arc<crate::loot::Book>>,
    inner: Mutex<Inner>,
    pub events: tokio::sync::broadcast::Sender<Event>,
}

#[derive(Debug)]
pub(crate) struct Hit {
    pub damage: u16,
    pub dead: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Curse {
    pub category: u8,
    pub percent: u16,
    pub duration_ms: u64,
}

pub(crate) struct Attack {
    pub position: Position,
    pub range: f32,
    pub attack: u16,
    pub percent: u16,
    pub extra: u16,
    pub skill: bool,
    pub drain_percent: u8,
    pub arrow: bool,
    pub penetrate: bool,
    pub magic: Option<MagicDamage>,
}

#[derive(Debug, Clone)]
struct Dot {
    owner: Option<String>,
    amount: u16,
    remaining: u8,
    next: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct MagicDamage {
    pub element: u8,
    pub first: u16,
    pub over_time: u16,
    pub duration: u8,
    pub charisma: u8,
    pub bonus: u16,
    pub raw_over_time: bool,
    pub raw_first: bool,
}

impl MagicDamage {
    fn sample(self, raw: u16, random: u64, resistance: u16) -> u16 {
        if raw == 0 {
            return 0;
        }
        let base =
            u32::from(raw) * u32::from(self.charisma) / 186 * 230 / (250 + u32::from(resistance));
        let roll = (random % (u64::from(base) + 1)) as f32;
        (0.85 * base as f32
            + 0.3 * roll
            + f32::from(self.charisma.saturating_sub(86))
            + f32::from(self.bonus)) as u16
    }
}

impl World {
    pub fn load_ai(&self, db: &rusqlite::Connection) -> anyhow::Result<()> {
        self.ai.lock().unwrap().load(db)
    }
    pub fn observe(&self, s: &crate::world_entry::Session, now: u64) {
        self.ai.lock().unwrap().observe(s, now);
    }
    pub fn incoming(&self, id: u16) -> Vec<ai::Strike> {
        self.ai.lock().unwrap().take(id)
    }
    pub fn forget(&self, id: u16) {
        self.ai.lock().unwrap().remove(id);
    }

    #[cfg(test)]
    pub fn target_attributes(&self, id: u16) -> (u32, u32) {
        let inner = self.inner.lock().unwrap();
        let mob = &inner.mobs[&id];
        (mob.maximum(), mob.defence())
    }
    pub fn curse(
        &self,
        id: u16,
        position: Position,
        range: f32,
        curse: Curse,
        pay: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let mut inner = self.inner.lock().unwrap();
        let mob = inner.mobs.get_mut(&id).ok_or("Unknown combat target.")?;
        if mob.position.zone != position.zone || mob.hp == 0 {
            return Err("The target is unavailable.");
        }
        if f32::from(mob.position.x.abs_diff(position.x))
            .hypot(f32::from(mob.position.z.abs_diff(position.z)))
            > (range + 2.0) * 10.0
        {
            return Err("The target is too far away.");
        }
        if !matches!(curse.category, 1 | 2) || !(1..=100).contains(&curse.percent) {
            return Err("Unsupported NPC curse.");
        }
        pay()?;
        mob.modifiers.insert(
            curse.category,
            Modifier {
                percent: curse.percent,
                until: skills::now_ms() + curse.duration_ms,
            },
        );
        let before = mob.hp;
        mob.hp = mob.hp.min(mob.maximum());
        let event = Event {
            zone: mob.position.zone,
            packet: mob.health(-((before - mob.hp).min(32767) as i16), false),
        };
        drop(inner);
        let _ = self.events.send(event);
        Ok(())
    }
    pub fn validate(&self, id: u16, position: Position, range: f32) -> Result<(), &'static str> {
        let inner = self.inner.lock().unwrap();
        let mob = inner.mobs.get(&id).ok_or("Unknown combat target.")?;
        if mob.position.zone != position.zone || mob.hp == 0 {
            return Err("The target is unavailable.");
        }
        if f32::from(mob.position.x.abs_diff(position.x))
            .hypot(f32::from(mob.position.z.abs_diff(position.z)))
            > (range + 2.0) * 10.0
        {
            return Err("The target is too far away.");
        }
        Ok(())
    }
    pub fn area_targets(&self, center: Position, radius: f32) -> Vec<u16> {
        let mut ids: Vec<_> = self
            .inner
            .lock()
            .unwrap()
            .mobs
            .values()
            .filter(|m| {
                m.hp > 0
                    && m.position.zone == center.zone
                    && f32::from(m.position.x.abs_diff(center.x))
                        .hypot(f32::from(m.position.z.abs_diff(center.z)))
                        <= radius * 10.0
            })
            .map(|m| m.id)
            .collect();
        ids.sort_unstable();
        ids
    }
    pub fn load(game: &Path) -> anyhow::Result<Self> {
        let looks = Table::from_encrypted_bytes(&std::fs::read(game.join("Data/NPC_Looks.tbl"))?)?;
        anyhow::ensure!(
            looks
                .rows
                .iter()
                .any(|row| row.first() == Some(&CellValue::U32(100))),
            "missing training target appearance"
        );
        let mut mobs = HashMap::new();
        for (id, x, z, armor, name) in [
            (10000, 3250, 4000, 0, "Training Target"),
            (10001, 3280, 4030, 240, "Armored Target"),
            (10002, 3250, 4060, 500, "Heavy Armor Target"),
        ] {
            mobs.insert(
                id,
                Mob {
                    id,
                    resource: 100,
                    name: name.into(),
                    position: Position {
                        zone: 21,
                        x,
                        z,
                        y: 0,
                    },
                    level: 80,
                    hp: 25000,
                    max_hp: 25000,
                    armor,
                    respawn: 0,
                    next_regen: skills::now_ms() + 10000,
                    dots: Vec::new(),
                    speed_percent: 100,
                    slow_until: 0,
                    modifiers: HashMap::new(),
                    definition: None,
                    damage_by: HashMap::new(),
                },
            );
        }
        Ok(Self {
            loot: None,
            ai: Mutex::new(ai::Brain::default()),
            quest_kills: Mutex::new(HashMap::new()),
            experience: Mutex::new(HashMap::new()),
            inner: Mutex::new(Inner {
                mobs,
                rng: skills::now_ms().max(1),
            }),
            events: tokio::sync::broadcast::channel(256).0,
        })
    }

    pub fn load_population(&self, db: &rusqlite::Connection, game: &Path) -> anyhow::Result<()> {
        let definitions = crate::population::load(db, game)?;
        let mut inner = self.inner.lock().unwrap();
        for s in definitions.into_iter().filter(|s| s.monster) {
            anyhow::ensure!(
                !inner.mobs.contains_key(&s.id),
                "duplicate world entity {}",
                s.id
            );
            inner.mobs.insert(
                s.id,
                Mob {
                    id: s.id,
                    resource: s.appearance,
                    name: s.name.clone(),
                    position: s.position,
                    level: s.level,
                    hp: s.hp,
                    max_hp: s.hp,
                    armor: s.armor,
                    respawn: 0,
                    next_regen: skills::now_ms() + 10000,
                    dots: Vec::new(),
                    speed_percent: 100,
                    slow_until: 0,
                    modifiers: HashMap::new(),
                    definition: Some(s),
                    damage_by: HashMap::new(),
                },
            );
        }
        tracing::info!(
            monsters = inner.mobs.len(),
            "world combat population loaded"
        );
        Ok(())
    }

    #[cfg(test)]
    pub fn spawns(&self, zone: u8) -> Vec<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        let mut mobs: Vec<_> = inner
            .mobs
            .values()
            .filter(|m| m.position.zone == zone && m.hp > 0)
            .collect();
        mobs.sort_by_key(|m| m.id);
        mobs.into_iter().map(Mob::spawn).collect()
    }

    pub fn nearby_spawns(&self, position: Position) -> Vec<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        let mut mobs: Vec<_> = inner
            .mobs
            .values()
            .filter(|m| m.hp > 0 && crate::population::same_region_view(m.position, position))
            .collect();
        mobs.sort_by_key(|m| m.id);
        mobs.into_iter().map(Mob::spawn).collect()
    }
    pub fn visible_from(&self, id: u16, position: Position) -> bool {
        self.inner
            .lock()
            .unwrap()
            .mobs
            .get(&id)
            .is_some_and(|m| m.hp > 0 && crate::population::same_region_view(m.position, position))
    }

    pub fn slow(&self, id: u16, percent: u16, duration: u64) {
        if let Some(mob) = self.inner.lock().unwrap().mobs.get_mut(&id)
            && mob.hp > 0
        {
            mob.speed_percent = percent;
            mob.slow_until = skills::now_ms() + duration;
        }
    }

    pub fn health(&self, id: u16, zone: u8) -> Option<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        inner
            .mobs
            .get(&id)
            .filter(|m| m.position.zone == zone)
            .map(|m| m.health(0, true))
    }

    #[cfg(test)]
    pub fn hit(
        &self,
        id: u16,
        attack: Attack,
        pay: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<Hit, &'static str> {
        self.hit_owned(id, attack, None, pay)
    }
    pub fn hit_owned(
        &self,
        id: u16,
        attack: Attack,
        owner: Option<&str>,
        pay: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<Hit, &'static str> {
        let Attack {
            position,
            range,
            attack,
            percent,
            extra,
            skill,
            drain_percent,
            arrow,
            penetrate,
            magic,
        } = attack;
        let mut inner = self.inner.lock().unwrap();
        inner.rng ^= inner.rng << 13;
        inner.rng ^= inner.rng >> 7;
        inner.rng ^= inner.rng << 17;
        let random = inner.rng;
        let mob = inner.mobs.get_mut(&id).ok_or("Unknown combat target.")?;
        if mob.position.zone != position.zone || mob.hp == 0 {
            return Err("The target is unavailable.");
        }
        let dx = (f32::from(mob.position.x) - f32::from(position.x)) / 10.0;
        let dz = (f32::from(mob.position.z) - f32::from(position.z)) / 10.0;
        // TODO(re): Resolve each NPC model's collision radius; fixtures use one metre.
        if dx.hypot(dz) > range + 1.0 + 1.0 {
            tracing::info!(
                target = id,
                player_x = position.x,
                player_z = position.z,
                target_x = mob.position.x,
                target_z = mob.position.z,
                distance = dx.hypot(dz),
                limit = range + 2.0,
                "combat range rejected"
            );
            return Err("The target is too far away.");
        }
        pay()?;
        let amount = if drain_percent > 0 {
            mob.hp * u32::from(drain_percent) / 100
        } else {
            let base = if penetrate {
                u32::from(attack)
            } else {
                u32::from(attack) * 200 / (mob.defence() + 240)
            };
            let hit = base * u32::from(percent) / 100;
            let roll = (random % (u64::from(hit) + 1)) as f32;
            let physical = if arrow {
                0.6 * hit as f32 + roll + 0.99
            } else if skill {
                hit as f32 + 0.3 * roll + 0.99
            } else {
                0.85 * hit as f32 + 0.3 * roll
            };
            physical as u32 + u32::from(extra)
        };
        let magic_damage = magic.map_or(0, |magic| {
            if magic.raw_first {
                magic.first
            } else {
                magic.sample(magic.first, random >> 16, mob.resistance(magic.element))
            }
        });
        let damage = (amount + u32::from(magic_damage)).min(32767).min(mob.hp) as u16;
        if let Some(magic) = magic
            && magic.over_time > 0
            && magic.duration >= 2
            && mob.dots.len() < 64
        {
            let count = magic.duration / 2;
            mob.dots.push(Dot {
                owner: owner.map(str::to_owned),
                amount: if magic.raw_over_time {
                    magic.over_time
                } else {
                    magic.sample(magic.over_time, random >> 32, mob.resistance(magic.element))
                } / u16::from(count),
                remaining: count,
                next: skills::now_ms() + 2000,
            });
        }
        mob.hp -= u32::from(damage);
        if let Some(owner) = owner {
            *mob.damage_by.entry(owner.to_owned()).or_default() += u32::from(damage);
        }
        let dead = mob.hp == 0;
        if dead {
            mob.respawn = mob.respawn_at(skills::now_ms());
            mob.dots.clear();
            self.quest_death(mob);
        }
        let health = mob.health(-(damage as i16), false);
        let zone = mob.position.zone;
        let loot_packet = if dead { self.drop(mob) } else { None };
        drop(inner);
        let _ = self.events.send(Event {
            zone,
            packet: health,
        });
        if dead {
            let mut packet = vec![0x11];
            packet.extend(id.to_le_bytes());
            let _ = self.events.send(Event { zone, packet });
        }
        if let Some(packet) = loot_packet {
            let _ = self.events.send(Event { zone, packet });
        }
        Ok(Hit { damage, dead })
    }
    fn quest_death(&self, mob: &Mob) {
        if let Some(definition) = &mob.definition {
            let total_damage = mob.damage_by.values().map(|v| u64::from(*v)).sum();
            let mut credits = self.experience.lock().unwrap();
            for (owner, &damage) in &mob.damage_by {
                if damage > 0 {
                    credits
                        .entry(owner.clone())
                        .or_default()
                        .push(ExperienceCredit {
                            prototype: definition.prototype,
                            level: mob.level,
                            position: mob.position,
                            damage,
                            total_damage,
                        });
                }
            }
        }
        if let (Some(definition), Some((owner, _))) = (
            mob.definition.as_ref(),
            mob.damage_by.iter().max_by_key(|(_, damage)| *damage),
        ) {
            self.quest_kills
                .lock()
                .unwrap()
                .entry(owner.clone())
                .or_default()
                .push(definition.prototype);
        }
    }
    pub fn take_quest_kills(&self, name: &str) -> Vec<u16> {
        self.quest_kills
            .lock()
            .unwrap()
            .remove(name)
            .unwrap_or_default()
    }
    pub fn take_experience(&self, name: &str) -> Vec<ExperienceCredit> {
        self.experience
            .lock()
            .unwrap()
            .remove(name)
            .unwrap_or_default()
    }
    fn drop(&self, mob: &Mob) -> Option<Vec<u8>> {
        let owner = mob.damage_by.iter().max_by_key(|(_, damage)| *damage)?.0;
        self.loot.as_ref()?.create(
            mob.definition.as_ref()?.prototype,
            mob.id,
            mob.position,
            owner,
        )
    }

    pub(crate) fn tick(&self, now: u64) -> Vec<Event> {
        let mut inner = self.inner.lock().unwrap();
        let mut events = Vec::new();
        inner.rng = inner.rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let random = inner.rng;
        for mob in inner.mobs.values_mut() {
            let hp_modified = mob.modifiers.contains_key(&1);
            mob.modifiers.retain(|_, m| now < m.until);
            if hp_modified && !mob.modifiers.contains_key(&1) && mob.hp > 0 {
                events.push(Event {
                    zone: mob.position.zone,
                    packet: mob.health(0, false),
                });
            }
            if mob.speed_percent != 100 && now >= mob.slow_until {
                mob.speed_percent = 100;
                mob.slow_until = 0;
            }

            if mob.hp == 0 && now >= mob.respawn {
                if let Some(s) = mob.definition.as_ref() {
                    let seed =
                        random.wrapping_add(u64::from(mob.id).wrapping_mul(1442695040888963407));
                    mob.position.x = crate::population::coordinate(s.bounds[0], s.bounds[2], seed);
                    mob.position.z = crate::population::coordinate(
                        s.bounds[1],
                        s.bounds[3],
                        seed.rotate_left(29),
                    );
                }
                mob.hp = mob.max_hp;
                mob.damage_by.clear();
                mob.modifiers.clear();
                mob.respawn = 0;
                mob.speed_percent = 100;
                mob.slow_until = 0;
                mob.dots.clear();
                mob.next_regen = now + 10000;
                let mut out = vec![0x0a, 2];
                out.extend(mob.id.to_le_bytes());
                events.push(Event {
                    zone: mob.position.zone,
                    packet: out,
                });
                events.push(Event {
                    zone: mob.position.zone,
                    packet: mob.spawn(),
                });
            } else if mob.hp > 0 {
                let mut loss = 0u32;
                for dot in &mut mob.dots {
                    if now >= dot.next && dot.remaining > 0 {
                        let amount = u32::from(dot.amount).min(mob.hp.saturating_sub(loss));
                        loss += amount;
                        if let Some(owner) = dot.owner.as_ref() {
                            *mob.damage_by.entry(owner.clone()).or_default() += amount;
                        }
                        dot.remaining -= 1;
                        dot.next = now + 2000;
                    }
                }
                mob.dots.retain(|dot| dot.remaining > 0);
                let loss = loss.min(mob.hp).min(32767);
                if loss > 0 {
                    mob.hp -= loss;
                    events.push(Event {
                        zone: mob.position.zone,
                        packet: mob.health(-(loss as i16), false),
                    });
                    if mob.hp == 0 {
                        self.quest_death(mob);
                        mob.dots.clear();
                        mob.respawn = mob.respawn_at(now);
                        let mut death = vec![0x11];
                        death.extend(mob.id.to_le_bytes());
                        events.push(Event {
                            zone: mob.position.zone,
                            packet: death,
                        });
                        if let Some(packet) = self.drop(mob) {
                            events.push(Event {
                                zone: mob.position.zone,
                                packet,
                            });
                        }
                        continue;
                    }
                }
                if now > mob.next_regen {
                    mob.next_regen = now + 10000;
                    let maximum = mob.maximum();
                    let amount = (maximum / 20).min(maximum.saturating_sub(mob.hp));
                    if amount > 0 {
                        mob.hp += amount;
                        events.push(Event {
                            zone: mob.position.zone,
                            packet: mob.health(amount.min(32767) as i16, false),
                        });
                    }
                }
            }
        }
        events.extend(self.ai.lock().unwrap().tick(&mut inner.mobs, now));
        events
    }

    pub async fn run(self: std::sync::Arc<Self>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            interval.tick().await;
            for event in self.tick(skills::now_ms()) {
                let _ = self.events.send(event);
            }
        }
    }
}
