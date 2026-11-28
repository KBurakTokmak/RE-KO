//! Data-driven pursuit, physical attacks and monster spells; owned by the world simulation.
use super::{Event, Mob, Role};
use crate::{combat, movement::Position, npc_magic, skills, world_entry::Session};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet, VecDeque};

/// Owner cap on a guard's monster search radius (the 1453 guards use 14 m).
const GUARD_AGGRO_M: f32 = 14.0;
/// Guards never chase a monster farther than this from their post.
const GUARD_LEASH_M: f32 = 20.0;

#[derive(Clone)]
struct Rule {
    act: u8,
    damage: u16,
    hit: f32,
    delay: u64,
    run: f32,
    walk: f32,
    stand: u64,
    range: f32,
    search: f32,
    trace: f32,
    bulk: f32,
    direct: u8,
    magic: u32,
    mode: u8,
}
#[derive(Clone)]
struct Player {
    name: String,
    nation: u8,
    position: Position,
    hp: u16,
    invisible: bool,
    disguised: bool,
    is_admin: bool,
    stats: combat::Stats,
    at: u64,
}

#[cfg(test)]
#[tokio::test]
async fn monster_disguise_blocks_proximity_aggro_but_keeps_damage_retaliation() {
    let game = reko_common::game_dir();
    if !game.join("Data/NPC_Looks.tbl").exists() {
        return;
    }
    let db = Connection::open_in_memory().unwrap();
    crate::population::tests::seed(&db);
    db.execute_batch(include_str!("npc_ai.sql")).unwrap();
    db.execute(
        "INSERT INTO world_monster_ai VALUES(100,5,100,100,1500,3,4,5,5,35,100,0,0,0,0,1,1500)",
        [],
    )
    .unwrap();
    let world = super::World::load(&game).unwrap();
    world.load_population(&db, &game).unwrap();
    let mut brain = Brain::default();
    brain.load(&db, None).unwrap();
    let mut inner = world.inner.lock().unwrap();
    let home = inner.mobs[&12000].position;
    let player = Player {
        nation: 1,
        name: "Disguised".into(),
        position: home,
        hp: 100,
        invisible: false,
        disguised: true,
        is_admin: false,
        stats: combat::Stats::default(),
        at: 1000,
    };
    brain.players.insert(1, player);
    brain.tick(&mut inner.mobs, 1000);
    assert_eq!(brain.agents[&12000].target, None);
    inner
        .mobs
        .get_mut(&12000)
        .unwrap()
        .damage_by
        .insert("Disguised".into(), 1);
    brain.tick(&mut inner.mobs, 1250);
    assert_eq!(brain.agents[&12000].target, Some(1));
}

#[cfg(test)]
#[tokio::test]
async fn monster_switches_aggro_when_another_player_deals_higher_damage() {
    let game = reko_common::game_dir();
    if !game.join("Data/NPC_Looks.tbl").exists() {
        return;
    }
    let db = Connection::open_in_memory().unwrap();
    crate::population::tests::seed(&db);
    db.execute_batch(include_str!("npc_ai.sql")).unwrap();
    db.execute(
        "INSERT INTO world_monster_ai VALUES(100,5,100,100,1500,3,4,5,5,35,100,0,0,0,0,1,1500)",
        [],
    )
    .unwrap();
    let world = super::World::load(&game).unwrap();
    world.load_population(&db, &game).unwrap();
    let mut brain = Brain::default();
    brain.load(&db, None).unwrap();
    let mut inner = world.inner.lock().unwrap();
    let home = inner.mobs[&12000].position;
    brain.players.insert(
        1,
        Player {
            nation: 1,
            name: "KarusTank".into(),
            position: home,
            hp: 500,
            invisible: false,
            disguised: false,
            is_admin: false,
            stats: combat::Stats::default(),
            at: 1000,
        },
    );
    brain.players.insert(
        2,
        Player {
            nation: 1,
            name: "ElmoDps".into(),
            position: home,
            hp: 500,
            invisible: false,
            disguised: false,
            is_admin: false,
            stats: combat::Stats {
                attack: 500,
                ..combat::Stats::default()
            },
            at: 1000,
        },
    );
    // Proximity aggro locks onto KarusTank first (0 damage dealt by KarusTank).
    brain.tick(&mut inner.mobs, 1000);
    assert_eq!(brain.agents[&12000].target, Some(1));
    // ElmoDps hits the mob -> switches immediately from 0-damage proximity target.
    inner
        .mobs
        .get_mut(&12000)
        .unwrap()
        .damage_by
        .insert("ElmoDps".into(), 200);
    brain.tick(&mut inner.mobs, 1250);
    assert_eq!(brain.agents[&12000].target, Some(2));
    // Now KarusTank deals higher damage across several ticks -> switches back to KarusTank.
    let mut switched = false;
    for step in 1..=20u64 {
        let now = 1250 + step * 250;
        brain.players.get_mut(&1).unwrap().at = now;
        brain.players.get_mut(&2).unwrap().at = now;
        *inner
            .mobs
            .get_mut(&12000)
            .unwrap()
            .damage_by
            .entry("KarusTank".into())
            .or_insert(0) += 400;
        brain.tick(&mut inner.mobs, now);
        if brain.agents[&12000].target == Some(1) {
            switched = true;
            break;
        }
    }
    assert!(
        switched,
        "mob should switch aggro to higher-damage attacker"
    );
}
struct Agent {
    target: Option<u16>,
    home: Position,
    chase_start: Position,
    next_attack: u64,
    last: u64,
    returning: bool,
    alive: bool,
    damage_seen: u64,
    damage_floor: HashMap<String, u32>,
    last_damage_by: HashMap<String, u32>,
    roam: Option<Position>,
    next_roam: u64,
    pattern: i8,
    moving: bool,
    cast: Option<Cast>,
}
#[derive(Clone, Copy)]
struct Cast {
    target: u16,
    at: u64,
}
#[derive(Clone)]
pub(crate) struct Strike {
    pub npc: u16,
    pub zone: u8,
    pub amount: u16,
    pub spell: Option<npc_magic::Hit>,
}
/// A guard swing at a monster, applied and re-validated by the world.
#[derive(Clone, Debug)]
pub(crate) struct GuardStrike {
    pub guard: u16,
    pub target: u16,
    pub amount: u32,
    /// Melee reach in metres used for re-validation.
    pub reach: f32,
}
struct GuardAgent {
    post: Position,
    target: Option<u16>,
    next_attack: u64,
    last: u64,
    returning: bool,
    roam: Option<Position>,
    next_roam: u64,
    pattern: i8,
    moving: bool,
}
/// A living monster guards may engage, captured at the start of a tick.
struct Prey {
    id: u16,
    position: Position,
    defence: u32,
    evade: Option<f32>,
    nation: u8,
}
#[derive(Default)]
pub(super) struct Brain {
    rules: HashMap<u16, Rule>,
    spells: HashMap<u32, npc_magic::Spell>,
    players: HashMap<u16, Player>,
    agents: HashMap<u16, Agent>,
    strikes: HashMap<u16, VecDeque<Strike>>,
    rng: u64,
    guard_rules: HashMap<u16, Rule>,
    guards: HashMap<u16, GuardAgent>,
    guard_strikes: Vec<GuardStrike>,
    /// Monsters guards ignore (rare-boss pool candidates).
    pub exempt: HashSet<u16>,
    /// Zones made hostile by events (homeland invasions): guards and towers
    /// there attack enemy-nation players, as in the PvP zones.
    invaded: HashSet<u8>,
}

/// Zones where guards and towers always attack enemy-nation players.
const GUARD_PVP_ZONES: [u8; 2] = [71, 72];
fn distance(a: Position, b: Position) -> f32 {
    f32::from(a.x.abs_diff(b.x)).hypot(f32::from(a.z.abs_diff(b.z))) / 10.0
}
fn random(seed: &mut u64, max: u32) -> u32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 16) as u32 % (max + 1)
}
/// Critical and success thresholds out of 10000 for a hit-rate/evasion ratio.
fn hit_chances(ratio: f32) -> (u32, u32) {
    if ratio >= 5.0 {
        (3500, 9800)
    } else if ratio >= 3.0 {
        (2500, 9600)
    } else if ratio >= 2.0 {
        (2000, 9400)
    } else if ratio >= 1.25 {
        (1500, 9200)
    } else if ratio >= 0.8 {
        (1000, 9000)
    } else if ratio >= 0.5 {
        (800, 8000)
    } else if ratio >= 0.33 {
        (600, 7000)
    } else if ratio >= 0.2 {
        (400, 6000)
    } else {
        (200, 5000)
    }
}
fn physical(rule: &Rule, target: &Player, attack_percent: u16, seed: &mut u64) -> u16 {
    let ac = u32::from(target.stats.defence);
    let hit = u32::from(rule.damage) * u32::from(attack_percent) * 2 / (ac + 240);
    if hit == 0 {
        return 0;
    }
    let (critical, success) = hit_chances(rule.hit / target.stats.evasion.max(0.001));
    let roll = random(seed, 9999) + 1;
    if roll > success {
        return 0;
    }
    let mut damage = (0.3 * random(seed, hit) as f32) as u32 + (0.85 * hit as f32) as u32;
    if roll <= critical {
        damage = damage * 3 / 2;
    }
    damage.min((2.6 * f32::from(rule.damage)) as u32).min(32767) as u16
}
/// Known 1453 NPC-versus-NPC melee: a great success deals 70-130% of the
/// attacker's damage ignoring armor, a normal success 70-130% of damage minus
/// the target's armor, and a miss deals nothing.
fn npc_physical(rule: &Rule, defence: u32, evade: f32, seed: &mut u64) -> u32 {
    let (great, success) = hit_chances(rule.hit / evade.max(0.001));
    let roll = random(seed, 9999) + 1;
    if roll > success {
        return 0;
    }
    let hit = u32::from(rule.damage);
    let base = if roll <= great {
        hit
    } else {
        hit.saturating_sub(defence)
    };
    if base == 0 {
        return 0;
    }
    let spread = random(seed, (0.6 * base as f32) as u32);
    ((0.7 * base as f32) as u32 + spread).min(32000)
}
/// Resolves a landed monster spell: per-target effect packets, an area marker
/// and queued per-victim hits that each victim's session applies atomically.
fn cast_effect(
    players: &HashMap<u16, Player>,
    strikes: &mut HashMap<u16, VecDeque<Strike>>,
    rng: &mut u64,
    mob: &Mob,
    spell: &npc_magic::Spell,
    target: u16,
    area: bool,
) -> Vec<Event> {
    let Some(victim) = players.get(&target) else {
        return Vec::new();
    };
    let area = area && spell.area();
    let center = victim.position;
    let valid = |p: &Player| {
        p.hp > 0 && !p.invisible && !p.is_admin && p.position.zone == mob.position.zone
    };
    let mut targets: Vec<u16> = if area {
        let mut ids: Vec<u16> = players
            .iter()
            .filter(|(_, p)| {
                valid(p)
                    && distance(center, p.position) <= spell.radius
                    && (spell.range <= 0.0 || distance(mob.position, p.position) <= spell.range)
            })
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    } else if valid(victim)
        && (spell.range <= 0.0
            || distance(mob.position, victim.position)
                <= spell.range + std::f32::consts::FRAC_1_SQRT_2 / 10.0)
    {
        vec![target]
    } else {
        Vec::new()
    };
    targets.truncate(32);
    let mut events = Vec::new();
    for id in targets {
        let player = &players[&id];
        let resist = npc_magic::resistance(&player.stats, spell.element);
        let mut hit = npc_magic::Hit {
            skill: spell.id,
            ..Default::default()
        };
        let mut amount = 0;
        match spell.direct {
            // A durational DirectType 2 spell takes its first amount as raw HP.
            2 if spell.duration_s > 0 && spell.first < 0 => {
                amount = spell.first.unsigned_abs().min(32000);
            }
            1 | 8 if spell.first < 0 => {
                amount = npc_magic::damage(spell.first, resist, |max| random(rng, max));
                hit.absorb = spell.direct == 8;
            }
            2 | 3 => hit.mana = spell.first,
            9 if spell.first != 0 => hit.drain_percent = spell.first.unsigned_abs().min(100) as u8,
            _ => {}
        }
        if spell.duration_s > 0 && spell.over_time < 0 {
            let total = if spell.element == 4 {
                spell.over_time.unsigned_abs()
            } else {
                npc_magic::damage(spell.over_time, resist, |max| random(rng, max))
            };
            let ticks = (spell.duration_s / 2).max(1);
            let per_tick = (total / ticks).clamp(1, 32767) as i16;
            hit.dot = Some((-per_tick, u64::from(spell.duration_s) * 1000, spell.element));
        }
        let mut result = 1i16;
        if spell.element == 3 && spell.direct == 1 && amount > 0 {
            // Lightning stuns unless the victim's lightning resistance wins the roll.
            if npc_magic::stuns(player.stats.resistance[2], random(rng, 99)) {
                hit.stun_ms = 2000;
            } else {
                result = 0;
            }
        }
        if let Some(debuff) = spell.debuff {
            let resisted = debuff.category == 6
                && spell.element == 2
                && random(rng, 99)
                    >= 100u32
                        .saturating_sub(u32::from(player.stats.resistance[1]) * 100 / 250)
                        .clamp(15, 75);
            if !resisted {
                hit.debuff = Some(debuff);
            }
        }
        let mut effect = skills::effect(spell.id, mob.id);
        effect[8..10].copy_from_slice(&id.to_le_bytes());
        effect[12..14].copy_from_slice(&result.to_le_bytes());
        if let Some(debuff) = hit.debuff {
            let seconds = (debuff.duration_ms / 1000).min(32767) as u16;
            effect[16..18].copy_from_slice(&seconds.to_le_bytes());
            effect[20..22].copy_from_slice(&debuff.speed.to_le_bytes());
        }
        events.push(Event {
            personal_hp: Vec::new(),
            zone: mob.position.zone,
            packet: effect,
        });
        let queue = strikes.entry(id).or_default();
        if queue.len() < 32 {
            queue.push_back(Strike {
                npc: mob.id,
                zone: mob.position.zone,
                amount,
                spell: Some(hit),
            });
        }
    }
    if area {
        let mut marker = skills::effect(spell.id, mob.id);
        marker[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
        marker[10..12].copy_from_slice(&(center.x / 10).to_le_bytes());
        marker[12..14].copy_from_slice(&0u16.to_le_bytes());
        marker[14..16].copy_from_slice(&(center.z / 10).to_le_bytes());
        events.push(Event {
            personal_hp: Vec::new(),
            zone: mob.position.zone,
            packet: marker,
        });
    }
    events
}
/// Whether the last movement packet among new events keeps the NPC moving;
/// `None` when the NPC sent no movement (spell or attack packets only).
fn last_movement(events: &[Event]) -> Option<bool> {
    events
        .iter()
        .rev()
        .find(|e| e.packet.first() == Some(&0x0b) && e.packet.len() >= 11)
        .map(|e| e.packet[9..11] != [0, 0])
}
fn move_toward(
    mob: &mut Mob,
    target: Position,
    meters: f32,
    speed: f32,
    stop: f32,
) -> Option<Event> {
    let dx = f32::from(target.x) - f32::from(mob.position.x);
    let dz = f32::from(target.z) - f32::from(mob.position.z);
    let length = dx.hypot(dz);
    if length < 0.5 || meters <= 0.0 {
        return None;
    }
    let remaining = (length - stop * 10.0).max(0.0);
    let step = (meters * 10.0).min(remaining);
    let old = mob.position;
    mob.position.x = (f32::from(old.x) + dx / length * step)
        .round()
        .clamp(0.0, 65535.0) as u16;
    mob.position.z = (f32::from(old.z) + dz / length * step)
        .round()
        .clamp(0.0, 65535.0) as u16;
    if old == mob.position {
        return None;
    }
    // Native NPC MoveTo approaches destinations over 1.5 / 0.85 seconds.
    // Send a bounded destination ahead of the simulated position, not each tiny step.
    if remaining - step < 0.5 {
        return Some(movement_event(mob, 0.0, mob.position));
    }
    let lead = speed * 10.0 * 1.5 / 0.85;
    let destination = Position {
        x: (f32::from(mob.position.x) + dx / length * lead)
            .round()
            .clamp(0.0, 65535.0) as u16,
        z: (f32::from(mob.position.z) + dz / length * lead)
            .round()
            .clamp(0.0, 65535.0) as u16,
        ..mob.position
    };
    Some(movement_event(mob, speed, destination))
}
fn movement_event(mob: &Mob, speed: f32, destination: Position) -> Event {
    let mut p = vec![0x0b];
    p.extend(mob.id.to_le_bytes());
    p.extend(destination.x.to_le_bytes());
    p.extend(destination.z.to_le_bytes());
    p.extend(destination.y.to_le_bytes());
    p.extend(((speed * 10.0).round().clamp(0.0, 65535.0) as u16).to_le_bytes());
    Event {
        personal_hp: Vec::new(),
        zone: mob.position.zone,
        packet: p,
    }
}
fn roam_destination(
    pattern: &mut i8,
    home: Position,
    position: Position,
    rng: &mut u64,
) -> Position {
    if *pattern == 0 {
        *pattern = random(rng, 2) as i8 - 1;
        return home;
    }
    let offset = if matches!(*pattern, -2 | -1 | 1 | 2) {
        20
    } else {
        -20
    };
    *pattern = match *pattern {
        -4 | 4 => 0,
        -3..=-1 => *pattern - 1,
        _ => *pattern + 1,
    };
    Position {
        x: position.x.saturating_add_signed(offset),
        z: position.z.saturating_add_signed(offset),
        ..position
    }
}
impl Brain {
    pub fn load(
        &mut self,
        db: &Connection,
        catalog: Option<&skills::Catalog>,
    ) -> anyhow::Result<()> {
        let modes = npc_magic::modes(db)?;
        let mut q=db.prepare("SELECT prototype,act_type,damage,hit_rate,attack_delay,run_speed,attack_range,search_range,tracing_range,bulk,direct_attack,walk_speed,stand_ms,magic1 FROM world_monster_ai")?;
        for r in q.query_map([], |r| {
            Ok((
                r.get(0)?,
                Rule {
                    act: r.get(1)?,
                    damage: r.get(2)?,
                    hit: r.get::<_, u16>(3)? as f32,
                    delay: r.get(4)?,
                    run: r.get::<_, u16>(5)? as f32,
                    range: r.get::<_, u16>(6)? as f32,
                    search: r.get::<_, u16>(7)? as f32,
                    trace: r.get::<_, u16>(8)? as f32,
                    bulk: r.get::<_, u16>(9)? as f32 / 100.0,
                    direct: r.get(10)?,
                    walk: r.get::<_, u16>(11)? as f32,
                    stand: r.get(12)?,
                    magic: r.get(13)?,
                    mode: 0,
                },
            ))
        })? {
            let (id, mut rule) = r?;
            rule.mode = modes.get(&id).copied().unwrap_or(0);
            let casts = matches!(rule.mode, 2 | 4 | 5) || matches!(rule.direct, 1 | 2);
            if casts
                && rule.magic != 0
                && let Some(spell) = catalog.and_then(|c| npc_magic::Spell::resolve(c, rule.magic))
            {
                self.spells.insert(rule.magic, spell);
            } else {
                rule.magic = 0;
            }
            self.rules.insert(id, rule);
        }
        self.load_guards(db, catalog)?;
        self.rng = skills::now_ms();
        tracing::info!(
            prototypes = self.rules.len(),
            spells = self.spells.len(),
            guards = self.guard_rules.len(),
            "monster AI rules loaded"
        );
        Ok(())
    }
    /// Guard combat rules come from the imported 1453 NPC table (`byType`
    /// 11-13); guards have no `world_monster_ai` rows.
    fn load_guards(
        &mut self,
        db: &Connection,
        catalog: Option<&skills::Catalog>,
    ) -> anyhow::Result<()> {
        let present: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='world_reference_rules')",
            [],
            |r| r.get(0),
        )?;
        if !present {
            return Ok(());
        }
        let mut q = db.prepare(
            "SELECT rule_id,json_extract(data,'$.sDamage'),json_extract(data,'$.sHitRate'),json_extract(data,'$.sAttackDelay'),json_extract(data,'$.bySpeed1'),json_extract(data,'$.bySpeed2'),json_extract(data,'$.byAttackRange'),json_extract(data,'$.bySearchRange'),json_extract(data,'$.byTracingRange'),json_extract(data,'$.sBulk'),json_extract(data,'$.sStandtime'),json_extract(data,'$.iMagic1') FROM world_reference_rules WHERE table_name='OBJECT_NPC' AND CAST(json_extract(data,'$.byType') AS INTEGER) IN (11,12,13,62)",
        )?;
        let rows = q.query_map([], |r| {
            let n = |i: usize| -> rusqlite::Result<u64> {
                Ok(match r.get::<_, rusqlite::types::Value>(i)? {
                    rusqlite::types::Value::Integer(v) => v.max(0) as u64,
                    rusqlite::types::Value::Real(v) => v.max(0.0) as u64,
                    rusqlite::types::Value::Text(v) => v.trim().parse().unwrap_or(0),
                    _ => 0,
                })
            };
            let small = |i: usize| -> rusqlite::Result<f32> { Ok(n(i)?.min(65535) as f32) };
            Ok((
                r.get::<_, u16>(0)?,
                Rule {
                    act: 7,
                    damage: n(1)?.min(u64::from(u16::MAX)) as u16,
                    hit: small(2)?,
                    delay: n(3)?,
                    walk: small(4)?,
                    run: small(5)?,
                    range: small(6)?,
                    search: small(7)?.min(GUARD_AGGRO_M),
                    trace: small(8)?,
                    bulk: small(9)? / 100.0,
                    stand: n(10)?,
                    direct: 0,
                    magic: n(11)?.min(u64::from(u32::MAX)) as u32,
                    mode: 0,
                },
            ))
        })?;
        for row in rows {
            let (id, mut rule) = row?;
            // Towers cast their first magic slot; guards fight in melee.
            if rule.magic != 0
                && let Some(spell) = catalog.and_then(|c| npc_magic::Spell::resolve(c, rule.magic))
            {
                self.spells.insert(rule.magic, spell);
            } else {
                if rule.magic != 0 {
                    tracing::warn!(
                        npc = id,
                        skill = rule.magic,
                        "guard spell unavailable; tower stays idle"
                    );
                }
                rule.magic = 0;
            }
            self.guard_rules.insert(id, rule);
        }
        Ok(())
    }
    /// Marks a zone as invaded (or clears it) so its guards and towers attack
    /// enemy-nation players.
    pub fn set_invaded(&mut self, zone: u8, invaded: bool) {
        if invaded {
            self.invaded.insert(zone);
        } else {
            self.invaded.remove(&zone);
        }
    }
    fn guards_hostile_to_players(&self, zone: u8) -> bool {
        GUARD_PVP_ZONES.contains(&zone) || self.invaded.contains(&zone)
    }
    pub fn take_guard_strikes(&mut self) -> Vec<GuardStrike> {
        std::mem::take(&mut self.guard_strikes)
    }
    /// Snapshot of monsters guards may engage, by zone, sorted by id.
    fn prey(&self, mobs: &HashMap<u16, Mob>) -> HashMap<u8, Vec<Prey>> {
        let mut prey: HashMap<u8, Vec<Prey>> = HashMap::new();
        if self.guard_rules.is_empty() {
            return prey;
        }
        for mob in mobs.values() {
            let Some(s) = mob.definition.as_ref() else {
                continue;
            };
            // One-life event spawns and GM summons (regen type 1) and rare
            // bosses are never engaged, nor objects the monster AI skips;
            // suppression rooms lie outside guard zones.
            if mob.role != Role::Monster
                || mob.hp == 0
                || s.regen_type == 1
                || matches!(s.kind, 50..=55 | 60 | 61 | 65..=68 | 171)
                || self.exempt.contains(&mob.id)
                || !crate::population::GUARD_ZONES.contains(&mob.position.zone)
            {
                continue;
            }
            prey.entry(mob.position.zone).or_default().push(Prey {
                id: mob.id,
                position: mob.position,
                defence: mob.defence(),
                evade: self.rules.get(&s.prototype).map(|r| r.hit),
                nation: s.nation,
            });
        }
        for list in prey.values_mut() {
            list.sort_unstable_by_key(|p| p.id);
        }
        prey
    }
    /// Guards pick the nearest hostile monster within their search range,
    /// chase it no farther than the leash from their post, then walk back.
    fn guard_tick(
        &mut self,
        mob: &mut Mob,
        prey: &HashMap<u8, Vec<Prey>>,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        let Some(s) = mob.definition.as_ref() else {
            return;
        };
        let (nation, move_type, size) = (s.nation, s.move_type, s.size);
        let tower = s.kind == crate::population::TOWER_KIND;
        let pvp = nation != 0 && self.guards_hostile_to_players(mob.position.zone);
        let Some(rule) = self.guard_rules.get(&s.prototype) else {
            return;
        };
        if mob.hp == 0 || (rule.damage == 0 && rule.magic == 0) {
            return;
        }
        let agent = self.guards.entry(mob.id).or_insert(GuardAgent {
            post: mob.position,
            target: None,
            next_attack: now,
            last: now,
            returning: false,
            roam: None,
            next_roam: now.saturating_add(rule.stand),
            pattern: 0,
            moving: false,
        });
        let elapsed = now.saturating_sub(agent.last).min(500) as f32 / 1000.0;
        agent.last = now;
        let candidates = prey.get(&mob.position.zone).map_or(&[][..], Vec::as_slice);
        let find = |id: u16| {
            candidates
                .binary_search_by_key(&id, |p| p.id)
                .ok()
                .map(|i| &candidates[i])
        };
        // 1453 hostility: a nation-0 guard is friendly to all, otherwise units
        // of another nation (or nation 3) are hostile.
        let hostile = |p: &Prey| !tower && nation != 0 && (p.nation != nation || p.nation == 3);
        // Enemy-nation players in PvP or invaded zones; towers use their attack range.
        let players = &self.players;
        let tower_spell = self.spells.get(&rule.magic).filter(|_| rule.magic != 0);
        let reach_search = if tower {
            let range = rule.range.min(30.0);
            tower_spell
                .filter(|s| s.range > 0.0)
                .map_or(range, |s| range.min(s.range))
        } else {
            rule.search
        };
        let enemy = |p: &Player| {
            pvp && p.hp > 0
                && !p.invisible
                && !p.is_admin
                && p.nation != nation
                && p.position.zone == mob.position.zone
        };
        if let Some(id) = agent.target.filter(|id| crate::pvp::is_player_id(*id)) {
            let keep = players.get(&id).is_some_and(|p| {
                enemy(p)
                    && distance(agent.post, p.position) <= GUARD_LEASH_M.max(reach_search)
                    && (move_type != 4 || distance(mob.position, p.position) <= reach_search)
            });
            if !keep {
                agent.target = None;
                agent.returning = distance(mob.position, agent.post) >= 0.2;
                agent.roam = None;
            }
        }
        let chasing_monster = agent.target.is_some_and(|id| !crate::pvp::is_player_id(id));
        if (agent.target.is_none() || chasing_monster) && !agent.returning {
            let enemy_player = players
                .iter()
                .filter(|(_, p)| {
                    enemy(p)
                        && distance(mob.position, p.position) <= reach_search
                        && distance(agent.post, p.position) <= GUARD_LEASH_M.max(reach_search)
                })
                .min_by(|(ia, a), (ib, b)| {
                    distance(mob.position, a.position)
                        .total_cmp(&distance(mob.position, b.position))
                        .then(ia.cmp(ib))
                })
                .map(|(id, _)| *id);
            // Known 1453 guards check players before monsters.
            if enemy_player.is_some() {
                agent.target = enemy_player;
                agent.roam = None;
            }
        }
        if let Some(id) = agent.target.filter(|id| crate::pvp::is_player_id(*id)) {
            let target = &players[&id];
            let reach = if tower {
                reach_search
            } else {
                1.0f32.min(rule.range) + rule.bulk * f32::from(size) / 100.0
            };
            let dist = distance(mob.position, target.position);
            let speed = rule.run * f32::from(mob.speed_percent) / 100.0;
            let event_start = events.len();
            if dist > reach + std::f32::consts::FRAC_1_SQRT_2 / 10.0 {
                if move_type != 4
                    && let Some(e) = move_toward(
                        mob,
                        target.position,
                        (speed * elapsed).min((dist - reach).max(0.0)),
                        speed,
                        reach,
                    )
                {
                    events.push(e);
                }
            } else if now >= agent.next_attack {
                agent.next_attack = now.saturating_add(rule.delay.max(1));
                if let Some(spell) = tower_spell {
                    events.extend(cast_effect(
                        players,
                        &mut self.strikes,
                        &mut self.rng,
                        mob,
                        spell,
                        id,
                        false,
                    ));
                } else if rule.damage > 0 {
                    let amount = physical(rule, target, 100, &mut self.rng);
                    let queue = self.strikes.entry(id).or_default();
                    if queue.len() < 32 {
                        queue.push_back(Strike {
                            npc: mob.id,
                            zone: mob.position.zone,
                            amount,
                            spell: None,
                        });
                    }
                }
            }
            if let Some(moving) = last_movement(&events[event_start..]) {
                agent.moving = moving;
            } else if agent.moving {
                events.push(movement_event(mob, 0.0, mob.position));
                agent.moving = false;
            }
            return;
        }
        if tower {
            return;
        }
        if let Some(id) = agent.target {
            let keep = find(id).is_some_and(|p| {
                distance(agent.post, p.position) <= GUARD_LEASH_M
                    && (move_type != 4 || distance(mob.position, p.position) <= rule.search)
            });
            if !keep {
                agent.target = None;
                agent.returning = distance(mob.position, agent.post) >= 0.2;
                agent.roam = None;
            }
        }
        if agent.target.is_none() && !agent.returning {
            agent.target = candidates
                .iter()
                .filter(|p| {
                    hostile(p)
                        && distance(mob.position, p.position) <= rule.search
                        && distance(agent.post, p.position) <= GUARD_LEASH_M
                })
                .min_by(|a, b| {
                    distance(mob.position, a.position)
                        .total_cmp(&distance(mob.position, b.position))
                        .then(a.id.cmp(&b.id))
                })
                .map(|p| p.id);
            if agent.target.is_some() {
                agent.roam = None;
            }
        }
        let speed = rule.run * f32::from(mob.speed_percent) / 100.0;
        let event_start = events.len();
        if let Some(target) = agent.target.and_then(find) {
            let reach = 1.0f32.min(rule.range) + rule.bulk * f32::from(size) / 100.0;
            let dist = distance(mob.position, target.position);
            if dist > reach + std::f32::consts::FRAC_1_SQRT_2 / 10.0 {
                if move_type != 4
                    && let Some(e) = move_toward(
                        mob,
                        target.position,
                        (speed * elapsed).min((dist - reach).max(0.0)),
                        speed,
                        reach,
                    )
                {
                    events.push(e);
                }
            } else if now >= agent.next_attack {
                agent.next_attack = now.saturating_add(rule.delay.max(1));
                let amount = npc_physical(
                    rule,
                    target.defence,
                    target.evade.unwrap_or(rule.hit),
                    &mut self.rng,
                );
                self.guard_strikes.push(GuardStrike {
                    guard: mob.id,
                    target: target.id,
                    amount,
                    reach,
                });
            }
        } else if agent.returning {
            if distance(mob.position, agent.post) < 0.2 {
                agent.returning = false;
                agent.pattern = 0;
                agent.roam = None;
                agent.next_roam = now.saturating_add(rule.stand);
            } else if let Some(e) = move_toward(mob, agent.post, speed * elapsed, speed, 0.0) {
                events.push(e);
            }
        } else if matches!(move_type, 1 | 2)
            && self.players.values().any(|p| {
                p.position.zone == mob.position.zone && distance(p.position, mob.position) < 100.0
            })
        {
            if agent.roam.is_none() && now >= agent.next_roam {
                let destination =
                    roam_destination(&mut agent.pattern, agent.post, mob.position, &mut self.rng);
                if distance(mob.position, destination) >= 0.2 {
                    agent.roam = Some(destination);
                } else {
                    agent.next_roam = now.saturating_add(rule.stand);
                }
            }
            if let Some(destination) = agent.roam {
                let walk = rule.walk * f32::from(mob.speed_percent) / 100.0;
                if let Some(e) = move_toward(mob, destination, walk * elapsed, walk, 0.0) {
                    events.push(e);
                }
                if distance(mob.position, destination) < 0.2 {
                    agent.roam = None;
                    agent.next_roam = now.saturating_add(rule.stand);
                    events.push(movement_event(mob, 0.0, mob.position));
                }
            }
        }
        if let Some(moving) = last_movement(&events[event_start..]) {
            agent.moving = moving;
        } else if agent.moving {
            events.push(movement_event(mob, 0.0, mob.position));
            agent.moving = false;
        }
    }
    pub fn observe(&mut self, s: &Session, now: u64) {
        self.players.insert(
            s.player_id,
            Player {
                is_admin: s.is_admin,
                name: s.character.name.clone(),
                nation: s.character.nation(),
                position: s
                    .movement
                    .current_position(s.position, std::time::Instant::now()),
                hp: s.skills.hp,
                invisible: s.skills.buffs.get(&91).is_some_and(|b| now < b.until),
                disguised: crate::transformation::appearance(s) != 1,
                stats: s.combat,
                at: now,
            },
        );
    }
    pub fn provoke(&mut self, mob_id: u16, player_id: u16) {
        if self.players.contains_key(&player_id)
            && let Some(agent) = self.agents.get_mut(&mob_id)
        {
            agent.target = Some(player_id);
            agent.returning = false;
        }
    }
    pub fn remove(&mut self, id: u16) {
        self.players.remove(&id);
        self.strikes.remove(&id);
    }
    /// Drops all state of a removed entity so its ID can be reused.
    pub fn forget_mob(&mut self, id: u16) {
        self.agents.remove(&id);
        self.guards.remove(&id);
    }
    pub fn take(&mut self, id: u16) -> Vec<Strike> {
        self.strikes
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .collect()
    }
    pub fn tick(&mut self, mobs: &mut HashMap<u16, Mob>, now: u64) -> Vec<Event> {
        self.players.retain(|_, p| now.saturating_sub(p.at) < 2000);
        let mut events = Vec::new();
        let prey = self.prey(mobs);
        for mob in mobs.values_mut() {
            if mob.role == Role::Guard {
                self.guard_tick(mob, &prey, now, &mut events);
                continue;
            }
            let Some(s) = mob.definition.as_ref() else {
                continue;
            };
            let Some(rule) = self.rules.get(&s.prototype) else {
                continue;
            };
            if matches!(s.kind,50..=55|60|61|65..=68|171) || rule.damage == 0 {
                continue;
            }
            let agent = self.agents.entry(mob.id).or_insert(Agent {
                target: None,
                home: mob.position,
                chase_start: mob.position,
                next_attack: 0,
                last: now,
                returning: false,
                alive: false,
                damage_seen: 0,
                damage_floor: HashMap::new(),
                last_damage_by: HashMap::new(),
                roam: None,
                next_roam: now.saturating_add(rule.stand),
                pattern: 0,
                moving: false,
                cast: None,
            });
            let elapsed = now.saturating_sub(agent.last).min(500) as f32 / 1000.0;
            agent.last = now;
            if mob.hp == 0 {
                agent.target = None;
                agent.returning = false;
                agent.alive = false;
                agent.roam = None;
                agent.damage_seen = 0;
                agent.damage_floor.clear();
                agent.last_damage_by.clear();
                agent.moving = false;
                agent.cast = None;
                continue;
            }
            if !agent.alive {
                agent.home = mob.position;
                agent.chase_start = mob.position;
                agent.next_attack = now;
                agent.alive = true;
                agent.pattern = 0;
                agent.roam = None;
                agent.next_roam = now.saturating_add(rule.stand);
                agent.moving = false;
            }
            if now < mob.stunned_until {
                // A stun interrupts a cast in progress.
                agent.cast = None;
                if agent.moving {
                    events.push(movement_event(mob, 0.0, mob.position));
                    agent.moving = false;
                }
                continue;
            }
            let damage = mob.damage_by.values().map(|v| u64::from(*v)).sum();
            let new_damage_detected = damage > agent.damage_seen;
            if new_damage_detected {
                agent.returning = false;
                agent.chase_start = mob.position;
            }
            agent.damage_seen = damage;
            let valid = |p: &Player| {
                p.hp > 0 && !p.invisible && !p.is_admin && p.position.zone == mob.position.zone
            };
            let threat = |p: &Player| {
                mob.damage_by
                    .get(&p.name)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(agent.damage_floor.get(&p.name).copied().unwrap_or(0))
            };
            let attacked = |p: &Player| threat(p) > 0;
            if let Some(id) = agent.target {
                let keep = self.players.get(&id).is_some_and(|p| {
                    let range = if attacked(p) {
                        rule.trace
                    } else {
                        rule.search + 2.0
                    };
                    valid(p)
                        && (!p.disguised || attacked(p))
                        && distance(agent.chase_start, mob.position) <= range
                        && distance(mob.position, p.position)
                            < if attacked(p) {
                                100.0
                            } else {
                                range.max(rule.range)
                            }
                });
                if !keep {
                    agent.target = None;
                    agent.returning = true;
                    agent.roam = None;
                    agent.damage_floor = mob.damage_by.clone();
                } else if new_damage_detected && let Some(current) = self.players.get(&id) {
                    // 1453/Open-KO CNpc::ChangeTarget aggro switch when another player deals damage.
                    let challenger = self
                        .players
                        .iter()
                        .filter_map(|(&other_id, other)| {
                            if other_id == id || !valid(other) {
                                return None;
                            }
                            let dealt = mob
                                .damage_by
                                .get(&other.name)
                                .copied()
                                .unwrap_or(0)
                                .saturating_sub(
                                    agent.last_damage_by.get(&other.name).copied().unwrap_or(0),
                                );
                            (dealt > 0 && distance(mob.position, other.position) < 100.0)
                                .then_some((other_id, other, dealt))
                        })
                        .max_by_key(|&(_, _, dealt)| dealt);
                    if let Some((other_id, other, _)) = challenger {
                        let current_threat = threat(current);
                        let other_threat = threat(other);
                        let roll = random(&mut self.rng, 99);
                        let switch = current_threat == 0
                            || match roll {
                                0..50 => {
                                    other_threat > current_threat
                                        || (other_threat == current_threat
                                            && other.stats.attack >= current.stats.attack)
                                }
                                50..80 => {
                                    distance(mob.position, other.position)
                                        <= distance(mob.position, current.position)
                                }
                                80..95 => other.stats.defence <= current.stats.defence,
                                _ => true,
                            };
                        if switch {
                            agent.target = Some(other_id);
                            agent.chase_start = mob.position;
                            agent.returning = false;
                            agent.roam = None;
                        }
                    }
                }
            }
            if new_damage_detected {
                agent.last_damage_by = mob.damage_by.clone();
            }
            if agent.target.is_none() && !agent.returning {
                let attacked = |p: &Player| {
                    mob.damage_by.get(&p.name).is_some_and(|damage| {
                        *damage > agent.damage_floor.get(&p.name).copied().unwrap_or(0)
                    })
                };
                let aggressive = !matches!(rule.act, 1 | 2);
                agent.target = self
                    .players
                    .iter()
                    .filter(|(_, p)| {
                        valid(p)
                            && distance(mob.position, p.position)
                                < if attacked(p) { 100.0 } else { rule.search }
                            && ((aggressive && !p.disguised) || attacked(p))
                    })
                    .min_by(|(ia, a), (ib, b)| {
                        attacked(b).cmp(&attacked(a)).then_with(|| {
                            distance(mob.position, a.position)
                                .total_cmp(&distance(mob.position, b.position))
                                .then(ia.cmp(ib))
                        })
                    })
                    .map(|(id, _)| *id);
                if agent.target.is_some() {
                    agent.chase_start = mob.position;
                    agent.roam = None;
                }
            }
            let speed = rule.run * f32::from(mob.speed_percent) / 100.0;
            let event_start = events.len();
            if let Some(cast) = agent.cast {
                // Casting roots the monster until the spell lands or fails.
                if now >= cast.at {
                    agent.cast = None;
                    let bulk = rule.bulk * f32::from(s.size) / 100.0;
                    if let Some(spell) = self.spells.get(&rule.magic)
                        && self.players.get(&cast.target).is_some_and(|p| {
                            p.hp > 0
                                && !p.invisible
                                && !p.is_admin
                                && p.position.zone == mob.position.zone
                                && distance(mob.position, p.position) <= rule.range + bulk + 2.0
                        })
                    {
                        events.extend(cast_effect(
                            &self.players,
                            &mut self.strikes,
                            &mut self.rng,
                            mob,
                            spell,
                            cast.target,
                            false,
                        ));
                    }
                }
            } else if let Some(id) = agent.target {
                let target = &self.players[&id];
                let bulk = rule.bulk * f32::from(s.size) / 100.0;
                let spell = self.spells.get(&rule.magic);
                let melee = if rule.direct == 1 && spell.is_none() {
                    5.0f32.min(rule.range)
                } else {
                    1.0f32.min(rule.range)
                } + bulk;
                // byDirectAttack 1 always casts at range; 2 casts when outside melee.
                let long_cast = spell.is_some() && matches!(rule.direct, 1 | 2);
                // byDirectAttack 1 approaches to 5 m before casting; 2 casts from its
                // attack range. Never start a cast beyond the spell's own range.
                let mut cast_reach = if rule.direct == 1 {
                    5.0f32.min(rule.range.max(1.0))
                } else {
                    rule.range.max(1.0)
                } + bulk;
                if let Some(spell) = spell.filter(|s| s.range > 0.0) {
                    cast_reach = cast_reach.min(spell.range);
                }
                let reach = if long_cast { cast_reach } else { melee };
                let dist = distance(mob.position, target.position);
                // Positions round to 0.1 m per axis. The final diagonal step
                // can land just outside reach with no further representable move.
                let in_reach = dist <= reach + std::f32::consts::FRAC_1_SQRT_2 / 10.0;
                if !in_reach && s.move_type != 4 {
                    if let Some(e) = move_toward(
                        mob,
                        target.position,
                        (speed * elapsed).min((dist - reach).max(0.0)),
                        speed,
                        reach,
                    ) {
                        events.push(e);
                    }
                } else if in_reach
                    && now >= agent.next_attack
                    && let Some(spell) = spell.filter(|_| {
                        long_cast
                            && (rule.direct == 1
                                || dist > melee + std::f32::consts::FRAC_1_SQRT_2 / 10.0)
                    })
                {
                    agent.next_attack = now.saturating_add(rule.delay.max(1));
                    if spell.cast_ms > 0 {
                        let mut casting = skills::effect(spell.id, mob.id);
                        casting[1] = 1;
                        casting[8..10].copy_from_slice(&id.to_le_bytes());
                        events.push(Event {
                            personal_hp: Vec::new(),
                            zone: mob.position.zone,
                            packet: casting,
                        });
                        agent.cast = Some(Cast {
                            target: id,
                            at: now.saturating_add(spell.cast_ms),
                        });
                    } else {
                        events.extend(cast_effect(
                            &self.players,
                            &mut self.strikes,
                            &mut self.rng,
                            mob,
                            spell,
                            id,
                            false,
                        ));
                    }
                } else if in_reach && now >= agent.next_attack {
                    agent.next_attack = now.saturating_add(rule.delay.max(1));
                    // byMagicAttack 2/4/5: about 10% of melee swings become an instant spell.
                    let instant = spell.filter(|_| {
                        matches!(rule.mode, 2 | 4 | 5) && random(&mut self.rng, 9999) < 999
                    });
                    if let Some(spell) = instant {
                        if matches!(rule.mode, 4 | 5) {
                            agent.next_attack = agent.next_attack.saturating_add(1000);
                        }
                        events.extend(cast_effect(
                            &self.players,
                            &mut self.strikes,
                            &mut self.rng,
                            mob,
                            spell,
                            id,
                            matches!(rule.mode, 4 | 5),
                        ));
                    } else {
                        let percent = mob
                            .modifiers
                            .get(&9)
                            .filter(|m| now < m.until)
                            .map_or(100, |m| m.percent);
                        let amount = physical(rule, target, percent, &mut self.rng);
                        let queue = self.strikes.entry(id).or_default();
                        if queue.len() < 32 {
                            queue.push_back(Strike {
                                npc: mob.id,
                                zone: mob.position.zone,
                                amount,
                                spell: None,
                            });
                        }
                    }
                }
            } else if agent.returning {
                if distance(mob.position, agent.home) < 0.2 {
                    agent.returning = false;
                    agent.pattern = 0;
                    agent.roam = None;
                    agent.next_roam = now.saturating_add(rule.stand);
                } else if let Some(e) = move_toward(mob, agent.home, speed * elapsed, speed, 0.0) {
                    events.push(e);
                }
            } else if matches!(s.move_type, 1 | 2)
                && rule.search > 0.0
                && self.players.values().any(|p| {
                    p.position.zone == mob.position.zone
                        && distance(p.position, mob.position) < 100.0
                })
            {
                if agent.roam.is_none() && now >= agent.next_roam {
                    let destination = roam_destination(
                        &mut agent.pattern,
                        agent.home,
                        mob.position,
                        &mut self.rng,
                    );
                    let in_bounds = s.kind != 4
                        || (s.bounds[0].min(s.bounds[2])..=s.bounds[0].max(s.bounds[2]))
                            .contains(&destination.x)
                            && (s.bounds[1].min(s.bounds[3])..=s.bounds[1].max(s.bounds[3]))
                                .contains(&destination.z);
                    if in_bounds && distance(mob.position, destination) >= 0.2 {
                        agent.roam = Some(destination);
                    } else {
                        agent.next_roam = now.saturating_add(rule.stand);
                    }
                }
                if let Some(destination) = agent.roam {
                    let walk = rule.walk * f32::from(mob.speed_percent) / 100.0;
                    if let Some(e) = move_toward(mob, destination, walk * elapsed, walk, 0.0) {
                        events.push(e);
                    }
                    if distance(mob.position, destination) < 0.2 {
                        agent.roam = None;
                        agent.next_roam = now.saturating_add(rule.stand);
                        events.push(movement_event(mob, 0.0, mob.position));
                    }
                }
            }
            if let Some(moving) = last_movement(&events[event_start..]) {
                agent.moving = moving;
            } else if agent.moving {
                events.push(movement_event(mob, 0.0, mob.position));
                agent.moving = false;
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_npc_interpolation_tracks_walk_run_and_stop_without_crossing_the_goal() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let world = super::super::World::load(&game).unwrap();
        let original = world.inner.lock().unwrap().mobs[&10000].clone();
        for speed in [2.0, 4.0, 8.0] {
            let mut mob = original.clone();
            mob.position.x = 1000;
            let goal = Position {
                x: 2000,
                ..mob.position
            };
            let mut rendered = 100.0f32;
            let mut destination = rendered;
            let mut velocity = 0.0f32;
            for _ in 0..240 {
                rendered += (destination - rendered).signum()
                    * (velocity * 0.25).min((destination - rendered).abs());
                if let Some(e) = move_toward(&mut mob, goal, speed * 0.25, speed, 2.0) {
                    destination = f32::from(u16::from_le_bytes([e.packet[3], e.packet[4]])) / 10.0;
                    if e.packet[9..11] != [0, 0] {
                        velocity = (destination - rendered).abs() / 1.5 * 0.85;
                    }
                } else {
                    let stop = movement_event(&mob, 0.0, mob.position);
                    destination =
                        f32::from(u16::from_le_bytes([stop.packet[3], stop.packet[4]])) / 10.0;
                }
                assert!(
                    (f32::from(mob.position.x) / 10.0 - rendered).abs() < 3.0,
                    "native visual target must remain within basic melee reach at speed {speed}"
                );
                assert!(
                    rendered <= 198.05,
                    "arrival must cancel the extrapolated destination before overshoot: {rendered} at speed {speed}"
                );
            }
            assert_eq!(mob.position.x, 1980);
            assert!((rendered - 198.0).abs() < 0.01);
        }
    }
    #[test]
    fn admins_are_excluded_from_aggro_retaliation_and_existing_targets() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        db.execute_batch(include_str!("npc_ai.sql")).unwrap();
        db.execute("INSERT INTO world_monster_ai VALUES(100,3,100,100,1000,1,4,5,20,30,100,0,0,0,0,1,1000)",[]).unwrap();
        for act in [1, 3] {
            let world = super::super::World::load(&game).unwrap();
            world.load_population(&db, &game).unwrap();
            let mut inner = world.inner.lock().unwrap();
            let mobs = &mut inner.mobs;
            let home = mobs[&12000].position;
            let mut brain = Brain::default();
            brain.load(&db, None).unwrap();
            brain.rules.get_mut(&100).unwrap().act = act;
            let mut admin = Player {
                name: "Admin".into(),
                nation: 1,
                position: home,
                hp: 100,
                invisible: false,
                disguised: false,
                is_admin: true,
                stats: combat::Stats::default(),
                at: 1000,
            };
            brain.players.insert(1, admin.clone());
            brain.tick(mobs, 1000);
            assert!(brain.agents[&12000].target.is_none());
            mobs.get_mut(&12000)
                .unwrap()
                .damage_by
                .insert(admin.name.clone(), 1);
            brain.tick(mobs, 1100);
            assert!(brain.agents[&12000].target.is_none());
            let mut normal = admin.clone();
            normal.is_admin = false;
            normal.name = "Normal".into();
            mobs.get_mut(&12000)
                .unwrap()
                .damage_by
                .insert(normal.name.clone(), 1);
            brain.players.insert(2, normal.clone());
            brain.tick(mobs, 1200);
            assert_eq!(brain.agents[&12000].target, Some(2));
            normal.is_admin = true;
            normal.at = 1300;
            brain.players.insert(2, normal);
            brain.tick(mobs, 1300);
            assert!(brain.agents[&12000].target.is_none());
            admin.is_admin = false;
            admin.at = 1400;
            brain.players.insert(1, admin.clone());
            mobs.get_mut(&12000)
                .unwrap()
                .damage_by
                .insert(admin.name, 2);
            brain.tick(mobs, 1400);
            assert_eq!(brain.agents[&12000].target, Some(1));
        }
    }
    #[test]
    fn ranged_retaliation_refreshes_leash_and_forgets_threat_without_erasing_damage_credit() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        db.execute_batch(include_str!("npc_ai.sql")).unwrap();
        db.execute(
            "INSERT INTO world_monster_ai VALUES(100,1,100,100,1500,3,4,5,5,35,100,0,0,0,0,1,1500)",
            [],
        )
        .unwrap();
        let world = super::super::World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let mut brain = Brain::default();
        brain.load(&db, None).unwrap();
        let mut inner = world.inner.lock().unwrap();
        let mobs = &mut inner.mobs;
        let home = mobs[&12000].position;
        let mut p = Player {
            name: "Ranged attacker".into(),
            nation: 1,
            position: Position {
                x: home.x + 300,
                ..home
            },
            hp: 100,
            invisible: false,
            disguised: false,
            is_admin: false,
            stats: combat::Stats::default(),
            at: 1000,
        };
        brain.players.insert(1, p.clone());
        brain.tick(mobs, 1000);
        assert_eq!(
            brain.agents[&12000].target, None,
            "neutral proximity must not aggro"
        );
        mobs.get_mut(&12000)
            .unwrap()
            .damage_by
            .insert(p.name.clone(), 10);
        brain.tick(mobs, 1250);
        assert_eq!(brain.agents[&12000].target, Some(1));
        assert!(
            mobs[&12000].position.x > home.x,
            "ranged attacker must be pursued"
        );
        assert!(
            brain.take(1).is_empty(),
            "must approach before dealing melee damage"
        );
        mobs.get_mut(&12000).unwrap().position.x = home.x + 360;
        p.position.x = home.x + 650;
        p.at = 1500;
        brain.players.insert(1, p.clone());
        brain.tick(mobs, 1500);
        assert_eq!(
            brain.agents[&12000].target, None,
            "35m leash expires without fresh attacks"
        );
        assert!(brain.agents[&12000].returning);
        assert_eq!(
            mobs[&12000].damage_by[&p.name], 10,
            "XP/loot contribution must survive threat loss"
        );
        mobs.get_mut(&12000)
            .unwrap()
            .damage_by
            .insert(p.name.clone(), 11);
        brain.tick(mobs, 1750);
        assert_eq!(
            brain.agents[&12000].target,
            Some(1),
            "a fresh attack interrupts the return"
        );
        let next = Position {
            x: brain.agents[&12000].chase_start.x + 360,
            ..home
        };
        let mob = mobs.get_mut(&12000).unwrap();
        mob.position = next;
        mob.damage_by.insert(p.name.clone(), 12);
        p.position.x = next.x + 300;
        p.at = 2000;
        brain.players.insert(1, p.clone());
        brain.tick(mobs, 2000);
        assert_eq!(
            brain.agents[&12000].target,
            Some(1),
            "continued damage renews pursuit"
        );
        assert_eq!(brain.agents[&12000].chase_start, next);
        mobs.get_mut(&12000).unwrap().position.x = next.x + 360;
        p.position.x = next.x + 650;
        p.at = 2250;
        brain.players.insert(1, p.clone());
        brain.tick(mobs, 2250);
        assert!(brain.agents[&12000].returning);
        mobs.get_mut(&12000).unwrap().position = home;
        p.position.x = home.x + 300;
        p.at = 2500;
        brain.players.insert(1, p.clone());
        brain.tick(mobs, 2500);
        brain.tick(mobs, 2750);
        assert_eq!(
            brain.agents[&12000].target, None,
            "old damage credit must not restart retaliation"
        );
        mobs.get_mut(&12000)
            .unwrap()
            .damage_by
            .insert(p.name.clone(), 13);
        brain.tick(mobs, 3000);
        assert_eq!(brain.agents[&12000].target, Some(1));
        p.invisible = true;
        p.at = 3250;
        brain.players.insert(1, p.clone());
        brain.tick(mobs, 3250);
        assert_eq!(brain.agents[&12000].target, None);
    }

    #[test]
    fn roaming_uses_walk_speed_stand_time_source_pattern_and_stationary_flags() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        db.execute_batch(include_str!("npc_ai.sql")).unwrap();
        db.execute(
            "INSERT INTO world_monster_ai VALUES(100,1,100,100,1500,3,4,5,5,35,100,0,0,0,0,1,1500)",
            [],
        )
        .unwrap();
        let world = super::super::World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let mut brain = Brain::default();
        brain.load(&db, None).unwrap();
        let mut inner = world.inner.lock().unwrap();
        let mobs = &mut inner.mobs;
        let home = mobs[&12000].position;
        let mut p = Player {
            name: "Observer".into(),
            nation: 1,
            position: Position {
                x: home.x + 500,
                ..home
            },
            hp: 100,
            invisible: false,
            disguised: false,
            is_admin: false,
            stats: combat::Stats::default(),
            at: 1000,
        };
        brain.players.insert(1, p.clone());
        brain.tick(mobs, 1000);
        brain.agents.get_mut(&12000).unwrap().pattern = 1;
        assert!(brain.tick(mobs, 1250).is_empty());
        assert_eq!(mobs[&12000].position, home);
        p.at = 2500;
        brain.players.insert(1, p.clone());
        let events = brain.tick(mobs, 2500);
        assert_eq!(events[0].packet[0], 0x0b);
        assert_eq!(
            u16::from_le_bytes(events[0].packet[9..11].try_into().unwrap()),
            30
        );
        assert!(
            distance(home, mobs[&12000].position) > 0.0
                && distance(home, mobs[&12000].position) < 1.6
        );
        brain.tick(mobs, 2750);
        let events = brain.tick(mobs, 3000);
        assert_eq!(
            mobs[&12000].position,
            Position {
                x: home.x + 20,
                z: home.z + 20,
                ..home
            }
        );
        assert_eq!(&events.last().unwrap().packet[9..11], &[0, 0]);
        assert_eq!(brain.agents[&12000].next_roam, 4500);
        assert!(brain.tick(mobs, 3250).is_empty());
        let here = mobs[&12000].position;
        brain.remove(1);
        assert!(brain.tick(mobs, 5000).is_empty());
        p.at = 5250;
        brain.players.insert(1, p);
        for move_type in [0, 4] {
            mobs.get_mut(&12000)
                .unwrap()
                .definition
                .as_mut()
                .unwrap()
                .move_type = move_type;
            assert!(brain.tick(mobs, 5250).is_empty());
            assert_eq!(mobs[&12000].position, here);
        }
    }
    #[test]
    fn monster_spells_cast_land_interrupt_and_apply_area_dot_and_debuffs() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists()
            || !game.join("Data/Skill_Magic_Main_us.tbl").exists()
        {
            return;
        }
        let catalog = skills::Catalog::load(&game).unwrap();
        let db = Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        db.execute_batch(include_str!("npc_ai.sql")).unwrap();
        // byDirectAttack 1 with Deruvish lightning: always casts within attack range.
        db.execute(
            "INSERT INTO world_monster_ai VALUES(100,3,100,100,1500,3,4,20,35,35,100,1,300123,0,0,1,1500)",
            [],
        )
        .unwrap();
        let world = super::super::World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let mut brain = Brain::default();
        brain.load(&db, Some(&catalog)).unwrap();
        let lightning = brain.spells[&300123].clone();
        assert!(lightning.cast_ms > 0 && lightning.first < 0 && lightning.element == 3);
        let now = skills::now_ms();
        let home = world.inner.lock().unwrap().mobs[&12000].position;
        let player = |name: &str, dx: u16| Player {
            name: name.into(),
            nation: 1,
            position: Position {
                x: home.x + dx,
                ..home
            },
            hp: 100,
            invisible: false,
            disguised: false,
            is_admin: false,
            stats: combat::Stats::default(),
            at: now,
        };
        // byDirectAttack 1 casters chase until within 5 m, then cast.
        brain.players.insert(1, player("Caster", 40));
        let mut inner = world.inner.lock().unwrap();
        let events = {
            let at = now;
            for p in brain.players.values_mut() {
                p.at = at;
            }
            brain.tick(&mut inner.mobs, at)
        };
        let casting: Vec<_> = events
            .iter()
            .filter(|e| e.packet[..2] == [0x31, 1])
            .collect();
        assert_eq!(casting.len(), 1, "ranged caster within 5 m starts one cast");
        assert_eq!(&casting[0].packet[6..10], &[0xe0, 0x2e, 1, 0]);
        assert!(
            brain.take(1).is_empty(),
            "nothing lands before the cast time"
        );
        // The cast lands after its cast time with a queued spell hit.
        let events = {
            let at = now + lightning.cast_ms;
            for p in brain.players.values_mut() {
                p.at = at;
            }
            brain.tick(&mut inner.mobs, at)
        };
        assert!(
            events
                .iter()
                .any(|e| e.packet[..2] == [0x31, 3] && e.packet[8..10] == [1, 0])
        );
        let hits = brain.take(1);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].amount > 0);
        assert_eq!(hits[0].spell.as_ref().unwrap().skill, 300123);
        // A stun interrupts the next cast, and an invisible target escapes it.
        {
            let at = now + 1500 + lightning.cast_ms;
            for p in brain.players.values_mut() {
                p.at = at;
            }
            brain.tick(&mut inner.mobs, at)
        };
        assert!(brain.agents[&12000].cast.is_some());
        inner.mobs.get_mut(&12000).unwrap().stunned_until = now + 1_000_000;
        {
            let at = now + 1600 + lightning.cast_ms;
            for p in brain.players.values_mut() {
                p.at = at;
            }
            brain.tick(&mut inner.mobs, at)
        };
        assert!(brain.agents[&12000].cast.is_none());
        inner.mobs.get_mut(&12000).unwrap().stunned_until = 0;
        {
            let at = now + 4000 + lightning.cast_ms;
            for p in brain.players.values_mut() {
                p.at = at;
            }
            brain.tick(&mut inner.mobs, at)
        };
        brain.players.get_mut(&1).unwrap().invisible = true;
        {
            let at = now + 4000 + 2 * lightning.cast_ms;
            for p in brain.players.values_mut() {
                p.at = at;
            }
            brain.tick(&mut inner.mobs, at)
        };
        assert!(brain.take(1).is_empty());

        // An area spell (Dread Mare lightning) hits everyone within its radius of the victim, not beyond.
        let fire = npc_magic::Spell::resolve(&catalog, 300236).unwrap();
        assert!(
            !npc_magic::Spell::resolve(&catalog, 300139).unwrap().area(),
            "moral 7 stays single-target"
        );
        assert!(fire.area());
        brain.players.get_mut(&1).unwrap().invisible = false;
        let near_far = |name: &str, dz: u16| Player {
            position: Position {
                z: home.z + dz,
                ..player(name, 40).position
            },
            ..player(name, 40)
        };
        brain
            .players
            .insert(2, near_far("Near", (fire.radius as u16 - 1) * 10));
        brain
            .players
            .insert(3, near_far("Far", (fire.radius as u16 + 5) * 10));
        brain.players.insert(
            4,
            Player {
                nation: 1,
                is_admin: true,
                ..player("Admin", 100)
            },
        );
        let mut strikes = HashMap::new();
        let mob = inner.mobs[&12000].clone();
        let events = cast_effect(&brain.players, &mut strikes, &mut 7, &mob, &fire, 1, true);
        let mut hit: Vec<_> = strikes.keys().copied().collect();
        hit.sort_unstable();
        assert_eq!(hit, [1, 2]);
        assert!(
            events.iter().any(|e| e.packet[8..10] == [0xff, 0xff]),
            "area marker"
        );

        // Harpy poison: damage over time plus an attack debuff; cold-free so never resisted.
        let harpy = npc_magic::Spell::resolve(&catalog, 300135).unwrap();
        let mut strikes = HashMap::new();
        cast_effect(&brain.players, &mut strikes, &mut 7, &mob, &harpy, 1, false);
        let hit = strikes[&1][0].spell.clone().unwrap();
        let (per_tick, duration, _) = hit.dot.unwrap();
        assert!(per_tick < 0 && duration == u64::from(harpy.duration_s) * 1000);
        assert_eq!(hit.debuff, harpy.debuff);
        assert!(hit.debuff.is_some_and(|d| d.duration_ms > 0));
    }
    #[test]
    fn lightning_stun_stops_npc_movement_and_attacks_then_expires_without_extending() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        db.execute_batch(include_str!("npc_ai.sql")).unwrap();
        db.execute(
            "INSERT INTO world_monster_ai VALUES(100,3,100,100,1500,3,4,5,35,35,100,0,0,0,0,1,1500)",
            [],
        )
        .unwrap();
        let world = super::super::World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let mut brain = Brain::default();
        brain.load(&db, None).unwrap();
        let now = skills::now_ms();
        let home = world.inner.lock().unwrap().mobs[&12000].position;
        brain.players.insert(
            1,
            Player {
                name: "StunTest".into(),
                nation: 1,
                position: Position {
                    x: home.x + 100,
                    ..home
                },
                hp: 100,
                invisible: false,
                disguised: false,
                is_admin: false,
                stats: combat::Stats::default(),
                at: now,
            },
        );
        {
            let mut inner = world.inner.lock().unwrap();
            inner
                .mobs
                .get_mut(&12000)
                .unwrap()
                .definition
                .as_mut()
                .unwrap()
                .resistance[2] = 0;
            brain.tick(&mut inner.mobs, now);
            brain.tick(&mut inner.mobs, now + 250);
            assert!(brain.agents[&12000].moving);
            inner.rng = 0;
        }
        let attack = || super::super::Attack {
            position: home,
            range: 50.0,
            attack: 0,
            percent: 0,
            extra: 0,
            skill: true,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: Some(super::super::MagicDamage {
                element: 3,
                first: 20,
                over_time: 0,
                duration: 0,
                charisma: 86,
                bonus: 0,
                raw_first: false,
                raw_over_time: false,
            }),
        };
        world
            .hit_owned(12000, attack(), Some("StunTest"), || Ok(()))
            .unwrap();
        let until = world.inner.lock().unwrap().mobs[&12000].stunned_until;
        assert!(until >= now + 2000);
        world
            .hit_owned(12000, attack(), Some("StunTest"), || Ok(()))
            .unwrap();
        let mut inner = world.inner.lock().unwrap();
        assert_eq!(inner.mobs[&12000].stunned_until, until);
        let position = inner.mobs[&12000].position;
        brain.players.get_mut(&1).unwrap().at = until - 1;
        let events = brain.tick(&mut inner.mobs, until - 1);
        assert!(
            events
                .iter()
                .any(|e| e.packet[0] == 0x0b && e.packet[9..11] == [0, 0])
        );
        assert_eq!(inner.mobs[&12000].position, position);
        assert!(brain.take(1).is_empty());
        brain.players.get_mut(&1).unwrap().at = until + 250;
        brain.tick(&mut inner.mobs, until + 250);
        assert!(inner.mobs[&12000].position.x > position.x);
    }

    #[tokio::test]
    async fn mage_armor_reflects_once_and_mana_shield_commits_with_incoming_damage() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        lobby.skills = Some(crate::skills::Catalog::load(&reko_common::game_dir()).unwrap());
        let world =
            std::sync::Arc::new(super::super::World::load(&reko_common::game_dir()).unwrap());
        s.character.class = 110;
        s.progress.points[8] = 0;
        s.position = Position {
            zone: 21,
            x: 3250,
            z: 4000,
            y: 0,
        };
        s.skills.set_limits(5000, 5000);
        s.skills.hp = 5000;
        s.skills.mana = 1000;
        for (category, skill) in [(25, 110573), (31, 110815)] {
            s.skills.buffs.insert(
                category,
                skills::Buff {
                    skill,
                    until: skills::now_ms() + 120000,
                },
            );
        }
        skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
        world.ai.lock().unwrap().strikes.insert(
            s.player_id,
            VecDeque::from([Strike {
                npc: 10000,
                zone: 21,
                amount: 1000,
                spell: None,
            }]),
        );
        lobby.monsters = Some(world.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        crate::utility::npc_strikes(&mut socket, peer, &lobby, "test", &mut s, skills::now_ms())
            .await;
        let persisted = skills::load(&lobby.db.lock().unwrap(), "test", 0).unwrap();
        assert_eq!((persisted.hp, persisted.mana), (4150, 400));
        assert!(!persisted.buffs.contains_key(&25));
        assert_eq!(world.inner.lock().unwrap().mobs[&10000].hp, 24825);
        world.ai.lock().unwrap().strikes.insert(
            s.player_id,
            VecDeque::from([Strike {
                npc: 10000,
                zone: 21,
                amount: 1000,
                spell: None,
            }]),
        );
        crate::utility::npc_strikes(&mut socket, peer, &lobby, "test", &mut s, skills::now_ms())
            .await;
        assert_eq!((s.skills.hp, s.skills.mana), (3250, 0));
        assert_eq!(world.inner.lock().unwrap().mobs[&10000].hp, 24825);
    }

    #[tokio::test]
    async fn incoming_damage_packet_names_the_attacker_instead_of_encoding_a_delta() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let world =
            std::sync::Arc::new(super::super::World::load(&reko_common::game_dir()).unwrap());
        s.skills.hp = 100;
        s.skills.max_hp = 100;
        crate::skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
        world.ai.lock().unwrap().strikes.insert(
            s.player_id,
            VecDeque::from([Strike {
                npc: 12000,
                zone: 21,
                amount: 17,
                spell: None,
            }]),
        );
        lobby.monsters = Some(world.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        crate::utility::npc_strikes(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            crate::skills::now_ms(),
        )
        .await;
        use tokio::io::AsyncReadExt;
        let mut frames = [0u8; 26];
        client.read_exact(&mut frames).await.unwrap();
        assert_eq!(frames[4], 0x17);
        assert_eq!(frames[17], 0x08);
        assert_eq!(&frames[9..11], &12000u16.to_le_bytes());
        assert_eq!(s.skills.hp, 83);
        assert_eq!(
            crate::skills::load(&lobby.db.lock().unwrap(), "test", 0)
                .unwrap()
                .hp,
            83
        );
        lobby
            .db
            .lock()
            .unwrap()
            .execute("INSERT INTO character_privileges VALUES ('test',0,1)", [])
            .unwrap();
        world.ai.lock().unwrap().strikes.insert(
            s.player_id,
            VecDeque::from([Strike {
                npc: 12000,
                zone: 21,
                amount: 99,
                spell: None,
            }]),
        );
        crate::utility::npc_strikes(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            crate::skills::now_ms(),
        )
        .await;
        assert_eq!(
            s.skills.hp, 83,
            "admin promotion discards already queued damage"
        );
        assert!(world.ai.lock().unwrap().players[&s.player_id].is_admin);
        assert!(world.incoming(s.player_id).is_empty());
        lobby
            .db
            .lock()
            .unwrap()
            .execute("UPDATE character_privileges SET is_admin=0", [])
            .unwrap();
        world.ai.lock().unwrap().strikes.insert(
            s.player_id,
            VecDeque::from([Strike {
                npc: 12000,
                zone: 21,
                amount: 17,
                spell: None,
            }]),
        );
        crate::utility::npc_strikes(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            crate::skills::now_ms(),
        )
        .await;
        assert_eq!(
            s.skills.hp, 66,
            "revocation restores incoming monster damage"
        );
    }

    #[test]
    fn aggressive_and_passive_targets_chase_attack_return_and_reset_on_respawn() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        db.execute_batch(include_str!("npc_ai.sql")).unwrap();
        db.execute("INSERT INTO world_monster_ai VALUES(100,3,100,100,1000,1,4,5,20,30,100,0,0,0,0,1,1000)",[]).unwrap();
        let world = super::super::World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let mut brain = Brain::default();
        brain.load(&db, None).unwrap();
        let mut mobs = world.inner.lock().unwrap();
        let home = mobs.mobs[&12000].position;
        let mut player = Player {
            name: "AI test".into(),
            nation: 1,
            position: Position {
                x: home.x + 100,
                ..home
            },
            hp: 100,
            stats: combat::Stats {
                evasion: 100.0,
                ..Default::default()
            },
            invisible: false,
            disguised: false,
            is_admin: false,
            at: 1000,
        };
        brain.players.insert(1, player.clone());
        brain.tick(&mut mobs.mobs, 1000);
        assert_eq!(brain.agents[&12000].target, Some(1));
        assert_eq!(brain.tick(&mut mobs.mobs, 1250)[0].packet[0], 0x0b);
        assert!(mobs.mobs[&12000].position.x > home.x);
        player.position = mobs.mobs[&12000].position;
        player.at = 1500;
        brain.players.insert(1, player.clone());
        let stopped = brain.tick(&mut mobs.mobs, 1500);
        assert!(
            stopped
                .iter()
                .any(|e| e.packet[0] == 0x0b && e.packet[9..11] == [0, 0]),
            "entering attack range must cancel the last forecast destination"
        );
        assert_eq!(brain.take(1).len(), 1);
        brain.tick(&mut mobs.mobs, 1750);
        assert!(brain.take(1).is_empty());
        player.invisible = true;
        player.at = 2000;
        brain.players.insert(1, player.clone());
        brain.tick(&mut mobs.mobs, 2000);
        assert!(brain.agents[&12000].target.is_none());
        assert!(brain.take(1).is_empty());
        brain.rules.get_mut(&100).unwrap().act = 1;
        brain.agents.clear();
        player.invisible = false;
        player.position = mobs.mobs[&12000].position;
        player.at = 2250;
        brain.players.insert(1, player.clone());
        brain.tick(&mut mobs.mobs, 2250);
        assert!(brain.agents[&12000].target.is_none());
        mobs.mobs
            .get_mut(&12000)
            .unwrap()
            .damage_by
            .insert(player.name.clone(), 1);
        brain.tick(&mut mobs.mobs, 2500);
        assert_eq!(brain.agents[&12000].target, Some(1));
        mobs.mobs.get_mut(&12000).unwrap().hp = 0;
        brain.tick(&mut mobs.mobs, 2750);
        assert!(!brain.agents[&12000].alive);
        let respawn = Position {
            x: home.x + 200,
            ..home
        };
        let mob = mobs.mobs.get_mut(&12000).unwrap();
        mob.hp = 100;
        mob.position = respawn;
        mob.damage_by.clear();
        brain.tick(&mut mobs.mobs, 3000);
        assert_eq!(brain.agents[&12000].home, respawn);
        brain.take(1);
        let size = f32::from(mobs.mobs[&12000].definition.as_ref().unwrap().size);
        let rule = brain.rules.get_mut(&100).unwrap();
        rule.direct = 0;
        rule.bulk = 110.0 / size;
        player.position = Position {
            x: respawn.x + 21,
            z: respawn.z + 4,
            ..respawn
        };
        player.at = 3250;
        brain.players.insert(1, player.clone());
        mobs.mobs
            .get_mut(&12000)
            .unwrap()
            .damage_by
            .insert(player.name.clone(), 1);
        brain.tick(&mut mobs.mobs, 3250);
        assert_eq!(mobs.mobs[&12000].position, respawn);
        assert_eq!(
            brain.take(1).len(),
            1,
            "the final rounded diagonal step must allow a strike"
        );

        player.position.x += 1;
        player.at = 4250;
        brain.players.insert(1, player.clone());
        brain.tick(&mut mobs.mobs, 4250);
        assert!(
            brain.take(1).is_empty(),
            "a player beyond rounding tolerance still requires chase movement"
        );
        brain.remove(1);
        assert!(!brain.players.contains_key(&1));
    }
    #[test]
    fn original_physical_formula_respects_armor_hit_rate_and_damage_cap() {
        let rule = Rule {
            act: 3,
            damage: 100,
            hit: 100.0,
            delay: 1000,
            run: 4.0,
            walk: 2.0,
            stand: 5000,
            range: 5.0,
            search: 20.0,
            trace: 30.0,
            bulk: 1.0,
            direct: 0,
            magic: 0,
            mode: 0,
        };
        let mut p = Player {
            name: "test".into(),
            nation: 1,
            position: Position::default(),
            hp: 100,
            invisible: false,
            disguised: false,
            is_admin: false,
            stats: combat::Stats {
                evasion: 100.0,
                ..Default::default()
            },
            at: 0,
        };
        let mut seed = 1;
        let unarmored: u32 = (0..10000)
            .map(|_| u32::from(physical(&rule, &p, 100, &mut seed)))
            .sum();
        p.stats.defence = 500;
        seed = 1;
        let armored: u32 = (0..10000)
            .map(|_| u32::from(physical(&rule, &p, 100, &mut seed)))
            .sum();
        assert!(unarmored > armored * 2);
        for _ in 0..10000 {
            assert!(physical(&rule, &p, 100, &mut seed) <= 260);
        }
    }

    fn bystander(position: Position, at: u64) -> Player {
        Player {
            name: "Bystander".into(),
            nation: 2,
            position,
            hp: 100,
            invisible: false,
            disguised: false,
            is_admin: false,
            stats: combat::Stats::default(),
            at,
        }
    }

    #[test]
    fn guards_attack_enemy_players_only_in_pvp_or_invaded_zones() {
        use super::super::guard_tests::{GUARD, POST, fixture};
        let Some((world, _db)) = fixture() else {
            return;
        };
        let mut now = skills::now_ms();
        let near = Position {
            x: POST.x + 40,
            ..POST
        };
        let mut run = |player: Player, ticks: usize| {
            let mut hits = 0;
            for _ in 0..ticks {
                now += 250;
                let mut p = player.clone();
                p.at = now;
                world.ai.lock().unwrap().players.insert(1, p);
                world.tick(now);
                hits += world.ai.lock().unwrap().take(1).len();
            }
            hits
        };
        // Moradon is not a PvP zone: an enemy-nation player is ignored.
        assert_eq!(run(bystander(near, 0), 12), 0);
        // An invasion makes the guard chase and strike the enemy player.
        world.set_guard_invaded(21, true);
        assert!(run(bystander(near, 0), 24) > 0, "guard attacks the invader");
        assert!(crate::pvp::is_player_id(
            world.ai.lock().unwrap().guards[&GUARD].target.unwrap_or(1)
        ));
        // Same-nation players are never attacked, even during an invasion.
        let friend = Player {
            nation: 1,
            ..bystander(near, 0)
        };
        world
            .ai
            .lock()
            .unwrap()
            .guards
            .get_mut(&GUARD)
            .unwrap()
            .target = None;
        assert_eq!(run(friend, 12), 0);
        // Ending the invasion stops attacks on enemies.
        world.set_guard_invaded(21, false);
        world
            .ai
            .lock()
            .unwrap()
            .guards
            .get_mut(&GUARD)
            .unwrap()
            .target = None;
        assert_eq!(run(bystander(near, 0), 12), 0);
    }

    #[test]
    fn guards_ignore_players_other_guards_and_same_nation_units() {
        use super::super::guard_tests::{GUARD, MONSTER, POST, fixture, place};
        let Some((world, _db)) = fixture() else {
            return;
        };
        {
            let mut inner = world.inner.lock().unwrap();
            let mut other = inner.mobs[&GUARD].clone();
            other.id = 12002;
            other.position.x = POST.x + 30;
            other.definition.as_mut().unwrap().nation = 2;
            inner.mobs.insert(other.id, other);
        }
        let mut now = skills::now_ms();
        let quiet = |now: &mut u64| {
            for _ in 0..8 {
                *now += 250;
                world
                    .ai
                    .lock()
                    .unwrap()
                    .players
                    .insert(1, bystander(POST, *now));
                assert!(world.tick(*now).iter().all(|e| e.packet[0] != 0x08));
            }
        };
        // An enemy-nation guard and a player stand next to the guard.
        quiet(&mut now);
        {
            let mut ai = world.ai.lock().unwrap();
            assert_eq!(ai.guards[&GUARD].target, None);
            assert_eq!(ai.guards[&12002].target, None, "guards never fight guards");
            assert!(ai.take(1).is_empty(), "guards never attack players");
        }
        // A monster of the guard's own nation is not hostile.
        world.inner.lock().unwrap().mobs.remove(&12002);
        place(&world, MONSTER, 50, 0);
        world
            .inner
            .lock()
            .unwrap()
            .mobs
            .get_mut(&MONSTER)
            .unwrap()
            .definition
            .as_mut()
            .unwrap()
            .nation = 1;
        quiet(&mut now);
        assert_eq!(world.ai.lock().unwrap().guards[&GUARD].target, None);
        assert_eq!(world.inner.lock().unwrap().mobs[&GUARD].position, POST);
        world
            .inner
            .lock()
            .unwrap()
            .mobs
            .get_mut(&MONSTER)
            .unwrap()
            .definition
            .as_mut()
            .unwrap()
            .nation = 0;
        now += 250;
        world.tick(now);
        assert_eq!(
            world.ai.lock().unwrap().guards[&GUARD].target,
            Some(MONSTER)
        );
    }

    #[test]
    fn guards_pick_the_nearest_eligible_monster_and_skip_bosses_and_one_life_spawns() {
        use super::super::guard_tests::{GUARD, MONSTER, fixture, place};
        let Some((world, _db)) = fixture() else {
            return;
        };
        place(&world, MONSTER, 120, 0);
        {
            let mut inner = world.inner.lock().unwrap();
            for (id, dx) in [(12003u16, 40i32), (12004, 60), (12005, 80)] {
                let mut extra = inner.mobs[&MONSTER].clone();
                extra.id = id;
                extra.position.x = (i32::from(extra.position.x) - 120 + dx) as u16;
                inner.mobs.insert(id, extra);
            }
            // 12003 is a rare-boss candidate, 12004 a one-life/summoned spawn.
            inner
                .mobs
                .get_mut(&12004)
                .unwrap()
                .definition
                .as_mut()
                .unwrap()
                .regen_type = 1;
        }
        world.ai.lock().unwrap().exempt.insert(12003);
        world.tick(skills::now_ms() + 250);
        assert_eq!(world.ai.lock().unwrap().guards[&GUARD].target, Some(12005));
        world.inner.lock().unwrap().mobs.get_mut(&12005).unwrap().hp = 0;
        let ai_target =
            |world: &super::super::World| world.ai.lock().unwrap().guards[&GUARD].target;
        world.tick(skills::now_ms() + 500);
        // The guard was still at its post, so it re-acquires the next nearest.
        assert_eq!(ai_target(&world), Some(MONSTER));
    }

    #[test]
    fn monster_keeps_its_player_target_while_a_guard_hits_it() {
        use super::super::guard_tests::{GUARD, MONSTER, fixture, place};
        let Some((world, _db)) = fixture() else {
            return;
        };
        world.ai.lock().unwrap().rules.get_mut(&100).unwrap().act = 3;
        place(&world, MONSTER, 50, 1_000_000);
        let spot = world.inner.lock().unwrap().mobs[&MONSTER].position;
        let mut now = skills::now_ms();
        let mut guarded = false;
        for _ in 0..40 {
            now += 250;
            world
                .ai
                .lock()
                .unwrap()
                .players
                .insert(1, bystander(spot, now));
            world.tick(now);
            let ai = world.ai.lock().unwrap();
            assert_eq!(ai.agents[&MONSTER].target, Some(1));
            guarded = world.inner.lock().unwrap().mobs[&MONSTER].guarded;
            if guarded {
                assert_eq!(ai.guards[&GUARD].target, Some(MONSTER));
                break;
            }
        }
        assert!(guarded, "the guard must land a hit");
        let inner = world.inner.lock().unwrap();
        assert!(inner.mobs[&MONSTER].damage_by.is_empty());
        assert!(inner.mobs[&MONSTER].hp < 1_000_000);
    }

    #[test]
    fn npc_versus_npc_damage_follows_armor_and_great_success_rules() {
        let rule = Rule {
            act: 7,
            damage: 3000,
            hit: 194.0,
            delay: 1500,
            run: 7.0,
            walk: 2.0,
            stand: 1000,
            range: 7.0,
            search: 14.0,
            trace: 35.0,
            bulk: 0.9,
            direct: 0,
            magic: 0,
            mode: 0,
        };
        let mut seed = 3;
        let mut misses = 0;
        for _ in 0..10000 {
            let damage = npc_physical(&rule, 5000, 194.0, &mut seed);
            // Armor above the attack only lets great successes through.
            assert!(damage == 0 || (2100..=3900).contains(&damage));
            misses += u32::from(damage == 0);
        }
        assert!(misses > 8000);
        for _ in 0..10000 {
            let damage = npc_physical(&rule, 500, 194.0, &mut seed);
            assert!(damage == 0 || (1750..=3900).contains(&damage));
        }
    }
}
