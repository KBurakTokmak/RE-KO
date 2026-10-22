//! Data-driven pursuit and physical attacks; owned by the world simulation.
use super::{Event, Mob};
use crate::{combat, movement::Position, skills, world_entry::Session};
use rusqlite::Connection;
use std::collections::{HashMap, VecDeque};

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
}
#[derive(Clone)]
struct Player {
    name: String,
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
    brain.load(&db).unwrap();
    let mut inner = world.inner.lock().unwrap();
    let home = inner.mobs[&12000].position;
    let player = Player {
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
    roam: Option<Position>,
    next_roam: u64,
    pattern: i8,
    moving: bool,
}
#[derive(Clone)]
pub(crate) struct Strike {
    pub npc: u16,
    pub zone: u8,
    pub amount: u16,
}
#[derive(Default)]
pub(super) struct Brain {
    rules: HashMap<u16, Rule>,
    players: HashMap<u16, Player>,
    agents: HashMap<u16, Agent>,
    strikes: HashMap<u16, VecDeque<Strike>>,
    rng: u64,
}
fn distance(a: Position, b: Position) -> f32 {
    f32::from(a.x.abs_diff(b.x)).hypot(f32::from(a.z.abs_diff(b.z))) / 10.0
}
fn random(seed: &mut u64, max: u32) -> u32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 16) as u32 % (max + 1)
}
fn physical(rule: &Rule, target: &Player, attack_percent: u16, seed: &mut u64) -> u16 {
    let ac = u32::from(target.stats.defence);
    let hit = u32::from(rule.damage) * u32::from(attack_percent) * 2 / (ac + 240);
    if hit == 0 {
        return 0;
    }
    let ratio = rule.hit / target.stats.evasion.max(0.001);
    let (critical, success) = if ratio >= 5.0 {
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
    };
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
        zone: mob.position.zone,
        packet: p,
    }
}
fn roam_destination(agent: &mut Agent, position: Position, rng: &mut u64) -> Position {
    if agent.pattern == 0 {
        agent.pattern = random(rng, 2) as i8 - 1;
        return agent.home;
    }
    let offset = if matches!(agent.pattern, -2 | -1 | 1 | 2) {
        20
    } else {
        -20
    };
    agent.pattern = match agent.pattern {
        -4 | 4 => 0,
        -3..=-1 => agent.pattern - 1,
        _ => agent.pattern + 1,
    };
    Position {
        x: position.x.saturating_add_signed(offset),
        z: position.z.saturating_add_signed(offset),
        ..position
    }
}
impl Brain {
    pub fn load(&mut self, db: &Connection) -> anyhow::Result<()> {
        let mut q=db.prepare("SELECT prototype,act_type,damage,hit_rate,attack_delay,run_speed,attack_range,search_range,tracing_range,bulk,direct_attack,walk_speed,stand_ms FROM world_monster_ai")?;
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
                },
            ))
        })? {
            let (id, rule) = r?;
            self.rules.insert(id, rule);
        }
        self.rng = skills::now_ms();
        tracing::info!(prototypes = self.rules.len(), "monster AI rules loaded");
        Ok(())
    }
    pub fn observe(&mut self, s: &Session, now: u64) {
        self.players.insert(
            s.player_id,
            Player {
                is_admin: s.is_admin,
                name: s.character.name.clone(),
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
    pub fn remove(&mut self, id: u16) {
        self.players.remove(&id);
        self.strikes.remove(&id);
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
        for mob in mobs.values_mut() {
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
                roam: None,
                next_roam: now.saturating_add(rule.stand),
                pattern: 0,
                moving: false,
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
                agent.moving = false;
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
                if agent.moving {
                    events.push(movement_event(mob, 0.0, mob.position));
                    agent.moving = false;
                }
                continue;
            }
            let damage = mob.damage_by.values().map(|v| u64::from(*v)).sum();
            if damage > agent.damage_seen {
                agent.returning = false;
                agent.chase_start = mob.position;
            }
            agent.damage_seen = damage;
            let valid = |p: &Player| {
                p.hp > 0 && !p.invisible && !p.is_admin && p.position.zone == mob.position.zone
            };
            let attacked = |p: &Player| {
                mob.damage_by.get(&p.name).is_some_and(|damage| {
                    *damage > agent.damage_floor.get(&p.name).copied().unwrap_or(0)
                })
            };
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
                }
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
            if let Some(id) = agent.target {
                let target = &self.players[&id];
                let bulk = rule.bulk * f32::from(s.size) / 100.0;
                let reach = if rule.direct == 1 {
                    5.0f32.min(rule.range)
                } else {
                    1.0f32.min(rule.range)
                } + bulk;
                let dist = distance(mob.position, target.position);
                if dist > reach && s.move_type != 4 {
                    if let Some(e) = move_toward(
                        mob,
                        target.position,
                        (speed * elapsed).min((dist - reach).max(0.0)),
                        speed,
                        reach,
                    ) {
                        events.push(e);
                    }
                } else if dist <= reach && now >= agent.next_attack {
                    agent.next_attack = now.saturating_add(rule.delay.max(1));
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
                        });
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
                    let destination = roam_destination(agent, mob.position, &mut self.rng);
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
            if events.len() > event_start {
                agent.moving = events.last().is_some_and(|e| e.packet[9..11] != [0, 0]);
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
            brain.load(&db).unwrap();
            brain.rules.get_mut(&100).unwrap().act = act;
            let mut admin = Player {
                name: "Admin".into(),
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
        brain.load(&db).unwrap();
        let mut inner = world.inner.lock().unwrap();
        let mobs = &mut inner.mobs;
        let home = mobs[&12000].position;
        let mut p = Player {
            name: "Ranged attacker".into(),
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
        brain.load(&db).unwrap();
        let mut inner = world.inner.lock().unwrap();
        let mobs = &mut inner.mobs;
        let home = mobs[&12000].position;
        let mut p = Player {
            name: "Observer".into(),
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
        brain.load(&db).unwrap();
        let now = skills::now_ms();
        let home = world.inner.lock().unwrap().mobs[&12000].position;
        brain.players.insert(
            1,
            Player {
                name: "StunTest".into(),
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
        brain.load(&db).unwrap();
        let mut mobs = world.inner.lock().unwrap();
        let home = mobs.mobs[&12000].position;
        let mut player = Player {
            name: "AI test".into(),
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
        };
        let mut p = Player {
            name: "test".into(),
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
}
