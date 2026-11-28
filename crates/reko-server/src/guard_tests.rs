//! Town guards attacking monsters: World-level regression tests.
use super::*;
use rusqlite::Connection;

pub(super) const GUARD: u16 = 12001;
pub(super) const MONSTER: u16 = 12000;
/// Guard post (zone 21, 110 m / 100 m).
pub(super) const POST: Position = Position {
    zone: 21,
    x: 1100,
    z: 1000,
    y: 0,
};

/// A world with one passive monster far from one Moradon guard (move type 0).
pub(super) fn fixture() -> Option<(World, Connection)> {
    let game = reko_common::game_dir();
    let rules = std::path::Path::new("../../logs/reference-coefficients.csv");
    if !game.join("Data/NPC_Looks.tbl").exists() || !rules.exists() {
        return None;
    }
    let db = Connection::open_in_memory().unwrap();
    crate::population::tests::seed(&db);
    crate::loot::initialize(&db).unwrap();
    db.execute_batch(include_str!("npc_ai.sql")).unwrap();
    db.execute_batch("CREATE TABLE IF NOT EXISTS world_reference_rules (table_name TEXT NOT NULL, rule_id INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY(table_name,rule_id));").unwrap();
    // Passive monster parked 190 m away from the guard; tests move it.
    db.execute(
        "UPDATE world_spawns SET x0=3000,x1=3000,z0=1000,z1=1000,move_type=0",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO world_monster_ai VALUES(100,1,100,100,1500,3,4,5,5,35,100,0,0,0,0,1,1500)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO world_monster_loot VALUES (100,1000,0)", [])
        .unwrap();
    db.execute("INSERT INTO world_spawns VALUES (12001,'test',2,0,0,21,21020,21000,'Guard',11,0,1,60,100,150250000,170250000,0,1100,1000,1100,1000,10000,5000,250,250,250,250,250,250,30000,0,0,'')",[]).unwrap();
    db.execute(
        "INSERT INTO world_reference_rules VALUES ('OBJECT_NPC',21020,?1)",
        [r#"{"sSid":"21020","strName":"Guard","sPid":"21000","sSize":"100","byGroup":"1","byActType":"7","byType":"11","sAc":"5000","sHitRate":"194","sEvadeRate":"194","sDamage":"3000","sAttackDelay":"1500","bySpeed1":"2","bySpeed2":"7","sStandtime":"1000","sBulk":"90","byAttackRange":"7","bySearchRange":"14","byTracingRange":"35","byDirectAttack":"0","byMagicAttack":"0"}"#],
    )
    .unwrap();
    let catalog = crate::combat::Catalog::load(&game, rules).unwrap();
    let mut world = World::load(&game).unwrap();
    world.loot = Some(std::sync::Arc::new(
        crate::loot::Book::load(&db, &catalog).unwrap(),
    ));
    world.load_population(&db, &game).unwrap();
    world.load_ai(&db, None).unwrap();
    Some((world, db))
}

pub(super) fn place(world: &World, id: u16, dx: i32, hp: u32) {
    let mut inner = world.inner.lock().unwrap();
    let mob = inner.mobs.get_mut(&id).unwrap();
    mob.position = Position {
        x: (i32::from(POST.x) + dx) as u16,
        ..POST
    };
    if hp > 0 {
        mob.hp = hp;
        mob.max_hp = hp;
    }
}

fn meters(a: Position, b: Position) -> f32 {
    f32::from(a.x.abs_diff(b.x)).hypot(f32::from(a.z.abs_diff(b.z))) / 10.0
}

fn attack(position: Position, drain_percent: u8) -> Attack {
    Attack {
        position,
        range: 5.0,
        attack: 0,
        percent: 0,
        extra: 1,
        skill: true,
        drain_percent,
        arrow: false,
        penetrate: false,
        magic: None,
    }
}

/// Ticks until the guard lands at least one point of damage; returns all events.
fn until_guarded(world: &World, now: &mut u64) -> Vec<Event> {
    let mut events = Vec::new();
    for _ in 0..80 {
        *now += 250;
        events.extend(world.tick(*now));
        if world.inner.lock().unwrap().mobs[&MONSTER].guarded {
            return events;
        }
    }
    panic!("guard never damaged the monster");
}

#[test]
fn guard_spawn_packet_matches_the_former_static_npc_and_leaves_the_merchant_list() {
    let Some((world, db)) = fixture() else {
        return;
    };
    let game = reko_common::game_dir();
    let definition = crate::population::load(&db, &game)
        .unwrap()
        .into_iter()
        .find(|s| s.id == GUARD)
        .unwrap();
    let merchant = crate::shops::merchant(definition).spawn();
    let live = world
        .spawns(21)
        .into_iter()
        .find(|p| p[2..4] == GUARD.to_le_bytes())
        .unwrap();
    assert_eq!(live, merchant);
    assert!(
        !crate::shops::load_population(&db, &game)
            .unwrap()
            .iter()
            .any(|m| m.id == GUARD)
    );
    // War-zone guards stay static merchants.
    db.execute("UPDATE world_spawns SET zone=61 WHERE id=?1", [GUARD])
        .unwrap();
    assert!(
        crate::shops::load_population(&db, &game)
            .unwrap()
            .iter()
            .any(|m| m.id == GUARD)
    );
    let war = World::load(&game).unwrap();
    war.load_population(&db, &game).unwrap();
    assert!(!war.inner.lock().unwrap().mobs.contains_key(&GUARD));
}

#[test]
fn guard_acquires_inside_14m_kills_without_any_reward_and_ignores_15m() {
    let Some((world, _db)) = fixture() else {
        return;
    };
    let mut now = skills::now_ms();
    place(&world, MONSTER, 150, 0);
    for _ in 0..12 {
        now += 250;
        assert!(world.tick(now).iter().all(|e| e.packet[0] != 0x08));
    }
    assert_eq!(world.inner.lock().unwrap().mobs[&GUARD].position, POST);
    // A player tags the monster, then it wanders into the guard's 14 m range.
    let target = world.inner.lock().unwrap().mobs[&MONSTER].position;
    world
        .hit_owned(MONSTER, attack(target, 0), Some("Tagger"), || Ok(()))
        .unwrap();
    place(&world, MONSTER, 140, 0);
    let mut events = until_guarded(&world, &mut now);
    for _ in 0..40 {
        if world.inner.lock().unwrap().mobs[&MONSTER].hp == 0 {
            break;
        }
        now += 250;
        events.extend(world.tick(now));
    }
    let attacks: Vec<_> = events.iter().filter(|e| e.packet[0] == 0x08).collect();
    assert!(!attacks.is_empty());
    let last = &attacks.last().unwrap().packet;
    assert_eq!(last.len(), 7);
    assert_eq!(&last[..3], &[0x08, 1, 2]);
    assert_eq!(&last[3..5], &GUARD.to_le_bytes());
    assert_eq!(&last[5..7], &MONSTER.to_le_bytes());
    assert_eq!(
        events
            .iter()
            .filter(|e| e.packet == [0x11, 0xe0, 0x2e])
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .all(|e| e.packet[0] != 0x23 && e.personal_hp.is_empty())
    );
    assert!(world.take_experience("Tagger").is_empty());
    assert!(world.take_quest_kills("Tagger").is_empty());
    // The guard never left its 20 m leash and walks back to its post.
    for _ in 0..40 {
        now += 250;
        world.tick(now);
    }
    assert_eq!(world.inner.lock().unwrap().mobs[&GUARD].position, POST);
}

#[test]
fn guard_leashes_at_twenty_meters_returns_to_post_and_stays() {
    let Some((world, _db)) = fixture() else {
        return;
    };
    let mut now = skills::now_ms();
    place(&world, MONSTER, 130, 1_000_000);
    until_guarded(&world, &mut now);
    place(&world, MONSTER, 250, 0);
    for _ in 0..60 {
        now += 250;
        world.tick(now);
        let inner = world.inner.lock().unwrap();
        assert!(meters(inner.mobs[&GUARD].position, POST) <= 20.0);
    }
    let inner = world.inner.lock().unwrap();
    assert_eq!(inner.mobs[&GUARD].position, POST);
    assert!(inner.mobs[&MONSTER].hp > 0 && inner.mobs[&MONSTER].guarded);
}

#[test]
fn taint_blocks_rewards_for_later_player_and_dot_kills_survives_regen_and_resets_on_respawn() {
    for dot in [false, true] {
        let Some((world, _db)) = fixture() else {
            return;
        };
        let mut now = skills::now_ms();
        place(&world, MONSTER, 130, 1_000_000);
        until_guarded(&world, &mut now);
        // Pull the monster away from the guard and let it regenerate.
        place(&world, MONSTER, 900, 0);
        {
            let mut inner = world.inner.lock().unwrap();
            let mob = inner.mobs.get_mut(&MONSTER).unwrap();
            mob.hp = 10;
            mob.next_regen = 0;
        }
        now += 250;
        world.tick(now);
        let (position, hp) = {
            let inner = world.inner.lock().unwrap();
            let mob = &inner.mobs[&MONSTER];
            assert!(mob.guarded, "regeneration keeps the taint");
            (mob.position, mob.hp)
        };
        assert!(hp > 10);
        let mut events = world.events.subscribe();
        if dot {
            let mut a = attack(position, 0);
            a.extra = 0;
            a.magic = Some(MagicDamage {
                element: 1,
                first: 0,
                over_time: u16::MAX,
                duration: 2,
                charisma: 0,
                bonus: 0,
                raw_over_time: true,
                raw_first: true,
            });
            world
                .inner
                .lock()
                .unwrap()
                .mobs
                .get_mut(&MONSTER)
                .unwrap()
                .hp = 100;
            world
                .hit_owned(MONSTER, a, Some("Killer"), || Ok(()))
                .unwrap();
            now = skills::now_ms() + 2500;
            let ticked = world.tick(now);
            assert!(ticked.iter().any(|e| e.packet == [0x11, 0xe0, 0x2e]));
            assert!(ticked.iter().all(|e| e.packet[0] != 0x23));
        } else {
            assert!(
                world
                    .hit_owned(MONSTER, attack(position, 100), Some("Killer"), || Ok(()))
                    .unwrap()
                    .dead
            );
        }
        while let Ok(e) = events.try_recv() {
            assert_ne!(e.packet[0], 0x23, "tainted monsters drop no loot");
        }
        assert_eq!(world.inner.lock().unwrap().mobs[&MONSTER].hp, 0);
        assert!(world.take_experience("Killer").is_empty());
        assert!(world.take_quest_kills("Killer").is_empty());
        // Respawn clears the taint; an untainted kill rewards normally.
        now = world.inner.lock().unwrap().mobs[&MONSTER].respawn;
        world.tick(now);
        let position = {
            let inner = world.inner.lock().unwrap();
            let mob = &inner.mobs[&MONSTER];
            assert!(mob.hp > 0 && !mob.guarded);
            mob.position
        };
        let mut events = world.events.subscribe();
        assert!(
            world
                .hit_owned(MONSTER, attack(position, 100), Some("Killer"), || Ok(()))
                .unwrap()
                .dead
        );
        assert_eq!(world.take_experience("Killer").len(), 1);
        assert_eq!(world.take_quest_kills("Killer"), [100]);
        let mut looted = false;
        while let Ok(e) = events.try_recv() {
            looted |= e.packet[0] == 0x23;
        }
        assert!(looted, "untainted kills keep their loot");
    }
}

#[test]
fn party_kill_of_a_tainted_monster_credits_no_member() {
    let Some((mut world, _db)) = fixture() else {
        return;
    };
    let lobby = crate::social::tests::lobby();
    let a = crate::social::tests::player(&lobby, "GuardA", 1, 80);
    let b = crate::social::tests::player(&lobby, "GuardB", 1, 80);
    lobby
        .social
        .lock()
        .unwrap()
        .parties
        .insert(a.player_id, vec![a.player_id, b.player_id]);
    world.social = Some(lobby.social.clone());
    let mut now = skills::now_ms();
    place(&world, MONSTER, 130, 1_000_000);
    until_guarded(&world, &mut now);
    place(&world, MONSTER, 900, 0);
    let position = world.inner.lock().unwrap().mobs[&MONSTER].position;
    assert!(
        world
            .hit_owned(MONSTER, attack(position, 100), Some("GuardA"), || Ok(()))
            .unwrap()
            .dead
    );
    for name in ["GuardA", "GuardB"] {
        assert!(world.take_experience(name).is_empty());
        assert!(world.take_quest_kills(name).is_empty());
    }
}

#[test]
fn players_cannot_hit_area_debuff_or_validate_guards() {
    let Some((world, _db)) = fixture() else {
        return;
    };
    let pay = || -> Result<(), &'static str> { panic!("guard targets must not charge") };
    assert!(
        world
            .hit_owned(GUARD, attack(POST, 100), Some("P"), pay)
            .is_err()
    );
    assert!(world.hit(GUARD, attack(POST, 0), pay).is_err());
    assert!(world.reflect(GUARD, POST, 1000, 1, "P").is_err());
    assert!(world.validate(GUARD, POST, 5.0).is_err());
    assert!(world.position_if_valid(GUARD, POST, 5.0).is_err());
    assert!(!world.area_targets(POST, 50.0).contains(&GUARD));
    let curse = Curse {
        category: 2,
        percent: 50,
        duration_ms: 10000,
    };
    assert!(world.curse(GUARD, POST, 5.0, curse, "P", pay).is_err());
    world.inner.lock().unwrap().mobs.get_mut(&GUARD).unwrap().hp = 9000;
    world.stun(GUARD, 10000);
    world.sleep(GUARD, 10000);
    world.slow(GUARD, 50, 10000, true);
    world.heal(GUARD, 21, 1);
    {
        let inner = world.inner.lock().unwrap();
        let guard = &inner.mobs[&GUARD];
        assert_eq!(guard.stunned_until, 0);
        assert_eq!(guard.speed_percent, 100);
        assert!(guard.modifiers.is_empty());
        assert_eq!(guard.hp, 9000, "players cannot heal guards");
    }
    // The target bar still works for guards.
    assert!(world.health(GUARD, 21).is_some());
    assert!(world.visible_from(GUARD, POST));
}

#[test]
fn guard_strikes_revalidate_so_a_concurrent_kill_finalises_once() {
    let Some((world, _db)) = fixture() else {
        return;
    };
    place(&world, MONSTER, 15, 0);
    let strike = |amount| ai::GuardStrike {
        guard: GUARD,
        target: MONSTER,
        amount,
        reach: 2.0,
    };
    let now = skills::now_ms();
    let mut inner = world.inner.lock().unwrap();
    let events = world.apply_guard_strikes(&mut inner, vec![strike(500), strike(500)], now);
    assert_eq!(events.iter().filter(|e| e.packet[0] == 0x11).count(), 1);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.packet[0] == 0x08)
            .map(|e| e.packet[2])
            .collect::<Vec<_>>(),
        [2]
    );
    // A missed swing still animates but does not taint a fresh monster.
    let mob = inner.mobs.get_mut(&MONSTER).unwrap();
    mob.hp = 100;
    mob.guarded = false;
    let events = world.apply_guard_strikes(&mut inner, vec![strike(0)], now);
    assert_eq!(events[0].packet, [0x08, 1, 0, 0xe1, 0x2e, 0xe0, 0x2e]);
    assert!(!inner.mobs[&MONSTER].guarded);
    // Out-of-reach and dead targets are ignored.
    inner.mobs.get_mut(&MONSTER).unwrap().position.x = POST.x + 200;
    assert!(
        world
            .apply_guard_strikes(&mut inner, vec![strike(500)], now)
            .is_empty()
    );
    let mob = inner.mobs.get_mut(&MONSTER).unwrap();
    mob.position.x = POST.x + 15;
    mob.hp = 0;
    assert!(
        world
            .apply_guard_strikes(&mut inner, vec![strike(500)], now)
            .is_empty()
    );
}
