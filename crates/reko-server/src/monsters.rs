//! Combat fixtures and imported historical spawn rectangles/respawn timers.

use crate::{movement::Position, skills};
use reko_assets::{CellValue, Table};
use std::{collections::HashMap, path::Path, sync::Mutex};
fn basic_reach(weapon: f32, body: f32) -> f32 {
    ((weapon + body) * 1.2).max(3.0) + 0.15
}
#[path = "npc_ai.rs"]
mod ai;
#[cfg(test)]
#[path = "guard_tests.rs"]
mod guard_tests;

#[derive(Clone)]
pub(crate) struct Event {
    pub personal_hp: Vec<(String, i16)>,
    pub zone: u8,
    pub packet: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct ExperienceCredit {
    pub party: Option<(u32, u32)>,
    pub prototype: u16,
    pub level: u8,
    pub position: Position,
    pub damage: u32,
    pub total_damage: u64,
}

/// Owner of an event-managed world entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EventTag {
    Temple,
    War,
    Siege,
}

/// Who may damage an event entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Restriction {
    /// Players of this nation cannot damage it (own war NPCs and monuments).
    Nation(u8),
    /// Only clan members outside the owning clan may damage it.
    Clan { owner: u16 },
}

/// An event entity died or a monument changed hands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventDeath {
    pub tag: EventTag,
    pub id: u16,
    pub prototype: u16,
    /// Entity nation before the death or capture.
    pub nation: u8,
    pub killer: Option<String>,
    pub captured_by: Option<u8>,
    /// Nation and clan of the last permitted attacker.
    pub attacker: Option<(u8, u16)>,
}

#[derive(Clone, Debug, Default)]
struct EventMark {
    tag: Option<EventTag>,
    restriction: Option<Restriction>,
    /// Monuments change nation instead of dying.
    capture: bool,
    /// Temporary ID: removed and reusable after death.
    summoned: bool,
    dead_since: u64,
    /// Nation and clan of the last permitted attacker, kept for kill credit
    /// when the killer has since gone offline.
    last_attacker: Option<(u8, u16)>,
}

/// Dead temporary entities keep their ID this long so every client has
/// processed the death before the ID is reused.
const SUMMON_ID_REUSE_MS: u64 = 10_000;

fn permits(restriction: Option<Restriction>, attacker: Option<(u8, u16)>) -> bool {
    match restriction {
        None => true,
        Some(Restriction::Nation(nation)) => attacker.is_some_and(|(n, _)| n != nation),
        Some(Restriction::Clan { owner }) => {
            attacker.is_some_and(|(_, clan)| clan != 0 && clan != owner)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn damage_events_attribute_only_actual_direct_and_periodic_damage_to_each_owner() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let world = World::load(&game).unwrap();
        let mut events = world.events.subscribe();
        let position = world.inner.lock().unwrap().mobs[&10000].position;
        world
            .hit_owned(
                10000,
                Attack {
                    position,
                    range: 5.0,
                    attack: 0,
                    percent: 0,
                    extra: 123,
                    skill: true,
                    drain_percent: 0,
                    arrow: false,
                    penetrate: false,
                    magic: None,
                },
                Some("DirectOwner"),
                || Ok(()),
            )
            .unwrap();
        let direct = events.try_recv().unwrap();
        assert_eq!(direct.personal_hp, vec![("DirectOwner".into(), -123)]);
        let now = skills::now_ms();
        {
            let mut inner = world.inner.lock().unwrap();
            let mob = inner.mobs.get_mut(&10000).unwrap();
            mob.hp = 1000;
            mob.dots = vec![
                Dot {
                    owner: Some("Alice".into()),
                    amount: 10,
                    remaining: 1,
                    next: now,
                },
                Dot {
                    owner: Some("Bob".into()),
                    amount: 20,
                    remaining: 1,
                    next: now,
                },
                Dot {
                    owner: Some("Alice".into()),
                    amount: 30,
                    remaining: 1,
                    next: now,
                },
            ];
        }
        let ticks = world.tick(now);
        let tick = ticks
            .iter()
            .find(|e| e.packet.first() == Some(&0x22) && e.packet[1..3] == 10000u16.to_le_bytes())
            .unwrap();
        let parts: HashMap<_, _> = tick.personal_hp.iter().cloned().collect();
        assert_eq!(parts["Alice"], -40);
        assert_eq!(parts["Bob"], -20);
        assert_eq!(world.inner.lock().unwrap().mobs[&10000].hp, 940);
        assert_eq!(
            i16::from_le_bytes(tick.packet[12..14].try_into().unwrap()),
            -60
        );
    }
    #[test]
    fn suppression_gate_removal_and_boss_countdown_work_for_direct_and_periodic_kills() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        for (zone, prototype, boss) in [
            (82, 7033, false),
            (83, 7034, false),
            (82, 7008, true),
            (83, 7013, true),
        ] {
            for periodic in [false, true] {
                let lobby = crate::social::tests::lobby();
                {
                    let db = lobby.db.lock().unwrap();
                    crate::population::tests::seed(&db);
                    db.execute("UPDATE world_spawns SET zone=?1,prototype=?2,hp=50,regen_type=1,x0=2000,x1=2000,z0=2000,z1=2000",rusqlite::params![zone,prototype]).unwrap();
                }
                let world = World::load(&game).unwrap();
                world
                    .load_population(&lobby.db.lock().unwrap(), &game)
                    .unwrap();
                let mut events = world.events.subscribe();
                let position = Position {
                    zone,
                    x: 2000,
                    z: 2000,
                    y: 0,
                };
                let now = skills::now_ms();
                let hit = world
                    .hit_owned(
                        12000,
                        Attack {
                            position,
                            range: 5.0,
                            attack: 0,
                            percent: 0,
                            extra: if periodic { 0 } else { 100 },
                            skill: true,
                            drain_percent: 0,
                            arrow: false,
                            penetrate: false,
                            magic: periodic.then_some(MagicDamage {
                                element: 1,
                                first: 0,
                                over_time: 100,
                                duration: 2,
                                charisma: 0,
                                bonus: 0,
                                raw_over_time: true,
                                raw_first: true,
                            }),
                        },
                        Some("Test"),
                        || Ok(()),
                    )
                    .unwrap();
                let mut packets = Vec::new();
                while let Ok(e) = events.try_recv() {
                    packets.push(e.packet);
                }
                if periodic {
                    assert!(!hit.dead);
                    packets.extend(world.tick(now + 3000).into_iter().map(|e| e.packet));
                } else {
                    assert!(hit.dead);
                }
                let out = vec![0x0a, 2, 0xe0, 0x2e]; // source entity12000
                assert_eq!(packets.contains(&out), !boss);
                assert!(!world.visible_from(12000, position));
                assert_eq!(world.suppression_deadline(zone).is_some(), boss);
                let deadline = world.suppression_deadline(zone);
                assert!(
                    world
                        .hit_owned(
                            12000,
                            Attack {
                                position,
                                range: 5.0,
                                attack: 100,
                                percent: 100,
                                extra: 100,
                                skill: true,
                                drain_percent: 0,
                                arrow: false,
                                penetrate: false,
                                magic: None
                            },
                            Some("Test"),
                            || Ok(())
                        )
                        .is_err()
                );
                world.tick(now + 100000000);
                assert_eq!(world.suppression_deadline(zone), deadline);
                assert!(!world.visible_from(12000, position));
            }
        }
    }
    #[test]
    fn party_monster_death_queues_each_members_rewards_and_kill_once() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let lobby = crate::social::tests::lobby();
        let a = crate::social::tests::player(&lobby, "WorldA", 1, 80);
        let b = crate::social::tests::player(&lobby, "WorldB", 1, 80);
        lobby
            .social
            .lock()
            .unwrap()
            .parties
            .insert(a.player_id, vec![a.player_id, b.player_id]);
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        let mut world = World::load(&game).unwrap();
        world.social = Some(lobby.social.clone());
        world.load_population(&db, &game).unwrap();
        let position = world.inner.lock().unwrap().mobs[&12000].position;
        let hit = world
            .hit_owned(
                12000,
                Attack {
                    position,
                    range: 5.0,
                    attack: 0,
                    percent: 0,
                    extra: 1000,
                    skill: true,
                    drain_percent: 0,
                    arrow: false,
                    penetrate: false,
                    magic: None,
                },
                Some("WorldA"),
                || Ok(()),
            )
            .unwrap();
        assert!(hit.dead);
        for name in ["WorldA", "WorldB"] {
            let credits = world.take_experience(name);
            assert_eq!(credits.len(), 1);
            assert_eq!(credits[0].party, Some((2, 160)));
            assert_eq!(world.take_quest_kills(name), [100]);
            assert!(world.take_experience(name).is_empty());
        }
    }

    #[test]
    fn percent_drains_use_current_health_above_legacy_packet_limit() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let world = World::load(&game).unwrap();
        let position = {
            let mut inner = world.inner.lock().unwrap();
            let mob = inner.mobs.get_mut(&10000).unwrap();
            mob.hp = 1_000_000;
            mob.max_hp = 1_000_000;
            mob.position
        };
        let mut events = world.events.subscribe();
        for (percent, expected, remaining) in [
            (10, 100000, 900000),
            (10, 90000, 810000),
            (5, 40500, 769500),
            (5, 38475, 731025),
        ] {
            let hit = world
                .hit_owned(
                    10000,
                    Attack {
                        position,
                        range: 5.0,
                        attack: 0,
                        percent: 0,
                        extra: 0,
                        skill: true,
                        drain_percent: percent,
                        arrow: false,
                        penetrate: false,
                        magic: None,
                    },
                    Some("DrainTester"),
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(hit.damage, expected);
            assert!(!hit.dead);
            let health = events.try_recv().unwrap().packet;
            assert_eq!(
                u32::from_le_bytes(health[8..12].try_into().unwrap()),
                remaining
            );
            assert_eq!(
                i16::from_le_bytes(health[12..14].try_into().unwrap()),
                -32767
            );
        }
        assert_eq!(
            world.inner.lock().unwrap().mobs[&10000].damage_by["DrainTester"],
            268975
        );
    }

    #[test]
    fn basic_reach_has_owner_multiplier_and_minimum_without_affecting_skill_ranges() {
        assert!((basic_reach(0.1, 0.2) - 3.15).abs() < 0.001);
        assert!((basic_reach(1.0, 4.0) - 6.15).abs() < 0.001);
    }

    #[test]
    fn natural_boss_melee_and_dot_death_use_one_timer_and_manual_summons_do_not() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        crate::bosses::initialize(&db).unwrap();
        db.execute_batch("INSERT INTO world_boss_pools VALUES ('21:test',21,'Test Boss',21600000);INSERT INTO world_boss_members VALUES (12000,'21:test');").unwrap();
        let world = World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let p = world.inner.lock().unwrap().mobs[&12000].position;
        let attack = || Attack {
            position: p,
            range: 5.0,
            attack: 0,
            percent: 100,
            extra: 1000,
            skill: true,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        assert!(
            world
                .hit_owned(12000, attack(), Some("Tester"), || Ok(()))
                .unwrap()
                .dead
        );
        assert!(world.boss_status(21, "Test")[0].contains("06:00:00"));
        let due = world
            .bosses
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .status(21, "", skills::now_ms());
        assert_eq!(due.len(), 1);
        let now = skills::now_ms();
        world.tick(now + 21_599_000);
        assert!(world.validate(12000, p, 100.0).is_err());
        world.tick(now + 21_601_000);
        assert_eq!(world.inner.lock().unwrap().mobs[&12000].hp, 100);
        let mut definition = crate::population::load(&db, &game).unwrap().remove(0);
        definition.position = p;
        let manual = world.summon(definition, &Default::default()).unwrap();
        assert!(
            world
                .hit_owned(manual, attack(), Some("Tester"), || Ok(()))
                .unwrap()
                .dead
        );
        assert!(world.boss_status(21, "")[0].contains("alive"));
        let mut dot = attack();
        dot.extra = 0;
        dot.magic = Some(MagicDamage {
            element: 1,
            first: 0,
            over_time: 100,
            duration: 2,
            charisma: 60,
            bonus: 0,
            raw_first: true,
            raw_over_time: true,
        });
        let p2 = world.inner.lock().unwrap().mobs[&12000].position;
        dot.position = p2;
        world
            .hit_owned(12000, dot, Some("Tester"), || Ok(()))
            .unwrap();
        world.tick(skills::now_ms() + 3000);
        assert_eq!(world.inner.lock().unwrap().mobs[&12000].hp, 0);
        assert!(world.boss_status(21, "")[0].contains("respawn in"));
        assert_eq!(world.take_quest_kills("Tester").len(), 3);
    }
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
        let above_base_stat_cap = MagicDamage {
            charisma: 305,
            ..spell
        };
        assert!(above_base_stat_cap.sample(1000, 7, 0) > spell.sample(1000, 7, 0));
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
            stunned_until: 0,
            modifiers: HashMap::new(),
            definition: None,
            damage_by: HashMap::new(),
            role: Role::Monster,
            guarded: false,
            event: EventMark::default(),
        };
        let packet = mob.spawn();
        assert_eq!(packet.len(), 48);
        assert_eq!(&packet[..9], &[0x0a, 1, 0x10, 0x27, 150, 0, 100, 0, 0]);
        assert_eq!(packet[23], 6);
        assert_eq!(&packet[24..30], b"Target");
        let world = World {
            social: None,
            bosses: Mutex::new(None),
            appearances: std::collections::HashSet::from([100]),
            radii: crate::model_radius::Radii::default(),
            inner: Mutex::new(Inner {
                mobs: HashMap::from([(mob.id, mob)]),
                rng: 1,
            }),
            events: tokio::sync::broadcast::channel(16).0,
            loot: None,
            ai: Mutex::new(ai::Brain::default()),
            suppression: Mutex::new(crate::suppression::Rooms::default()),
            quest_kills: Mutex::new(HashMap::new()),
            experience: Mutex::new(HashMap::new()),
            event_deaths: Mutex::new(Vec::new()),
            armed: Mutex::new(std::collections::HashSet::new()),
            restricted: std::sync::atomic::AtomicUsize::new(0),
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
    fn transformed_basic_melee_uses_form_radius_and_scale_without_extending_skills_or_expired_forms()
     {
        let Some((_lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let game = reko_common::game_dir();
        let world = World::load(&game).unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        let mut definition = crate::population::load(&db, &game).unwrap().remove(0);
        definition.appearance = 1701;
        definition.size = 100;
        definition.hp = 25000;
        definition.position = Position {
            zone: 21,
            x: 3250,
            z: 4000,
            y: 0,
        };
        let target_position = definition.position;
        let target = world.summon(definition, &Default::default()).unwrap();
        let normal = world.radii.player(s.character.race, None).unwrap();
        let transformed = world.radii.player(s.character.race, Some(472350)).unwrap();
        assert!((transformed - world.radii.monsters[&1701] * 2.4).abs() < 0.0001);
        assert!(transformed > normal);
        let target_radius = world.radii.monsters[&1701];
        let human_reach = basic_reach(1.0, (normal + target_radius) / 2.0);
        let form_reach = basic_reach(1.0, (transformed + target_radius) / 2.0);
        let gap = (human_reach + form_reach) / 2.0;
        s.position = Position {
            x: target_position.x - (gap * 10.0).round() as u16,
            ..target_position
        };
        let attack = |position| Attack {
            position,
            range: 1.0,
            attack: 0,
            percent: 100,
            extra: 10000,
            skill: false,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        assert!(
            world
                .hit_session_owned(target, attack(s.position), &s, true, || panic!(
                    "Rejected human reach must not charge"
                ))
                .is_err()
        );
        s.skills.buffs.insert(
            crate::transformation::CATEGORY,
            crate::skills::Buff {
                skill: 472350,
                until: crate::skills::now_ms() + 60000,
            },
        );
        assert_eq!(
            world
                .hit_session_owned(target, attack(s.position), &s, true, || Ok(()))
                .unwrap()
                .damage,
            10000
        );
        assert!(
            world
                .hit_session_owned(target, attack(s.position), &s, false, || panic!(
                    "Transformation must not extend spell reach"
                ))
                .is_err()
        );
        s.skills
            .buffs
            .get_mut(&crate::transformation::CATEGORY)
            .unwrap()
            .until = 0;
        assert!(
            world
                .hit_session_owned(target, attack(s.position), &s, true, || panic!(
                    "Expired form must not retain reach"
                ))
                .is_err()
        );
        s.skills
            .buffs
            .get_mut(&crate::transformation::CATEGORY)
            .unwrap()
            .until = crate::skills::now_ms() + 60000;
        s.position.x = target_position.x - ((form_reach + 1.0) * 10.0).round() as u16;
        assert!(
            world
                .hit_session_owned(target, attack(s.position), &s, true, || panic!(
                    "Distant form attack must not charge"
                ))
                .is_err()
        );
    }
    #[test]
    fn basic_reach_uses_original_joint_radii_and_spawn_scale() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::population::tests::seed(&db);
        let world = World::load(&game).unwrap();
        let mut definition = crate::population::load(&db, &game).unwrap().remove(0);
        definition.appearance = 1701;
        definition.position.x = 3250;
        definition.position.z = 4000;
        definition.hp = 25000;
        definition.size = 100;
        let small = world
            .summon(definition.clone(), &Default::default())
            .unwrap();
        definition.size = 240;
        let large = world.summon(definition, &Default::default()).unwrap();
        let attack = |x| Attack {
            position: Position {
                x,
                z: 4000,
                ..Position::default()
            },
            range: 1.0,
            attack: 0,
            percent: 100,
            extra: 10000,
            skill: false,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        assert!(
            world
                .hit_basic_owned(small, attack(3210), Some("Test"), 2, || panic!(
                    "out-of-range hit charged"
                ))
                .is_err()
        );
        assert_eq!(
            world
                .hit_basic_owned(large, attack(3210), Some("Test"), 2, || Ok(()))
                .unwrap()
                .damage,
            10000
        );
        assert!(
            world
                .hit_basic_owned(large, attack(3180), Some("Test"), 2, || panic!(
                    "out-of-range hit charged"
                ))
                .is_err()
        );
        assert_eq!(
            world
                .hit_basic_owned(small, attack(3230), Some("Test"), 2, || Ok(()))
                .unwrap()
                .damage,
            10000
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

fn lightning_stun_chance(resistance: u16) -> u16 {
    (50.0 - f32::from(resistance) / 2.0).clamp(0.0, 100.0) as u16
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
    stunned_until: u64,
    modifiers: HashMap<u8, Modifier>,
    definition: Option<crate::population::Spawn>,
    damage_by: HashMap<String, u32>,
    role: Role,
    /// A guard damaged this monster: its death rewards nobody until it respawns.
    guarded: bool,
    event: EventMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Monster,
    /// Town guard: simulated, attacks monsters, immune to players.
    Guard,
}

#[derive(Debug, Clone)]
struct Modifier {
    percent: u16,
    until: u64,
}

impl Mob {
    fn from_spawn(s: crate::population::Spawn, role: Role) -> Self {
        Self {
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
            stunned_until: 0,
            modifiers: HashMap::new(),
            definition: Some(s),
            damage_by: HashMap::new(),
            role,
            guarded: false,
            event: EventMark::default(),
        }
    }
    /// Players may attack, debuff, heal-target or otherwise affect this entity.
    fn player_targetable(&self) -> bool {
        self.role == Role::Monster
    }
    fn gate_out(&self) -> Option<Vec<u8>> {
        let s = self.definition.as_ref()?;
        if !matches!((s.position.zone, s.prototype), (82, 7033) | (83, 7034)) {
            return None;
        }
        let mut p = vec![0x0a, 2];
        p.extend(self.id.to_le_bytes());
        Some(p)
    }
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
    pub social: Option<std::sync::Arc<Mutex<crate::social::Hub>>>,
    bosses: Mutex<Option<crate::bosses::Scheduler>>,
    appearances: std::collections::HashSet<u32>,
    radii: crate::model_radius::Radii,
    suppression: Mutex<crate::suppression::Rooms>,
    quest_kills: Mutex<HashMap<String, Vec<u16>>>,
    experience: Mutex<HashMap<String, Vec<ExperienceCredit>>>,
    ai: Mutex<ai::Brain>,
    pub loot: Option<std::sync::Arc<crate::loot::Book>>,
    inner: Mutex<Inner>,
    pub events: tokio::sync::broadcast::Sender<Event>,
    event_deaths: Mutex<Vec<EventDeath>>,
    /// Static NPC IDs temporarily replaced by combat entities.
    armed: Mutex<std::collections::HashSet<u16>>,
    /// Count of restricted or capturable entities; zero skips attacker lookups.
    restricted: std::sync::atomic::AtomicUsize,
}

#[derive(Debug)]
pub(crate) struct Hit {
    pub damage: u32,
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
    pub charisma: u16,
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
    pub fn load_ai(
        &self,
        db: &rusqlite::Connection,
        catalog: Option<&crate::skills::Catalog>,
    ) -> anyhow::Result<()> {
        self.ai.lock().unwrap().load(db, catalog)
    }
    pub fn observe(&self, s: &crate::world_entry::Session, now: u64) {
        self.ai.lock().unwrap().observe(s, now);
    }
    pub fn incoming(&self, id: u16) -> Vec<ai::Strike> {
        self.ai.lock().unwrap().take(id)
    }
    /// Marks invaded homeland zones (`op invade`; war invasions will call it);
    /// their guards and towers then attack enemy-nation players. Not persisted.
    /// Whether Mage Armor can reflect onto this NPC (guards and towers cannot be hit).
    pub(crate) fn reflectable(&self, id: u16) -> bool {
        self.inner
            .lock()
            .unwrap()
            .mobs
            .get(&id)
            .is_some_and(Mob::player_targetable)
    }
    pub(crate) fn set_guard_invaded(&self, zone: u8, invaded: bool) {
        self.ai.lock().unwrap().set_invaded(zone, invaded);
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
        owner: &str,
        pay: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let attacker = self.attacker(Some(owner));
        let mut inner = self.inner.lock().unwrap();
        let mob = inner.mobs.get_mut(&id).ok_or("Unknown combat target.")?;
        if mob.position.zone != position.zone || mob.hp == 0 || !mob.player_targetable() {
            return Err("The target is unavailable.");
        }
        if !permits(mob.event.restriction, attacker) {
            return Err("You cannot attack this target.");
        }
        if f32::from(mob.position.x.abs_diff(position.x))
            .hypot(f32::from(mob.position.z.abs_diff(position.z)))
            > (range + 2.0) * 10.0
        {
            return Err("The target is too far away.");
        }
        if !matches!(curse.category, 1 | 2 | 4 | 5) || !(1..=100).contains(&curse.percent) {
            return Err("Unsupported NPC curse.");
        }
        // Superior Parasite (30% HP reduction) does not apply to unique/boss monsters.
        let is_boss = self
            .bosses
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|b| b.members().contains(&id));
        if curse.category == 1 && curse.percent <= 70 && is_boss {
            return Err("This skill does not apply to unique monsters.");
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
            personal_hp: vec![(owner.to_owned(), -((before - mob.hp).min(32767) as i16))],
            zone: mob.position.zone,
            packet: mob.health(-((before - mob.hp).min(32767) as i16), false),
        };
        drop(inner);
        let _ = self.events.send(event);
        Ok(())
    }
    pub fn stun(&self, id: u16, duration_ms: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mob) = inner.mobs.get_mut(&id)
            && mob.hp > 0
            && mob.player_targetable()
        {
            mob.stunned_until = mob.stunned_until.max(skills::now_ms() + duration_ms);
        }
    }
    pub fn sleep(&self, id: u16, duration_ms: u64) {
        self.stun(id, duration_ms);
    }
    pub fn provoke(&self, id: u16, player_id: u16) {
        let alive = self
            .inner
            .lock()
            .unwrap()
            .mobs
            .get(&id)
            .is_some_and(|m| m.hp > 0 && m.player_targetable());
        if alive {
            self.ai.lock().unwrap().provoke(id, player_id);
        }
    }
    pub fn validate(&self, id: u16, position: Position, range: f32) -> Result<(), &'static str> {
        self.position_if_valid(id, position, range).map(|_| ())
    }
    pub fn position_if_valid(
        &self,
        id: u16,
        position: Position,
        range: f32,
    ) -> Result<Position, &'static str> {
        let inner = self.inner.lock().unwrap();
        let mob = inner.mobs.get(&id).ok_or("Unknown combat target.")?;
        if mob.position.zone != position.zone || mob.hp == 0 || !mob.player_targetable() {
            return Err("The target is unavailable.");
        }
        if f32::from(mob.position.x.abs_diff(position.x))
            .hypot(f32::from(mob.position.z.abs_diff(position.z)))
            > (range + 2.0) * 10.0
        {
            return Err("The target is too far away.");
        }
        Ok(mob.position)
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
                    && m.player_targetable()
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
                    stunned_until: 0,
                    modifiers: HashMap::new(),
                    definition: None,
                    damage_by: HashMap::new(),
                    role: Role::Monster,
                    guarded: false,
                    event: EventMark::default(),
                },
            );
        }
        Ok(Self {
            social: None,
            bosses: Mutex::new(None),
            radii: crate::model_radius::Radii::load(game, &looks)?,
            appearances: looks
                .rows
                .iter()
                .filter_map(|r| match r.first() {
                    Some(CellValue::U32(id)) => Some(*id),
                    _ => None,
                })
                .collect(),
            loot: None,
            ai: Mutex::new(ai::Brain::default()),
            suppression: Mutex::new(crate::suppression::Rooms::default()),
            quest_kills: Mutex::new(HashMap::new()),
            experience: Mutex::new(HashMap::new()),
            inner: Mutex::new(Inner {
                mobs,
                rng: skills::now_ms().max(1),
            }),
            events: tokio::sync::broadcast::channel(256).0,
            event_deaths: Mutex::new(Vec::new()),
            armed: Mutex::new(std::collections::HashSet::new()),
            restricted: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    pub(crate) fn summon(
        &self,
        s: crate::population::Spawn,
        reserved: &std::collections::HashSet<u16>,
    ) -> anyhow::Result<u16> {
        self.summon_with(s, reserved, None, None, true)
    }

    /// One-life temporary entity. Its ID returns to the pool shortly after death
    /// or when its event despawns it. `model_checked` is false only for map
    /// objects whose look comes from the zone file instead of NPC_Looks.
    pub(crate) fn summon_with(
        &self,
        mut s: crate::population::Spawn,
        reserved: &std::collections::HashSet<u16>,
        tag: Option<EventTag>,
        restriction: Option<Restriction>,
        model_checked: bool,
    ) -> anyhow::Result<u16> {
        if model_checked {
            anyhow::ensure!(
                self.appearances.contains(&u32::from(s.appearance)),
                "This monster's model is absent from the original1453 client."
            );
            anyhow::ensure!(
                self.radii.monsters.contains_key(&s.appearance),
                "This monster's original character assets are missing or unsupported."
            );
        }
        anyhow::ensure!(
            s.hp > 0
                && s.size > 0
                && s.name.chars().count() <= 255
                && s.name.chars().all(|c| u32::from(c) <= 255),
            "Invalid monster template."
        );
        let mut inner = self.inner.lock().unwrap();
        let id = (30000..=32767)
            .find(|id| !reserved.contains(id) && !inner.mobs.contains_key(id))
            .ok_or_else(|| anyhow::anyhow!("Temporary monster IDs are all in use."))?;
        s.id = id;
        s.regen_type = 1;
        let mut mob = Mob::from_spawn(s, Role::Monster);
        mob.event = EventMark {
            tag,
            restriction,
            summoned: true,
            ..EventMark::default()
        };
        let event = Event {
            personal_hp: Vec::new(),
            zone: mob.position.zone,
            packet: mob.spawn(),
        };
        inner.mobs.insert(id, mob);
        self.recount(&inner);
        drop(inner);
        let _ = self.events.send(event);
        Ok(id)
    }

    fn recount(&self, inner: &Inner) {
        let count = inner
            .mobs
            .values()
            .filter(|m| m.event.restriction.is_some() || m.event.capture)
            .count();
        self.restricted
            .store(count, std::sync::atomic::Ordering::Relaxed);
    }

    /// Nation and clan of a named attacker, only while restricted entities exist.
    fn attacker(&self, owner: Option<&str>) -> Option<(u8, u16)> {
        if self.restricted.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            return None;
        }
        let hub = self.social.as_ref()?.lock().unwrap();
        let id = hub.named(owner?)?;
        let s = &hub.players[&id].s;
        Some((s.character.nation(), s.clan.id))
    }

    /// Turns static NPCs into one-life combat entities with their existing IDs.
    /// Returns the armed IDs with their nation.
    pub(crate) fn arm(
        &self,
        spawns: Vec<crate::population::Spawn>,
        tag: EventTag,
        capture: bool,
    ) -> Vec<(u16, u8)> {
        let mut inner = self.inner.lock().unwrap();
        let mut armed = self.armed.lock().unwrap();
        let mut result = Vec::new();
        for mut s in spawns {
            if inner.mobs.contains_key(&s.id) {
                continue;
            }
            s.regen_type = 1;
            let nation = s.nation;
            let mut mob = Mob::from_spawn(s, Role::Monster);
            mob.event = EventMark {
                tag: Some(tag),
                restriction: matches!(nation, 1 | 2).then_some(Restriction::Nation(nation)),
                capture,
                ..EventMark::default()
            };
            armed.insert(mob.id);
            result.push((mob.id, nation));
            inner.mobs.insert(mob.id, mob);
        }
        drop(armed);
        self.recount(&inner);
        result
    }

    /// Removes every entity of an event. Temporary entities leave client view;
    /// armed static NPCs revert to their static entries.
    pub(crate) fn despawn_event(&self, tag: EventTag) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let ids: Vec<u16> = inner
            .mobs
            .values()
            .filter(|m| m.event.tag == Some(tag))
            .map(|m| m.id)
            .collect();
        let mut events = Vec::new();
        let mut armed = self.armed.lock().unwrap();
        let mut ai = self.ai.lock().unwrap();
        for id in &ids {
            let mob = inner.mobs.remove(id).unwrap();
            armed.remove(id);
            ai.forget_mob(*id);
            if mob.event.summoned {
                let mut out = vec![0x0a, 2];
                out.extend(id.to_le_bytes());
                events.push(Event {
                    personal_hp: Vec::new(),
                    zone: mob.position.zone,
                    packet: out,
                });
            }
        }
        drop(ai);
        drop(armed);
        self.recount(&inner);
        drop(inner);
        for event in events {
            let _ = self.events.send(event);
        }
        ids.len()
    }

    pub(crate) fn event_alive(&self, tag: EventTag) -> usize {
        self.inner
            .lock()
            .unwrap()
            .mobs
            .values()
            .filter(|m| m.event.tag == Some(tag) && m.hp > 0)
            .count()
    }

    pub(crate) fn take_event_deaths(&self) -> Vec<EventDeath> {
        std::mem::take(&mut *self.event_deaths.lock().unwrap())
    }

    /// Static NPC IDs currently replaced by combat entities.
    pub(crate) fn armed_ids(&self) -> std::collections::HashSet<u16> {
        self.armed.lock().unwrap().clone()
    }

    /// A monument changes nation and look instead of dying. Without a known
    /// capturing nation it is restored unchanged; it never stays dead.
    fn capture(&self, mob: &mut Mob, killer: Option<&String>) -> Vec<Event> {
        let online = killer.and_then(|name| {
            let hub = self.social.as_ref()?.lock().unwrap();
            let id = hub.named(name)?;
            Some(hub.players[&id].s.character.nation())
        });
        let nation = online
            .or(mob.event.last_attacker.map(|(nation, _)| nation))
            .filter(|n| matches!(n, 1 | 2));
        mob.hp = mob.max_hp;
        mob.respawn = 0;
        mob.damage_by.clear();
        mob.dots.clear();
        if let (Some(nation), Some(tag), Some(definition)) =
            (nation, mob.event.tag, mob.definition.as_mut())
        {
            let before = definition.nation;
            definition.nation = nation;
            mob.resource = if nation == 1 { 14003 } else { 14004 };
            mob.event.restriction = Some(Restriction::Nation(nation));
            self.event_deaths.lock().unwrap().push(EventDeath {
                tag,
                id: mob.id,
                prototype: definition.prototype,
                nation: before,
                killer: killer.cloned(),
                captured_by: Some(nation),
                attacker: mob.event.last_attacker,
            });
        }
        let mut out = vec![0x0a, 2];
        out.extend(mob.id.to_le_bytes());
        vec![
            Event {
                personal_hp: Vec::new(),
                zone: mob.position.zone,
                packet: out,
            },
            Event {
                personal_hp: Vec::new(),
                zone: mob.position.zone,
                packet: mob.spawn(),
            },
        ]
    }

    pub fn load_population(&self, db: &rusqlite::Connection, game: &Path) -> anyhow::Result<()> {
        let definitions = crate::population::load(db, game)?;
        *self.suppression.lock().unwrap() = crate::suppression::Rooms::load(&definitions);
        let bosses = crate::bosses::Scheduler::load(db, &definitions, skills::now_ms())?;
        let controlled = bosses.members();
        let active = bosses.active();
        let mut inner = self.inner.lock().unwrap();
        let mut guards = 0;
        for s in definitions
            .into_iter()
            .filter(|s| s.monster || crate::population::simulated_guard(s))
        {
            anyhow::ensure!(
                !inner.mobs.contains_key(&s.id),
                "duplicate world entity {}",
                s.id
            );
            let role = if s.monster {
                Role::Monster
            } else {
                guards += 1;
                Role::Guard
            };
            inner.mobs.insert(s.id, Mob::from_spawn(s, role));
        }
        // Guards never engage rare-boss candidates; suppression rooms and
        // one-life spawns are excluded by zone and regen type in the AI.
        self.ai.lock().unwrap().exempt = controlled.clone();
        for id in &controlled {
            let mob = inner.mobs.get_mut(id).unwrap();
            mob.hp = 0;
            mob.respawn = u64::MAX;
        }
        for s in active {
            let mob = inner.mobs.get_mut(&s.id).unwrap();
            mob.position = s.position;
            mob.hp = mob.max_hp;
            mob.respawn = 0;
        }
        *self.bosses.lock().unwrap() = Some(bosses);
        tracing::info!(
            monsters = inner.mobs.len() - guards,
            guards,
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

    pub fn slow(&self, id: u16, percent: u16, duration: u64, refresh: bool) {
        let now = skills::now_ms();
        if let Some(mob) = self.inner.lock().unwrap().mobs.get_mut(&id)
            && mob.hp > 0
            && mob.player_targetable()
            && (refresh || now >= mob.slow_until)
        {
            mob.speed_percent = percent;
            mob.slow_until = now + duration;
        }
    }

    pub(crate) fn reflect(
        &self,
        id: u16,
        position: Position,
        amount: u16,
        element: u8,
        owner: &str,
    ) -> Result<Hit, &'static str> {
        let resistance = {
            let inner = self.inner.lock().unwrap();
            let mob = inner
                .mobs
                .get(&id)
                .ok_or("Reflection target unavailable.")?;
            if mob.hp == 0 || mob.position.zone != position.zone || !mob.player_targetable() {
                return Err("Reflection target unavailable.");
            }
            mob.resistance(element)
        };
        let reflected = ((230 * u32::from(amount) / (u32::from(resistance) + 250)) / 100 * 25)
            .min(32767) as u16;
        self.hit_owned(
            id,
            Attack {
                position,
                range: 100.0,
                attack: 0,
                percent: 0,
                extra: 0,
                skill: true,
                drain_percent: 0,
                arrow: false,
                penetrate: false,
                magic: Some(MagicDamage {
                    element: 0,
                    first: reflected,
                    over_time: 0,
                    duration: 0,
                    charisma: 0,
                    bonus: 0,
                    raw_over_time: false,
                    raw_first: true,
                }),
            },
            Some(owner),
            || Ok(()),
        )
    }

    /// HP a monster absorbs or drains from a player with its spell.
    pub(crate) fn heal(&self, id: u16, zone: u8, amount: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mob) = inner.mobs.get_mut(&id)
            && mob.hp > 0
            && mob.player_targetable()
            && mob.position.zone == zone
        {
            mob.hp = mob.hp.saturating_add(amount).min(mob.max_hp);
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
        self.hit_with_body(id, attack, owner, pay, None, None)
    }
    #[cfg(test)]
    pub(crate) fn hit_basic_owned(
        &self,
        id: u16,
        attack: Attack,
        owner: Option<&str>,
        race: u8,
        pay: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<Hit, &'static str> {
        self.hit_with_body(id, attack, owner, pay, Some((race, None)), None)
    }
    pub(crate) fn hit_session_owned(
        &self,
        id: u16,
        attack: Attack,
        s: &crate::world_entry::Session,
        basic: bool,
        pay: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<Hit, &'static str> {
        self.hit_with_body(
            id,
            attack,
            Some(&s.character.name),
            pay,
            if basic {
                Some((s.character.race, Some(crate::transformation::appearance(s))))
            } else {
                None
            },
            Some(&s.movement),
        )
    }
    fn hit_with_body(
        &self,
        id: u16,
        attack: Attack,
        owner: Option<&str>,
        pay: impl FnOnce() -> Result<(), &'static str>,
        race: Option<(u8, Option<u32>)>,
        motion: Option<&crate::movement::Validator>,
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
        let attacker = self.attacker(owner);
        let mut inner = self.inner.lock().unwrap();
        inner.rng ^= inner.rng << 13;
        inner.rng ^= inner.rng >> 7;
        inner.rng ^= inner.rng << 17;
        let random = inner.rng;
        let mob = inner.mobs.get_mut(&id).ok_or("Unknown combat target.")?;
        if mob.position.zone != position.zone || mob.hp == 0 || !mob.player_targetable() {
            return Err("The target is unavailable.");
        }
        if !permits(mob.event.restriction, attacker) {
            return Err("You cannot attack this target.");
        }
        if mob.event.tag.is_some() && attacker.is_some() {
            mob.event.last_attacker = attacker;
        }
        let position = motion.map_or(position, |m| {
            m.combat_position(position, mob.position, std::time::Instant::now())
        });
        let dx = (f32::from(mob.position.x) - f32::from(position.x)) / 10.0;
        let dz = (f32::from(mob.position.z) - f32::from(position.z)) / 10.0;
        let body = race
            .and_then(|(race, form)| {
                let player = self.radii.player(race, form)?;
                let target = self.radii.monsters.get(&mob.resource)?;
                let scale = mob
                    .definition
                    .as_ref()
                    .map_or(1.0, |s| f32::from(s.size) / 100.0);
                Some((player + target * scale) / 2.0)
            })
            .unwrap_or(2.0);
        let limit = if race.is_some() {
            basic_reach(range, body)
        } else {
            range + body
        };
        if dx.hypot(dz) > limit {
            tracing::info!(
                target = id,
                player_x = position.x,
                player_z = position.z,
                target_x = mob.position.x,
                target_z = mob.position.z,
                distance = dx.hypot(dz),
                limit,
                "combat range rejected"
            );
            return Err("The target is too far away.");
        }
        pay()?;
        let amount = if drain_percent > 0 {
            ((u64::from(mob.hp) * u64::from(drain_percent) / 100) as u32).max(1)
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
        let amount = amount.saturating_add(u32::from(magic_damage));
        let damage = if drain_percent > 0 {
            amount
        } else {
            amount.min(32767)
        }
        .min(mob.hp);
        if let Some(magic) = magic
            && magic.over_time > 0
            && magic.duration >= 2
            && (attack == 0 || damage > 0)
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
        let now = skills::now_ms();
        let natural_boss = if damage == mob.hp {
            self.boss_death(id, now)?
        } else {
            false
        };
        mob.hp -= damage;
        if damage > 0
            && mob.hp > 0
            && magic.is_some_and(|m| m.element == 3 && !m.raw_first)
            && skills::now_ms() >= mob.stunned_until
            && (random >> 48) % 100 < u64::from(lightning_stun_chance(mob.resistance(3)))
        {
            mob.stunned_until = skills::now_ms() + 2000;
        }
        if let Some(owner) = owner {
            *mob.damage_by.entry(owner.to_owned()).or_default() += damage;
        }
        let dead = mob.hp == 0;
        let health = mob.health(-(damage.min(32767) as i16), false);
        let zone = mob.position.zone;
        let deaths = if dead {
            self.finish_death(mob, now, natural_boss)
        } else {
            Vec::new()
        };
        drop(inner);
        let _ = self.events.send(Event {
            personal_hp: owner
                .map(|name| vec![(name.to_owned(), -(damage.min(32767) as i16))])
                .unwrap_or_default(),
            zone,
            packet: health,
        });
        for event in deaths {
            let _ = self.events.send(event);
        }
        Ok(Hit { damage, dead })
    }
    pub(crate) fn suppression_deadline(&self, zone: u8) -> Option<u64> {
        self.suppression.lock().unwrap().deadline(zone)
    }
    /// Shared death bookkeeping for player hits, damage over time and guard
    /// strikes. The caller has already scheduled any natural-boss timer and set
    /// `hp` to zero. Returns the death, gate-out and loot packets in order.
    fn finish_death(&self, mob: &mut Mob, now: u64, natural_boss: bool) -> Vec<Event> {
        mob.event.dead_since = now;
        if let Some(tag) = mob.event.tag {
            let killer = mob
                .damage_by
                .iter()
                .max_by_key(|(_, damage)| **damage)
                .map(|(name, _)| name.clone());
            if mob.event.capture {
                let events = self.capture(mob, killer.as_ref());
                return events;
            }
            self.event_deaths.lock().unwrap().push(EventDeath {
                tag,
                id: mob.id,
                prototype: mob.definition.as_ref().map_or(0, |s| s.prototype),
                nation: mob.definition.as_ref().map_or(0, |s| s.nation),
                killer,
                captured_by: None,
                attacker: mob.event.last_attacker,
            });
        }
        mob.respawn = if natural_boss {
            u64::MAX
        } else {
            mob.respawn_at(now)
        };
        mob.dots.clear();
        self.quest_death(mob, now);
        let zone = mob.position.zone;
        let event = |packet| Event {
            personal_hp: Vec::new(),
            zone,
            packet,
        };
        let mut events = vec![event(crate::social::death_packet(mob.id).to_vec())];
        events.extend(mob.gate_out().map(event));
        events.extend(self.drop(mob).map(event));
        events
    }
    /// Applies guard swings under the world lock. Each strike is re-validated
    /// so a monster killed earlier in the same tick is finalised only once.
    fn apply_guard_strikes(
        &self,
        inner: &mut Inner,
        strikes: Vec<ai::GuardStrike>,
        now: u64,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        for strike in strikes {
            let Some(guard) = inner
                .mobs
                .get(&strike.guard)
                .filter(|g| g.role == Role::Guard && g.hp > 0)
                .map(|g| g.position)
            else {
                continue;
            };
            let Some(mob) = inner.mobs.get_mut(&strike.target) else {
                continue;
            };
            if mob.role != Role::Monster
                || mob.hp == 0
                || mob.position.zone != guard.zone
                || f32::from(mob.position.x.abs_diff(guard.x))
                    .hypot(f32::from(mob.position.z.abs_diff(guard.z)))
                    > (strike.reach + 2.0) * 10.0
            {
                continue;
            }
            let damage = strike.amount.min(mob.hp);
            let natural_boss = if damage == mob.hp {
                match self.boss_death(mob.id, now) {
                    Ok(boss) => boss,
                    Err(_) => continue,
                }
            } else {
                false
            };
            mob.hp -= damage;
            if damage > 0 {
                mob.guarded = true;
            }
            let result = if mob.hp == 0 { 2 } else { u8::from(damage > 0) };
            let mut packet = vec![0x08, 1, result];
            packet.extend(strike.guard.to_le_bytes());
            packet.extend(strike.target.to_le_bytes());
            events.push(Event {
                personal_hp: Vec::new(),
                zone: mob.position.zone,
                packet,
            });
            if damage > 0 {
                events.push(Event {
                    personal_hp: Vec::new(),
                    zone: mob.position.zone,
                    packet: mob.health(-(damage.min(32767) as i16), false),
                });
            }
            if mob.hp == 0 {
                events.extend(self.finish_death(mob, now, natural_boss));
            }
        }
        events
    }
    fn quest_death(&self, mob: &Mob, now: u64) {
        self.suppression.lock().unwrap().death(mob.id, now);
        if mob.guarded {
            // Guard-damaged kills grant no XP, NP or quest credit to anyone.
            return;
        }
        if let (Some(hub), Some(definition)) = (&self.social, &mob.definition) {
            let hub = hub.lock().unwrap();
            let (credits, kills) = hub.death(
                definition.prototype,
                mob.level,
                mob.position,
                &mob.damage_by,
            );
            let mut pending = self.experience.lock().unwrap();
            for (owner, credit) in credits {
                pending.entry(owner).or_default().push(credit);
            }
            drop(pending);
            let mut pending = self.quest_kills.lock().unwrap();
            for owner in kills {
                pending.entry(owner).or_default().push(definition.prototype);
            }
            return;
        }
        if let Some(definition) = &mob.definition {
            let total_damage = mob.damage_by.values().map(|v| u64::from(*v)).sum();
            let mut credits = self.experience.lock().unwrap();
            for (owner, &damage) in &mob.damage_by {
                if damage > 0 {
                    credits
                        .entry(owner.clone())
                        .or_default()
                        .push(ExperienceCredit {
                            party: None,
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
        if mob.guarded {
            return None;
        }
        let owner = mob.damage_by.iter().max_by_key(|(_, damage)| *damage)?.0;
        self.loot.as_ref()?.create(
            mob.definition.as_ref()?.prototype,
            mob.id,
            mob.position,
            owner,
        )
    }

    fn boss_death(&self, id: u16, now: u64) -> Result<bool, &'static str> {
        self.bosses
            .lock()
            .unwrap()
            .as_mut()
            .map_or(Ok(false), |b| b.death(id, now))
            .map_err(|error| {
                tracing::error!(entity=id,%error,"boss death timer scheduling failed");
                "Boss timer could not be scheduled; target unchanged."
            })
    }
    pub(crate) fn boss_status(&self, zone: u8, query: &str) -> Vec<String> {
        self.bosses
            .lock()
            .unwrap()
            .as_ref()
            .map_or_else(Vec::new, |b| b.status(zone, query, skills::now_ms()))
    }

    pub(crate) fn tick(&self, now: u64) -> Vec<Event> {
        let mut inner = self.inner.lock().unwrap();
        let mut events = Vec::new();
        inner.rng = inner.rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let random = inner.rng;
        let due = self
            .bosses
            .lock()
            .unwrap()
            .as_mut()
            .map_or_else(Vec::new, |b| b.tick(now));
        for s in due {
            let mob = inner.mobs.get_mut(&s.id).unwrap();
            mob.position = s.position;
            mob.hp = mob.max_hp;
            mob.respawn = 0;
            mob.damage_by.clear();
            mob.guarded = false;
            mob.modifiers.clear();
            mob.dots.clear();
            mob.speed_percent = 100;
            mob.slow_until = 0;
            mob.stunned_until = 0;
            mob.next_regen = now + 10000;
            let mut out = vec![0x0a, 2];
            out.extend(mob.id.to_le_bytes());
            events.push(Event {
                personal_hp: Vec::new(),
                zone: mob.position.zone,
                packet: out,
            });
            events.push(Event {
                personal_hp: Vec::new(),
                zone: mob.position.zone,
                packet: mob.spawn(),
            });
        }
        for mob in inner.mobs.values_mut() {
            let hp_modified = mob.modifiers.contains_key(&1);
            mob.modifiers.retain(|_, m| now < m.until);
            if hp_modified && !mob.modifiers.contains_key(&1) && mob.hp > 0 {
                events.push(Event {
                    personal_hp: Vec::new(),
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
                mob.guarded = false;
                mob.modifiers.clear();
                mob.respawn = 0;
                mob.speed_percent = 100;
                mob.slow_until = 0;
                mob.stunned_until = 0;
                mob.dots.clear();
                mob.next_regen = now + 10000;
                let mut out = vec![0x0a, 2];
                out.extend(mob.id.to_le_bytes());
                events.push(Event {
                    personal_hp: Vec::new(),
                    zone: mob.position.zone,
                    packet: out,
                });
                events.push(Event {
                    personal_hp: Vec::new(),
                    zone: mob.position.zone,
                    packet: mob.spawn(),
                });
            } else if mob.hp > 0 {
                let mut loss = 0u32;
                let mut owner_parts = Vec::new();
                for dot in &mut mob.dots {
                    if now >= dot.next && dot.remaining > 0 {
                        let amount = u32::from(dot.amount).min(mob.hp.saturating_sub(loss));
                        loss += amount;
                        if let Some(owner) = dot.owner.as_ref() {
                            *mob.damage_by.entry(owner.clone()).or_default() += amount;
                            owner_parts.push((owner.clone(), amount));
                        }
                        dot.remaining -= 1;
                        dot.next = now + 2000;
                    }
                }
                mob.dots.retain(|dot| dot.remaining > 0);
                let loss = loss.min(mob.hp).min(32767);
                if loss > 0 {
                    let natural_boss = if loss == mob.hp {
                        match self.boss_death(mob.id, now) {
                            Ok(boss) => boss,
                            Err(_) => continue,
                        }
                    } else {
                        false
                    };
                    mob.hp -= loss;
                    events.push(Event {
                        personal_hp: {
                            let mut budget = loss;
                            let mut totals: HashMap<String, u32> = HashMap::new();
                            for (owner, amount) in owner_parts {
                                let actual = amount.min(budget);
                                budget -= actual;
                                *totals.entry(owner).or_default() += actual;
                            }
                            totals
                                .into_iter()
                                .map(|(owner, amount)| (owner, -(amount.min(32767) as i16)))
                                .collect()
                        },
                        zone: mob.position.zone,
                        packet: mob.health(-(loss as i16), false),
                    });
                    if mob.hp == 0 {
                        events.extend(self.finish_death(mob, now, natural_boss));
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
                            personal_hp: Vec::new(),
                            zone: mob.position.zone,
                            packet: mob.health(amount.min(32767) as i16, false),
                        });
                    }
                }
            }
        }
        let strikes = {
            let mut ai = self.ai.lock().unwrap();
            events.extend(ai.tick(&mut inner.mobs, now));
            ai.take_guard_strikes()
        };
        events.extend(self.apply_guard_strikes(&mut inner, strikes, now));
        let expired: Vec<u16> = inner
            .mobs
            .values()
            .filter(|m| {
                m.event.summoned
                    && m.hp == 0
                    && now >= m.event.dead_since.saturating_add(SUMMON_ID_REUSE_MS)
            })
            .map(|m| m.id)
            .collect();
        if !expired.is_empty() {
            let mut ai = self.ai.lock().unwrap();
            for id in expired {
                let mob = inner.mobs.remove(&id).unwrap();
                ai.forget_mob(id);
                let mut out = vec![0x0a, 2];
                out.extend(id.to_le_bytes());
                events.push(Event {
                    personal_hp: Vec::new(),
                    zone: mob.position.zone,
                    packet: out,
                });
            }
            drop(ai);
            self.recount(&inner);
        }
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
