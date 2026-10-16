use crate::{
    combat, lobby::Lobby, magic, monsters, skills, starter, utility, world_entry::Session,
};
use rusqlite::Connection;
use std::{path::Path, sync::Arc};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn priest_healing_buffs_cures_attacks_and_npc_curses() {
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
    let c = crate::characters::Character {
        slot: 0,
        name: "PriestTest".into(),
        race: 4,
        class: 112,
        face: 0,
        hair: 0,
        stats: [150, 150, 60, 200, 50],
    };
    {
        let db = lobby.db.lock().unwrap();
        crate::characters::ensure_account(&db, "priest").unwrap();
        db.execute(
            "INSERT INTO lobby_characters VALUES ('priest',0,'PriestTest',4,112,0,0,?1)",
            [c.stats.as_slice()],
        )
        .unwrap();
        db.execute("INSERT INTO character_equipment VALUES ('priest',0,6,190250001,5000),('priest',0,8,170150001,5000)",[]).unwrap();
        db.execute("INSERT INTO character_privileges VALUES ('priest',0,1)", [])
            .unwrap();
        db.execute("INSERT INTO character_inventory VALUES ('priest',0,0,379066000,1,1),('priest',0,1,379062000,1,5)",[]).unwrap();
        for q in [54, 518, 519, 520, 521, 522, 523] {
            db.execute("INSERT INTO character_quests VALUES ('priest',0,?1,2)", [q])
                .unwrap();
        }
    }
    let mut s = Session::new(c).unwrap();
    s.entered = true;
    s.position.x = 3250;
    s.position.z = 4000;
    s.progress.level = 80;
    s.progress.points = [0, 0, 0, 0, 0, 80, 80, 80, 20];
    s.equipment = starter::equipment(&lobby.db.lock().unwrap(), "priest", 0).unwrap();
    crate::durability::refresh(
        &mut s,
        lobby.combat.as_ref().unwrap(),
        lobby.skills.as_ref(),
    );
    s.skills.set_limits(s.combat.hp, s.combat.mp);
    skills::save(&lobby.db.lock().unwrap(), "priest", 0, &s.skills).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut sock, peer) = listener.accept().await.unwrap();
    let player = s.player_id;
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
    utility::training_command(&mut sock, peer, &lobby, "priest", &mut s, b"hurt 1000").await;
    let hp = s.skills.hp;
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112518, player, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert_eq!(s.skills.hp, hp + 360);
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112539, player, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert_eq!(s.skills.periodic[&112539].amount, 100);
    let hp = s.skills.hp;
    s.next_regen = u64::MAX;
    super::tick_character(
        &mut sock,
        peer,
        &lobby,
        "priest",
        &mut s,
        skills::now_ms() + 2001,
    )
    .await;
    assert_eq!(s.skills.hp, hp + 100);
    let maximum = s.skills.max_hp;
    let current = s.skills.hp;
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112657, player, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert_eq!(s.skills.max_hp, maximum + 1500);
    assert_eq!(s.skills.hp, current + 1500);
    magic::handle(
        &mut sock,
        peer,
        &packet(112657, player, 6),
        &lobby,
        Some("priest"),
        Some(&mut s),
    )
    .await;
    assert_eq!(s.skills.max_hp, maximum);
    let defence = s.combat.defence;
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112660, player, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert_eq!(s.combat.defence, defence + 300);
    let world = lobby.monsters.as_ref().unwrap();
    assert_eq!(world.target_attributes(10002).1, 500);
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112703, 10002, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert_eq!(world.target_attributes(10002).1, 375);
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112757, u16::MAX, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert_eq!(world.target_attributes(10001).1, 168);
    assert_eq!(world.target_attributes(10002).1, 350);
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112745, 10002, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert_eq!(world.target_attributes(10002).0, 20000);
    world.tick(skills::now_ms() + 150001);
    assert_eq!(world.target_attributes(10002), (25000, 500));
    magic::handle(
        &mut sock,
        peer,
        &packet(112802, 10000, 3),
        &lobby,
        Some("priest"),
        Some(&mut s),
    )
    .await;
    let bag = starter::inventory(&lobby.db.lock().unwrap(), "priest", 0).unwrap();
    assert_eq!((bag[0].id, bag[1].count), (379066000, 4));
    let health = || {
        let p = world.health(10000, 21).unwrap();
        u32::from_le_bytes(p[8..12].try_into().unwrap())
    };
    let hp = health();
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112002, 10000, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert!(health() < hp);
    s.skills.periodic.insert(
        999,
        skills::Periodic {
            amount: -25,
            next: skills::now_ms() + 2000,
            until: skills::now_ms() + 20000,
        },
    );
    skills::save(&lobby.db.lock().unwrap(), "priest", 0, &s.skills).unwrap();
    for stage in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(112535, player, stage),
            &lobby,
            Some("priest"),
            Some(&mut s),
        )
        .await;
    }
    assert!(!s.skills.periodic.contains_key(&999));
    assert!(s.skills.periodic.contains_key(&112539));
}
