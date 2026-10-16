use crate::{
    admin, combat, lobby::Lobby, magic, monsters, skills, starter, utility, world_entry::Session,
};
use rusqlite::Connection;
use std::{path::Path, sync::Arc};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn warrior_combat_recovery_buffs_and_shared_admin_authorization() {
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
        name: "WarriorFixture".into(),
        race: 1,
        class: 106,
        face: 0,
        hair: 0,
        stats: [150, 150, 60, 50, 50],
    };
    {
        let db = lobby.db.lock().unwrap();
        crate::characters::ensure_account(&db, "testing").unwrap();
        crate::characters::set_nation(&db, "testing", 1).unwrap();
        db.execute(
            "INSERT INTO lobby_characters VALUES ('testing',0,'WarriorFixture',1,106,0,0,?1)",
            [character.stats.as_slice()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO character_equipment VALUES ('testing',0,6,120150001,5000)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO character_inventory VALUES ('testing',0,0,379063000,1,1),('testing',0,1,379059000,1,5)",[]).unwrap();
        for quest in [51, 510, 511] {
            db.execute(
                "INSERT INTO character_quests VALUES ('testing',0,?1,2)",
                [quest],
            )
            .unwrap();
        }
    }
    let mut s = Session::new(character).unwrap();
    s.entered = true;
    s.position.x = 3250;
    s.position.z = 4000;
    s.progress.level = 80;
    s.progress.points = [0, 0, 0, 0, 0, 80, 80, 80, 20];
    s.equipment = starter::equipment(&lobby.db.lock().unwrap(), "testing", 0).unwrap();
    crate::durability::refresh(
        &mut s,
        lobby.combat.as_ref().unwrap(),
        lobby.skills.as_ref(),
    );
    s.skills.set_limits(s.combat.hp, s.combat.mp);
    skills::save(&lobby.db.lock().unwrap(), "testing", 0, &s.skills).unwrap();
    assert_eq!(s.combat.resistance, [45; 6]);
    assert_eq!(
        s.combat.defence,
        u32::from(s.base_combat.defence).saturating_mul(120) as u16 / 100
    );
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
    assert!(
        !utility::training_command(&mut sock, peer, &lobby, "testing", &mut s, b"hurt 1000").await
    );
    lobby
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO character_privileges VALUES ('testing',0,1)",
            [],
        )
        .unwrap();
    assert!(admin::is_admin(&lobby.db.lock().unwrap(), "testing", 0).unwrap());
    assert!(!admin::is_admin(&lobby.db.lock().unwrap(), "testing", 1).unwrap());
    let hp = s.skills.hp;
    assert!(
        utility::training_command(&mut sock, peer, &lobby, "testing", &mut s, b"hurt 1000").await
    );
    assert_eq!(s.skills.hp, hp - 850);
    let hp = s.skills.hp;
    magic::handle(
        &mut sock,
        peer,
        &packet(106710, player, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    assert_eq!(s.skills.hp, hp - 100);
    magic::handle(
        &mut sock,
        peer,
        &packet(106730, player, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    assert_eq!(s.skills.periodic[&106730].amount, 25);
    let hp = s.skills.hp;
    s.next_regen = u64::MAX;
    super::tick_character(
        &mut sock,
        peer,
        &lobby,
        "testing",
        &mut s,
        skills::now_ms() + 2001,
    )
    .await;
    assert_eq!(s.skills.hp, hp + 25);
    let ap = s.combat.attack;
    magic::handle(
        &mut sock,
        peer,
        &packet(106705, player, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    assert!(s.combat.attack > ap);
    assert_eq!(s.combat.bonus_stats[0], 15);
    magic::handle(
        &mut sock,
        peer,
        &packet(106705, player, 6),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    assert_eq!(s.combat.bonus_stats, [0; 5]);
    assert!(
        s.skills.mana >= 300,
        "mana {} hp {}",
        s.skills.mana,
        s.skills.hp
    );
    magic::handle(
        &mut sock,
        peer,
        &packet(106802, 10000, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    let bag = starter::inventory(&lobby.db.lock().unwrap(), "testing", 0).unwrap();
    assert_eq!((bag[0].id, bag[0].count, bag[1].count), (379063000, 1, 4));
    magic::handle(
        &mut sock,
        peer,
        &packet(106575, 10000, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    let health = || {
        let p = lobby.monsters.as_ref().unwrap().health(10000, 21).unwrap();
        u32::from_le_bytes(p[8..12].try_into().unwrap())
    };
    let hp = health();
    lobby
        .monsters
        .as_ref()
        .unwrap()
        .tick(skills::now_ms() + 2001);
    assert_eq!(health(), hp - 100);
    for phase in [1, 3] {
        magic::handle(
            &mut sock,
            peer,
            &packet(106760, u16::MAX, phase),
            &lobby,
            Some("testing"),
            Some(&mut s),
        )
        .await;
    }
    let p = lobby.monsters.as_ref().unwrap().health(10001, 21).unwrap();
    assert!(u32::from_le_bytes(p[8..12].try_into().unwrap()) < 25000);
    let ac = s.combat.defence;
    magic::handle(
        &mut sock,
        peer,
        &packet(106675, player, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    assert!(s.combat.defence >= ac * 3);
    assert_eq!(s.skills.speed(lobby.skills.as_ref(), skills::now_ms()), 23);
    magic::handle(
        &mut sock,
        peer,
        &packet(106720, player, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    let before = (s.skills.hp, s.skills.mana);
    utility::training_command(&mut sock, peer, &lobby, "testing", &mut s, b"hurt 100").await;
    assert_eq!(s.skills.hp, before.0 - 76);
    assert_eq!(s.skills.mana, before.1 - 10);
    s.skills.buffs.remove(&31);
    s.skills.periodic.clear();
    s.skills.hp = 1;
    s.skills.mana = s.skills.max_mana;
    s.sitting = false;
    {
        let db = lobby.db.lock().unwrap();
        skills::save(&db, "testing", 0, &s.skills).unwrap();
        db.execute(
            "INSERT INTO character_inventory VALUES ('testing',0,20,389014000,1,2)",
            [],
        )
        .unwrap();
    }
    magic::handle(
        &mut sock,
        peer,
        &packet(490014, player, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    assert_eq!(s.skills.hp, 721);
    magic::handle(
        &mut sock,
        peer,
        &packet(106780, player, 3),
        &lobby,
        Some("testing"),
        Some(&mut s),
    )
    .await;
    assert_eq!(s.skills.hp, 826);
    assert_eq!(s.skills.periodic[&106780].amount, 296);
    super::tick_character(
        &mut sock,
        peer,
        &lobby,
        "testing",
        &mut s,
        skills::now_ms() + 2001,
    )
    .await;
    assert_eq!(
        s.skills.hp, 1122,
        "HP Booster must recover HP while standing"
    );
    lobby
        .db
        .lock()
        .unwrap()
        .execute("UPDATE character_privileges SET is_admin=0", [])
        .unwrap();
    assert!(
        !utility::training_command(&mut sock, peer, &lobby, "testing", &mut s, b"hurt 100").await
    );
    s.skills.hp = s.skills.max_hp;
    s.skills.mana = s.skills.max_mana;
    let previous = (s.skills.hp, s.skills.mana);
    lobby
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO character_stat_points VALUES ('testing',0,1)",
            [],
        )
        .unwrap();
    crate::stat_points::handle(&mut sock, peer, &lobby, "testing", &mut s, &[0x28, 2, 1, 0]).await;
    assert!(s.skills.max_hp > previous.0);
    assert_eq!(
        (s.skills.hp, s.skills.mana),
        previous,
        "allocating stamina must not silently fill the new capacity"
    );
}
