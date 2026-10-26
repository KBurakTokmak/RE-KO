use crate::{combat, lobby::Lobby, magic, monsters, skills, starter, world_entry::Session};
use rusqlite::Connection;
use std::{path::Path, sync::Arc};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn mage_projectiles_areas_dots_staff_buffs_and_payment_are_authoritative() {
    let game = reko_common::game_dir();
    let rules = Path::new("../../logs/reference-coefficients.csv");
    if !game.join("Data/Skill_Magic_Main_us.tbl").exists() || !rules.exists() {
        return;
    }
    let mut lobby = Lobby::new(
        Connection::open_in_memory().unwrap(),
        crate::characters::CreationRules::load(&game).unwrap(),
        crate::world_entry::Bootstrap { next_level_exp: 50 },
    )
    .unwrap();
    lobby.skills = Some(skills::Catalog::load(&game).unwrap());
    lobby.combat = Some(combat::Catalog::load(&game, rules).unwrap());
    lobby.monsters = Some(Arc::new(monsters::World::load(&game).unwrap()));
    let character = crate::characters::Character {
        slot: 0,
        name: "MageTest".into(),
        race: 3,
        class: 110,
        face: 0,
        hair: 0,
        stats: [50, 60, 60, 70, 255],
    };
    {
        let db = lobby.db.lock().unwrap();
        crate::characters::ensure_account(&db, "mage").unwrap();
        crate::characters::set_nation(&db, "mage", 1).unwrap();
        db.execute(
            "INSERT INTO lobby_characters VALUES ('mage',0,'MageTest',3,110,0,0,?1)",
            [character.stats.as_slice()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO character_equipment VALUES ('mage',0,6,180110001,4000)",
            [],
        )
        .unwrap();
    }
    let mut session = Session::new(character).unwrap();
    session.entered = true;
    session.position.x = 3250;
    session.position.z = 4000;
    session.progress.level = 80;
    session.progress.points = [0, 0, 0, 0, 0, 80, 80, 80, 20];
    session.equipment = starter::equipment(&lobby.db.lock().unwrap(), "mage", 0).unwrap();
    crate::durability::refresh(
        &mut session,
        lobby.combat.as_ref().unwrap(),
        lobby.skills.as_ref(),
    );
    session.skills.set_limits(5000, 10000);
    skills::save(&lobby.db.lock().unwrap(), "mage", 0, &session.skills).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut sock, peer) = listener.accept().await.unwrap();
    let player = session.player_id;
    let packet = |id, target, phase| {
        let mut p = skills::effect(id, player);
        p[1] = phase;
        p[8..10].copy_from_slice(&u16::to_le_bytes(target));
        if target == u16::MAX {
            p[10..12].copy_from_slice(&326u16.to_le_bytes());
            p[14..16].copy_from_slice(&402u16.to_le_bytes());
        }
        p
    };
    let hp = |id| {
        let p = lobby.monsters.as_ref().unwrap().health(id, 21).unwrap();
        u32::from_le_bytes(p[8..12].try_into().unwrap())
    };
    for phase in [1, 2] {
        magic::handle(
            &mut sock,
            peer,
            &packet(110515, 10000, phase),
            &lobby,
            Some("mage"),
            Some(&mut session),
        )
        .await;
    }
    assert_eq!(session.skills.mana, 9950);
    assert_eq!(hp(10000), 25000);
    session.position.x = 5000;
    session.pending_cast = None;
    magic::handle(
        &mut sock,
        peer,
        &packet(110515, 10000, 3),
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    assert!(hp(10000) < 25000);
    assert_eq!(session.skills.mana, 9950);
    let after = hp(10000);
    magic::handle(
        &mut sock,
        peer,
        &packet(110515, 10000, 3),
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    assert_eq!(hp(10000), after);
    assert_eq!(session.equipment[6].durability, 4000);
    session.position.x = 3250;
    for phase in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(110545, u16::MAX, phase),
            &lobby,
            Some("mage"),
            Some(&mut session),
        )
        .await;
    }
    assert_eq!(session.skills.mana, 9750);
    assert!(hp(10001) < 25000 && hp(10002) < 25000);
    for phase in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(110509, 10000, phase),
            &lobby,
            Some("mage"),
            Some(&mut session),
        )
        .await;
    }
    let before_tick = hp(10000);
    lobby
        .monsters
        .as_ref()
        .unwrap()
        .tick(skills::now_ms() + 2001);
    assert!(hp(10000) < before_tick);
    magic::handle(
        &mut sock,
        peer,
        &packet(110542, 10000, 3),
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    assert!(session.equipment[6].durability < 4000);
    assert_eq!(
        starter::equipment(&lobby.db.lock().unwrap(), "mage", 0).unwrap(),
        session.equipment
    );
    for phase in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(110548, player, phase),
            &lobby,
            Some("mage"),
            Some(&mut session),
        )
        .await;
    }
    assert_eq!(
        session.combat.resistance[0],
        session.base_combat.resistance[0] + 80
    );
    magic::handle(
        &mut sock,
        peer,
        &packet(110820, player, 3),
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    assert!(session.skills.buffs.contains_key(&23));
    magic::handle(
        &mut sock,
        peer,
        &packet(110001, 10000, 2),
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    assert!(!session.skills.buffs.contains_key(&23));
    assert!(session.magic_projectiles.contains_key(&110001));
    let mana = session.skills.mana;
    for phase in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(110535, 10000, phase),
            &lobby,
            Some("mage"),
            Some(&mut session),
        )
        .await;
    }
    assert_eq!(
        session.skills.mana, mana,
        "missing reagent must roll back mana and cooldown"
    );
    assert!(!session.skills.cooldowns.contains_key(&110535));
    assert_eq!(
        skills::load(&lobby.db.lock().unwrap(), "mage", 0)
            .unwrap()
            .mana,
        mana
    );
    let mut invalid = packet(110533, u16::MAX, 1);
    invalid[10..12].copy_from_slice(&1000u16.to_le_bytes());
    magic::handle(
        &mut sock,
        peer,
        &invalid,
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    invalid[1] = 3;
    magic::handle(
        &mut sock,
        peer,
        &invalid,
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    assert_eq!(
        session.skills.mana, mana,
        "distant area must not charge mana"
    );
    for phase in [1, 2] {
        magic::handle(
            &mut sock,
            peer,
            &packet(110533, u16::MAX, phase),
            &lobby,
            Some("mage"),
            Some(&mut session),
        )
        .await;
    }
    let before = hp(10001);
    session.position.x = 5000;
    let mut impact = packet(110533, u16::MAX, 3);
    impact[10..12].copy_from_slice(&1000u16.to_le_bytes());
    magic::handle(
        &mut sock,
        peer,
        &impact,
        &lobby,
        Some("mage"),
        Some(&mut session),
    )
    .await;
    assert!(
        hp(10001) < before,
        "paid area projectile uses its original ground target after movement"
    );
    assert_eq!(session.skills.mana, mana - 150);

    // Native casts can precede a queued movement forecast and its stop correction.
    session.position.x = 3225;
    session.position.z = 3946;
    session.skills.mana = 1000;
    session.skills.hp = 1000;
    {
        let db = lobby.db.lock().unwrap();
        skills::save(&db, "mage", 0, &session.skills).unwrap();
        db.execute(
            "INSERT INTO character_inventory VALUES ('mage',0,20,389020000,1,2)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO character_inventory VALUES ('mage',0,21,389014000,1,1)",
            [],
        )
        .unwrap();
        session.inventory = starter::inventory(&db, "mage", 0).unwrap();
    }
    let mut world = Some(session);
    let mut account = Some("mage".to_string());
    for (id, expected_mana, expected_hp) in [(490020, 2920, 1000), (490014, 2920, 1720)] {
        super::handle_game_payload(
            &mut sock,
            peer,
            &packet(id, player, 3),
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        let s = world.as_ref().unwrap();
        assert_eq!((s.skills.mana, s.skills.hp), (expected_mana, expected_hp));
    }
    assert_eq!(world.as_ref().unwrap().inventory[20].count, 1);
    assert_eq!(world.as_ref().unwrap().inventory[21].id, 0);
    super::handle_game_payload(
        &mut sock,
        peer,
        &packet(110560, u16::MAX, 1),
        &lobby,
        &mut account,
        &mut world,
    )
    .await;
    for movement in [
        [0x06, 0xa4, 0x0c, 0xac, 0x0f, 0x86, 0, 0x2d, 0, 3],
        [0x06, 0x99, 0x0c, 0x6a, 0x0f, 0x86, 0, 0, 0, 0],
    ] {
        super::handle_game_payload(&mut sock, peer, &movement, &lobby, &mut account, &mut world)
            .await;
    }
    assert!(world.as_ref().unwrap().pending_cast.is_some());
    let before = hp(10001);
    super::handle_game_payload(
        &mut sock,
        peer,
        &packet(110560, u16::MAX, 3),
        &lobby,
        &mut account,
        &mut world,
    )
    .await;
    assert_eq!(world.as_ref().unwrap().skills.mana, 2520);
    assert!(hp(10001) < before);
    for phase in [1, 4, 3] {
        super::handle_game_payload(
            &mut sock,
            peer,
            &packet(110545, if phase == 4 { player } else { u16::MAX }, phase),
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
    }
    assert_eq!(
        world.as_ref().unwrap().skills.mana,
        2520,
        "explicit cast cancellation still blocks completion"
    );
}

#[tokio::test]
async fn fire_spear_and_ignition_overlap_in_either_impact_order_without_shared_damage_lock() {
    for spear_first in [true, false] {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let game = reko_common::game_dir();
        lobby.skills = Some(skills::Catalog::load(&game).unwrap());
        lobby.monsters = Some(Arc::new(monsters::World::load(&game).unwrap()));
        s.character.class = 110;
        s.progress.points = [0, 0, 0, 0, 0, 80, 80, 80, 20];
        s.skills.set_limits(5000, 10000);
        skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let player = s.player_id;
        let packet = |id, phase| {
            let mut p = skills::effect(id, player);
            p[1] = phase;
            p[8..10].copy_from_slice(&10000u16.to_le_bytes());
            p
        };
        let hp = || {
            let p = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
            u32::from_le_bytes(p[8..12].try_into().unwrap())
        };
        let spells = &lobby.skills.as_ref().unwrap().spells;
        assert!(spells[&110527].name.eq_ignore_ascii_case("Fire Spear"));
        assert_eq!(spells[&110518].name, "Ignition");
        let mana = 10000 - spells[&110527].mana - spells[&110518].mana;
        for phase in [1, 2] {
            magic::handle(
                &mut socket,
                peer,
                &packet(110527, phase),
                &lobby,
                Some("test"),
                Some(&mut s),
            )
            .await;
        }
        assert!(s.magic_projectiles.contains_key(&110527));
        let launch_mana = s.skills.mana;
        magic::handle(
            &mut socket,
            peer,
            &packet(110518, 1),
            &lobby,
            Some("test"),
            Some(&mut s),
        )
        .await;
        let initial_hp = hp();
        if spear_first {
            for phase in [3, 4] {
                magic::handle(
                    &mut socket,
                    peer,
                    &packet(110527, phase),
                    &lobby,
                    Some("test"),
                    Some(&mut s),
                )
                .await;
            }
            assert!(hp() < initial_hp);
            assert_eq!(
                s.skills.mana, launch_mana,
                "paid projectile impact cannot charge again"
            );
            assert!(
                s.pending_cast
                    .as_ref()
                    .is_some_and(|c| c.matches(110518, 10000, skills::now_ms())),
                "older impact/cleanup must preserve Ignition cast"
            );
        }
        let before = hp();
        magic::handle(
            &mut socket,
            peer,
            &packet(110518, 3),
            &lobby,
            Some("test"),
            Some(&mut s),
        )
        .await;
        assert!(
            hp() < before,
            "Ignition deals damage even with Fire Spear still travelling"
        );
        assert_eq!(s.skills.mana, mana);
        assert!(s.pending_cast.is_none());
        if !spear_first {
            assert!(
                s.magic_projectiles.contains_key(&110527),
                "Ignition must not consume the other projectile"
            );
            let before = hp();
            magic::handle(
                &mut socket,
                peer,
                &packet(110527, 3),
                &lobby,
                Some("test"),
                Some(&mut s),
            )
            .await;
            assert!(hp() < before);
            assert_eq!(s.skills.mana, mana);
        }
        assert!(!s.magic_projectiles.contains_key(&110527));
        let before = hp();
        for id in [110527, 110518] {
            magic::handle(
                &mut socket,
                peer,
                &packet(id, 3),
                &lobby,
                Some("test"),
                Some(&mut s),
            )
            .await;
        }
        assert_eq!(hp(), before, "duplicate impact packets cannot damage again");
        assert_eq!(s.skills.mana, mana);
        let persisted = skills::load(&lobby.db.lock().unwrap(), "test", 0).unwrap();
        assert_eq!(persisted.mana, mana);
        assert!(
            persisted.cooldowns.contains_key(&110527) && persisted.cooldowns.contains_key(&110518)
        );
    }
}
