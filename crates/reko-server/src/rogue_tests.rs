use crate::{
    combat, inventory, lobby::Lobby, magic, monsters, resources, skills, starter, utility,
    world_entry::Session,
};
use rusqlite::{Connection, params};
use std::{net::SocketAddr, sync::Arc};
use tokio::net::{TcpListener, TcpStream};

async fn setup() -> Option<(Lobby, Session, TcpStream, TcpStream, SocketAddr)> {
    let game = reko_common::game_dir();
    if !game.join("Data/Skill_Magic_Main_us.tbl").exists() {
        return None;
    }
    let mut lobby = Lobby::new(
        Connection::open_in_memory().unwrap(),
        crate::characters::CreationRules::load(&game).unwrap(),
        crate::world_entry::Bootstrap { next_level_exp: 50 },
    )
    .unwrap();
    lobby.skills = Some(skills::Catalog::load(&game).unwrap());
    lobby.monsters = Some(Arc::new(monsters::World::load(&game).unwrap()));
    let character = crate::characters::Character {
        slot: 0,
        name: "AdminRogue80".into(),
        race: 2,
        class: 108,
        face: 0,
        hair: 0,
        stats: [60, 150, 255, 50, 50],
    };
    {
        let mut db = lobby.db.lock().unwrap();
        crate::characters::ensure_account(&db, "admin").unwrap();
        crate::characters::set_nation(&db, "admin", 1).unwrap();
        crate::characters::create(&mut db, "admin", &character).unwrap();
        db.execute("INSERT INTO character_privileges VALUES ('admin',0,1)", [])
            .unwrap();
        for (slot, id, count) in [
            (0, 391010000, 100),
            (1, 370004000, 2),
            (2, 379064000, 2),
            (27, 379060000, 2),
        ] {
            db.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES ('admin',0,?1,?2,1,?3)",params![slot,id,count]).unwrap();
        }
    }
    let mut session = Session::new(character).unwrap();
    session.entered = true;
    session.progress.level = 80;
    session.progress.points = [0, 0, 0, 0, 0, 80, 80, 80, 20];
    session.position.x = 3250;
    session.position.z = 4000;
    session.base_combat = combat::Stats {
        hp: 2690,
        mp: 4110,
        attack: 131,
        defence: 80,
        weapon_kind: 70,
        range: 35.0,
        ..combat::Stats::default()
    };
    session.combat = session.base_combat;
    session.skills.set_limits(2690, 4110);
    skills::save(&lobby.db.lock().unwrap(), "admin", 0, &session.skills).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (socket, peer) = listener.accept().await.unwrap();
    Some((lobby, session, socket, client, peer))
}

#[tokio::test]
async fn basic_r_uses_validated_final_stop_before_damage_without_trusting_reported_distance() {
    let Some((lobby, mut s, mut socket, _client, peer)) = setup().await else {
        return;
    };
    s.combat.weapon_kind = 11;
    s.combat.range = 1.0;
    s.position.x = 3262;
    s.position.z = 4034;
    let health = || {
        let p = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
        u32::from_le_bytes(p[8..12].try_into().unwrap())
    };
    let before = health();
    let mut session = Some(s);
    let mut account = Some("admin".into());
    let attack = [0x08, 1, 1, 0x10, 0x27, 0x77, 0, 9, 0];
    for _ in 0..3 {
        crate::handle_game_payload_inner(
            &mut socket,
            peer,
            &attack,
            &lobby,
            &mut account,
            &mut session,
        )
        .await;
        assert_eq!(
            health(),
            before,
            "a short client-reported distance cannot bypass authoritative reach"
        );
    }
    let mut stop = vec![6];
    stop.extend(6000u16.to_le_bytes());
    stop.extend(4022u16.to_le_bytes());
    stop.extend([0; 5]);
    crate::handle_game_payload_inner(&mut socket, peer, &stop, &lobby, &mut account, &mut session)
        .await;
    assert_eq!(
        session.as_ref().unwrap().position.x,
        3262,
        "forged final positions still fail movement validation"
    );
    stop[1..3].copy_from_slice(&3259u16.to_le_bytes());
    crate::handle_game_payload_inner(&mut socket, peer, &stop, &lobby, &mut account, &mut session)
        .await;
    let saved = crate::movement::load(&lobby.db.lock().unwrap(), "admin", 0).unwrap();
    assert_eq!((saved.x, saved.z), (3259, 4022));
    crate::handle_game_payload_inner(
        &mut socket,
        peer,
        &attack,
        &lobby,
        &mut account,
        &mut session,
    )
    .await;
    assert!(
        health() < before,
        "native movement-before-R ordering must resolve the stale-stop miss"
    );
    let hit = health();
    crate::handle_game_payload_inner(
        &mut socket,
        peer,
        &attack,
        &lobby,
        &mut account,
        &mut session,
    )
    .await;
    assert_eq!(
        health(),
        hit,
        "position correction cannot bypass attack cooldown"
    );
}

#[tokio::test]
async fn recovery_items_pay_once_clamp_and_reject_missing_or_forged_use() {
    let Some((mut lobby, mut s, mut socket, _client, peer)) = setup().await else {
        return;
    };
    let game = reko_common::game_dir();
    let rules = std::path::Path::new("../../logs/reference-coefficients.csv");
    if !rules.exists() {
        return;
    }
    lobby.combat = Some(combat::Catalog::load(&game, rules).unwrap());
    {
        let db = lobby.db.lock().unwrap();
        db.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES ('admin',0,3,389014000,1,2),('admin',0,4,389020000,1,1)",[]).unwrap();
        s.skills.hp = 500;
        s.skills.mana = 100;
        skills::save(&db, "admin", 0, &s.skills).unwrap();
    }
    let player = s.player_id;
    cast(&lobby, &mut s, &mut socket, peer, 490014, player, 3).await;
    assert_eq!(s.skills.hp, 1220);
    cast(&lobby, &mut s, &mut socket, peer, 490014, player, 3).await;
    assert_eq!(s.skills.hp, 1220);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[3].count,
        1
    );
    cast(&lobby, &mut s, &mut socket, peer, 490020, 10000, 3).await;
    assert_eq!(s.skills.mana, 100);
    cast(&lobby, &mut s, &mut socket, peer, 490020, player, 3).await;
    assert_eq!(s.skills.mana, 2020);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[4].id,
        0
    );
    {
        let db = lobby.db.lock().unwrap();
        s.skills.cooldowns.clear();
        s.skills.hp = 2600;
        db.execute("DELETE FROM character_skill_cooldowns", [])
            .unwrap();
        skills::save(&db, "admin", 0, &s.skills).unwrap();
    }
    cast(&lobby, &mut s, &mut socket, peer, 490014, player, 3).await;
    assert_eq!(s.skills.hp, 2690);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[3].id,
        0
    );
}

async fn cast(
    lobby: &Lobby,
    session: &mut Session,
    socket: &mut TcpStream,
    peer: SocketAddr,
    id: u32,
    target: u16,
    stage: u8,
) {
    let mut packet = skills::effect(id, session.player_id);
    packet[1] = stage;
    packet[8..10].copy_from_slice(&target.to_le_bytes());
    assert!(magic::handle(socket, peer, &packet, lobby, Some("admin"), Some(session)).await);
}

fn hp(lobby: &Lobby) -> u32 {
    let packet = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
    u32::from_le_bytes(packet[8..12].try_into().unwrap())
}

#[tokio::test]
async fn spike_and_basic_melee_follow_recent_movement_without_extending_sideways_or_stopped_reach()
{
    for (moving, target, accepted) in [
        (true, 10000, true),
        (false, 10000, false),
        (true, 10002, false),
    ] {
        let Some((lobby, mut s, mut socket, _client, peer)) = setup().await else {
            return;
        };
        s.combat.range = 1.0;
        s.combat.weapon_kind = 11;
        s.base_combat = s.combat;
        let mut packet = vec![6];
        packet.extend(3385u16.to_le_bytes());
        packet.extend(4000u16.to_le_bytes());
        packet.extend(0i16.to_le_bytes());
        packet.extend(90i16.to_le_bytes());
        packet.push(1);
        s.position = s
            .movement
            .validate(&packet, s.position, std::time::Instant::now(), 90)
            .unwrap();
        if !moving {
            packet[7..9].copy_from_slice(&0i16.to_le_bytes());
            packet[9] = 0;
            s.position = s
                .movement
                .validate(
                    &packet,
                    s.position,
                    std::time::Instant::now() + std::time::Duration::from_millis(1500),
                    90,
                )
                .unwrap();
        }
        let world = lobby.monsters.as_ref().unwrap();
        let basic = monsters::Attack {
            position: s.position,
            range: 1.0,
            attack: 131,
            percent: 100,
            extra: 0,
            skill: false,
            drain_percent: 0,
            arrow: false,
            penetrate: false,
            magic: None,
        };
        let mut charged = false;
        let result = world.hit_session_owned(target, basic, &s, true, || {
            charged = true;
            Ok(())
        });
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(charged, accepted);
        let health = || {
            let p = world.health(target, 21).unwrap();
            u32::from_le_bytes(p[8..12].try_into().unwrap())
        };
        let before = health();
        let mana = s.skills.mana;
        cast(&lobby, &mut s, &mut socket, peer, 108655, target, 3).await;
        if accepted {
            assert!(health() < before);
            assert!(s.skills.mana < mana);
        } else {
            assert_eq!(health(), before);
            assert_eq!(s.skills.mana, mana);
        }
    }
}

#[tokio::test]
async fn cures_preserve_buffs_and_healing_and_rollback_without_reagents() {
    let Some((lobby, mut s, mut socket, _client, peer)) = setup().await else {
        return;
    };
    let catalog = lobby.skills.as_ref().unwrap();
    if !catalog.effects.contains_key(&(5, 108736)) {
        return;
    }
    let cursed = catalog
        .spells
        .values()
        .find(|spell| {
            spell.types[0] == 4
                && spell.moral == 7
                && catalog
                    .effects
                    .get(&(4, spell.id))
                    .is_some_and(|r| combat::integer(r, 1) == 8)
        })
        .unwrap()
        .id;
    let now = skills::now_ms();
    s.skills.buffs.insert(
        8,
        skills::Buff {
            skill: cursed,
            until: now + 30000,
        },
    );
    s.skills.buffs.insert(
        6,
        skills::Buff {
            skill: 108725,
            until: now + 30000,
        },
    );
    {
        let db = lobby.db.lock().unwrap();
        skills::save(&db, "admin", 0, &s.skills).unwrap();
    }
    let player = s.player_id;
    cast(&lobby, &mut s, &mut socket, peer, 108736, player, 1).await;
    cast(&lobby, &mut s, &mut socket, peer, 108736, player, 3).await;
    assert!(s.skills.buffs.contains_key(&8));
    assert_eq!(s.skills.mana, 4110);
    {
        let db = lobby.db.lock().unwrap();
        db.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES ('admin',0,3,370005000,1,1),('admin',0,4,370006000,1,1)",[]).unwrap();
    }
    cast(&lobby, &mut s, &mut socket, peer, 108736, player, 1).await;
    cast(&lobby, &mut s, &mut socket, peer, 108736, player, 3).await;
    assert!(!s.skills.buffs.contains_key(&8));
    assert!(s.skills.buffs.contains_key(&6));
    assert_eq!(s.skills.mana, 4030);
    s.skills.periodic.insert(
        1,
        skills::Periodic {
            amount: -100,
            next: now + 2000,
            until: now + 10000,
        },
    );
    s.skills.periodic.insert(
        2,
        skills::Periodic {
            amount: 60,
            next: now + 2000,
            until: now + 10000,
        },
    );
    skills::save(&lobby.db.lock().unwrap(), "admin", 0, &s.skills).unwrap();
    cast(&lobby, &mut s, &mut socket, peer, 108748, player, 1).await;
    cast(&lobby, &mut s, &mut socket, peer, 108748, player, 3).await;
    assert!(!s.skills.periodic.contains_key(&1));
    assert!(s.skills.periodic.contains_key(&2));
    assert_eq!(s.skills.mana, 3950);
    s.skills.hp = 1000;
    s.skills.tick_periodic(now + 2000);
    assert_eq!(s.skills.hp, 1060);
}

#[tokio::test]
async fn arrows_are_paid_once_and_impacts_are_bounded() {
    let Some((lobby, mut session, mut socket, _client, peer)) = setup().await else {
        return;
    };
    let before = hp(&lobby);
    cast(&lobby, &mut session, &mut socket, peer, 108515, 10000, 3).await;
    assert_eq!(hp(&lobby), before);
    assert_eq!(session.skills.mana, 4110);
    cast(&lobby, &mut session, &mut socket, peer, 108515, 10000, 2).await;
    assert_eq!(session.skills.mana, 4070);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[0].count,
        97
    );
    for _ in 0..3 {
        cast(&lobby, &mut session, &mut socket, peer, 108515, 10000, 3).await;
    }
    assert!(hp(&lobby) < before);
    assert_eq!(session.skills.mana, 4070);
    let after = hp(&lobby);
    cast(&lobby, &mut session, &mut socket, peer, 108515, 10000, 3).await;
    assert_eq!(hp(&lobby), after);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[0].count,
        97
    );
    cast(&lobby, &mut session, &mut socket, peer, 108510, 10000, 2).await;
    cast(&lobby, &mut session, &mut socket, peer, 108510, 10000, 3).await;
    let poisoned = hp(&lobby);
    lobby
        .monsters
        .as_ref()
        .unwrap()
        .tick(skills::now_ms() + 2100);
    assert!(
        hp(&lobby) < poisoned,
        "poison must change authoritative HP on its own timer"
    );
}

#[tokio::test]
async fn captured_multi_arrow_cleanup_and_cross_skill_combo_keep_all_paid_hits() {
    let Some((lobby, mut session, mut socket, _client, peer)) = setup().await else {
        return;
    };
    for id in [108515, 108555] {
        cast(&lobby, &mut session, &mut socket, peer, id, 10000, 2).await;
    }
    assert_eq!(session.skills.mana, 3920);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[0].count,
        92
    );
    // Capture repeats the central projectile index before incrementing side arrows.
    for (id, indices) in [
        (108515, vec![30i16, 30, 31]),
        (108555, vec![32, 32, 33, 34, 35]),
    ] {
        for index in indices {
            session.position.x += 1;
            session.pending_cast = None;
            let before = hp(&lobby);
            let mut impact = skills::effect(id, session.player_id);
            impact[8..10].copy_from_slice(&10000u16.to_le_bytes());
            impact[10..].fill(0);
            impact[16..18].copy_from_slice(&index.to_le_bytes());
            magic::handle(
                &mut socket,
                peer,
                &impact,
                &lobby,
                Some("admin"),
                Some(&mut session),
            )
            .await;
            assert!(
                hp(&lobby) < before,
                "every paid close-range arrow must deal damage"
            );
            let mut cleanup = impact;
            cleanup[1] = 4;
            cleanup[16..18].copy_from_slice(&(-101i16).to_le_bytes());
            cleanup[18..20].copy_from_slice(&index.to_le_bytes());
            magic::handle(
                &mut socket,
                peer,
                &cleanup,
                &lobby,
                Some("admin"),
                Some(&mut session),
            )
            .await;
        }
        let before = hp(&lobby);
        cast(&lobby, &mut session, &mut socket, peer, id, 10000, 3).await;
        assert_eq!(hp(&lobby), before, "cleanup cannot replenish paid impacts");
    }
    assert_eq!(session.skills.mana, 3920);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[0].count,
        92
    );
}

#[tokio::test]
async fn utility_attributes_healing_and_item_rollbacks() {
    let Some((lobby, mut session, mut socket, _client, peer)) = setup().await else {
        return;
    };
    session.skills.hp = 2000;
    skills::save(&lobby.db.lock().unwrap(), "admin", 0, &session.skills).unwrap();
    let player = session.player_id;
    cast(&lobby, &mut session, &mut socket, peer, 108705, player, 3).await;
    assert_eq!(session.skills.hp, 2060);
    cast(&lobby, &mut session, &mut socket, peer, 108710, player, 3).await;
    assert_eq!(session.combat.defence, 280);
    let mana = session.skills.mana;
    cast(&lobby, &mut session, &mut socket, peer, 108730, player, 3).await;
    assert_eq!(session.skills.mana, mana);
    assert_eq!(session.combat.defence, 280);
    cast(&lobby, &mut session, &mut socket, peer, 108030, player, 1).await;
    cast(&lobby, &mut session, &mut socket, peer, 108030, player, 3).await;
    assert_eq!(session.combat.attack, 157);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[1].count,
        1
    );
    cast(&lobby, &mut session, &mut socket, peer, 108802, player, 1).await;
    cast(&lobby, &mut session, &mut socket, peer, 108802, player, 3).await;
    assert_eq!(session.combat.resistance, [120; 6]);
    assert!(
        utility::training_command(
            &mut socket,
            peer,
            &lobby,
            "admin",
            &mut session,
            b"hurt 2400"
        )
        .await
    );
    assert_eq!(session.combat.resistance, [170; 6]);
    assert!(
        !utility::training_command(
            &mut socket,
            peer,
            &lobby,
            "other",
            &mut session,
            b"/hurt 2400"
        )
        .await
    );
    let mana = session.skills.mana;
    let wolf = &lobby.skills.as_ref().unwrap().spells[&108030];
    assert!(
        resources::commit(&lobby, "admin", 0, wolf, 1, |_| Err(
            "reject after item reservation"
        ))
        .is_err()
    );
    assert_eq!(
        skills::load(&lobby.db.lock().unwrap(), "admin", 0)
            .unwrap()
            .mana,
        mana
    );
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "admin", 0).unwrap()[1].count,
        1
    );
    cast(&lobby, &mut session, &mut socket, peer, 108710, player, 6).await;
    assert_eq!(session.combat.defence, 80);
    let changed = [(
        0,
        starter::Item {
            id: 391010000,
            durability: 1,
            count: 97,
        },
    )];
    let packet = inventory::count_packet(&changed);
    assert_eq!(packet.len(), 24);
    assert_eq!(&packet[9..13], &97u32.to_le_bytes());
}

#[tokio::test]
async fn quest_unlocks_are_persisted_scoped_and_required_before_payment() {
    let Some((lobby, mut session, mut socket, _client, peer)) = setup().await else {
        return;
    };
    session.combat.weapon_kind = 11;
    session.base_combat.weapon_kind = 11;
    let before = hp(&lobby);
    cast(&lobby, &mut session, &mut socket, peer, 108670, 10000, 3).await;
    assert_eq!(session.skills.mana, 4110);
    assert_eq!(hp(&lobby), before);
    {
        let db = lobby.db.lock().unwrap();
        db.execute("INSERT INTO character_quests VALUES ('admin',0,52,1)", [])
            .unwrap();
        assert!(!crate::quests::completed(&db, "admin", 0, 52).unwrap());
    }
    cast(&lobby, &mut session, &mut socket, peer, 108670, 10000, 3).await;
    assert_eq!(hp(&lobby), before);
    {
        let db = lobby.db.lock().unwrap();
        db.execute(
            "UPDATE character_quests SET state=2 WHERE account='admin' AND slot=0 AND quest=52",
            [],
        )
        .unwrap();
        assert!(crate::quests::completed(&db, "admin", 0, 52).unwrap());
        assert!(!crate::quests::completed(&db, "admin", 1, 52).unwrap());
        assert!(!crate::quests::completed(&db, "other", 0, 52).unwrap());
        assert_eq!(
            crate::quests::snapshot(&db, "admin", 0).unwrap(),
            [0x64, 1, 2, 0, 52, 0, 2, 0xf4, 1, 2]
        );
        assert_eq!(
            crate::quests::snapshot(&db, "admin", 1).unwrap(),
            [0x64, 1, 0, 0]
        );
    }
    cast(&lobby, &mut session, &mut socket, peer, 108670, 10000, 3).await;
    assert!(hp(&lobby) < before);
    assert_eq!(session.skills.mana, 4010);
    let mana = session.skills.mana;
    cast(&lobby, &mut session, &mut socket, peer, 108820, 10000, 1).await;
    cast(&lobby, &mut session, &mut socket, peer, 108820, 10000, 3).await;
    assert_eq!(
        session.skills.mana, mana,
        "NPC-immune armor cancellation must not charge resources"
    );
}

#[tokio::test]
async fn self_damage_requires_explicit_admin_fixture_permission() {
    let Some((lobby, mut session, mut socket, _client, peer)) = setup().await else {
        return;
    };
    lobby
        .db
        .lock()
        .unwrap()
        .execute("DELETE FROM character_privileges", [])
        .unwrap();
    assert!(
        !utility::training_command(
            &mut socket,
            peer,
            &lobby,
            "admin",
            &mut session,
            b"hurt 2000"
        )
        .await
    );
    assert_eq!(session.skills.hp, 2690);
    lobby
        .db
        .lock()
        .unwrap()
        .execute("INSERT INTO character_privileges VALUES ('admin',0,1)", [])
        .unwrap();
    assert!(
        utility::training_command(
            &mut socket,
            peer,
            &lobby,
            "admin",
            &mut session,
            b"hurt 2000"
        )
        .await
    );
    assert_eq!(session.skills.hp, 990);
    let mut ordinary = Session::new(crate::characters::Character {
        slot: 1,
        name: "NormalRogue".into(),
        ..session.character.clone()
    })
    .unwrap();
    assert!(
        !utility::training_command(
            &mut socket,
            peer,
            &lobby,
            "admin",
            &mut ordinary,
            b"hurt 2000"
        )
        .await
    );
}

#[tokio::test]
async fn targets_regenerate_clamp_and_respawn_after_death() {
    let Some((lobby, session, _socket, _client, _peer)) = setup().await else {
        return;
    };
    let world = lobby.monsters.as_ref().unwrap();
    let attack = |ap, extra| monsters::Attack {
        position: session.position,
        range: 1.0,
        attack: ap,
        percent: 100,
        extra,
        skill: true,
        drain_percent: 0,
        arrow: false,
        penetrate: false,
        magic: None,
    };
    world.hit(10000, attack(3000, 0), || Ok(())).unwrap();
    let before = hp(&lobby);
    world.tick(skills::now_ms() + 11000);
    assert_eq!(hp(&lobby), before + 1250);
    world.tick(skills::now_ms() + 22000);
    assert!(hp(&lobby) <= 25000);
    world.hit(10000, attack(1, 32767), || Ok(())).unwrap();
    assert_eq!(hp(&lobby), 0);
    let now = skills::now_ms();
    assert!(world.tick(now + 500).is_empty());
    let events = world.tick(now + 1100);
    assert_eq!(events.len(), 2);
    assert_eq!(&events[0].packet[..2], &[0x0a, 2]);
    assert_eq!(&events[1].packet[..2], &[0x0a, 1]);
    assert_eq!(hp(&lobby), 25000);
}
