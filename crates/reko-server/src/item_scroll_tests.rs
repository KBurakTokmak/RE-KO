use crate::{
    combat, lobby::Lobby, magic, skills, starter, transformation, utility, world_entry::Session,
};
use std::net::SocketAddr;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};

async fn setup() -> Option<(Lobby, Session, TcpStream, TcpStream, SocketAddr)> {
    let (mut lobby, mut s) = crate::world_service_tests::setup()?;
    lobby.skills = Some(skills::Catalog::load(&reko_common::game_dir()).unwrap());
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    s.next_regen = skills::now_ms() + 60000;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, peer) = listener.accept().await.unwrap();
    Some((lobby, s, server, client, peer))
}

fn give(lobby: &Lobby, s: &mut Session, item: u32, count: u16) {
    let charged = lobby
        .combat
        .as_ref()
        .unwrap()
        .item(item)
        .is_some_and(|(base, _)| combat::integer(base, 10) == 255);
    let db = lobby.db.lock().unwrap();
    db.execute(
        "DELETE FROM character_inventory WHERE account='test' AND slot=0",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO character_inventory VALUES('test',0,0,?1,?2,?3)",
        rusqlite::params![
            item,
            if charged { count } else { 1 },
            if charged { 1 } else { count }
        ],
    )
    .unwrap();
    s.inventory = starter::inventory(&db, "test", 0).unwrap();
}

fn uses(lobby: &Lobby, s: &Session) -> u16 {
    let item = s.inventory[0];
    if lobby
        .combat
        .as_ref()
        .unwrap()
        .item(item.id)
        .is_some_and(|(base, _)| combat::integer(base, 10) == 255)
    {
        item.durability
    } else {
        item.count
    }
}

async fn drain(client: &mut TcpStream) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    loop {
        let mut h = [0; 4];
        if tokio::time::timeout(
            std::time::Duration::from_millis(30),
            client.read_exact(&mut h),
        )
        .await
        .is_err()
        {
            break;
        }
        assert_eq!(&h[..2], &[0xaa, 0x55]);
        let len = usize::from(u16::from_le_bytes([h[2], h[3]]));
        let mut p = vec![0; len + 2];
        tokio::time::timeout(std::time::Duration::from_secs(2), client.read_exact(&mut p))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&p[len..], &[0x55, 0xaa]);
        result.push(p[..len].to_vec());
    }
    result
}

async fn cast(
    lobby: &Lobby,
    s: &mut Session,
    sock: &mut TcpStream,
    client: &mut TcpStream,
    peer: SocketAddr,
    id: u32,
    phase: u8,
) -> Vec<Vec<u8>> {
    let mut p = skills::effect(id, s.player_id);
    p[1] = phase;
    assert!(magic::handle(sock, peer, &p, lobby, Some("test"), Some(s)).await);
    drain(client).await
}

fn failed(packets: &[Vec<u8>]) -> bool {
    packets.iter().any(|p| p.starts_with(&[0x31, 4]))
}

#[tokio::test]
async fn zone_loading_restores_pus_buffs_with_remaining_time_without_recharging_or_consuming() {
    let Some((lobby, mut s, mut sock, mut client, peer)) = setup().await else {
        return;
    };
    for id in [500035, 500033, 500030] {
        let item = lobby.skills.as_ref().unwrap().spells[&id].use_item;
        give(&lobby, &mut s, item, 3);
        assert!(!failed(
            &cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await
        ));
    }
    let now = skills::now_ms();
    s.skills.buffs.get_mut(&6).unwrap().until = now + 25 * 60 * 1000;
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    let before = s.skills.buffs.clone();
    let cooldowns = s.skills.cooldowns.clone();
    let inventory = s.inventory;
    let limits = (
        s.skills.hp,
        s.skills.max_hp,
        s.skills.mana,
        s.skills.max_mana,
    );
    let position = crate::movement::Position {
        zone: 1,
        x: 3540,
        z: 16100,
        y: 0,
    };
    crate::movement::save(&lobby.db.lock().unwrap(), "test", 0, position).unwrap();
    let begin = crate::travel::begin(&lobby, &mut s, position);
    assert_eq!(&begin[..4], &[0x27, 3, 1, 0]);
    crate::travel::handle(&mut sock, peer, &lobby, "test", &mut s, &[0x27, 1]).await;
    assert!(
        !drain(&mut client)
            .await
            .iter()
            .any(|p| p.starts_with(&[0x31, 3]))
    );
    crate::travel::handle(&mut sock, peer, &lobby, "test", &mut s, &[0x27, 2]).await;
    let packets = drain(&mut client).await;
    assert!(s.entered && !s.zone_loading);
    for id in [500035, 500033, 500030] {
        assert_eq!(
            packets
                .iter()
                .filter(|p| p.len() == 22
                    && p[..2] == [0x31, 3]
                    && u32::from_le_bytes(p[2..6].try_into().unwrap()) == id)
                .count(),
            1,
            "restore {id} once at load completion"
        );
    }
    let speed = packets
        .iter()
        .find(|p| {
            p.len() == 22
                && p[..2] == [0x31, 3]
                && u32::from_le_bytes(p[2..6].try_into().unwrap()) == 500035
        })
        .unwrap();
    assert!((1498..=1500).contains(&u16::from_le_bytes(speed[16..18].try_into().unwrap())));
    assert!(s.skills.speed(lobby.skills.as_ref(), skills::now_ms()) > 45);
    assert_eq!(s.inventory, inventory);
    assert_eq!(s.skills.cooldowns, cooldowns);
    assert_eq!(
        (
            s.skills.hp,
            s.skills.max_hp,
            s.skills.mana,
            s.skills.max_mana
        ),
        limits
    );
    let saved = skills::load(&lobby.db.lock().unwrap(), "test", 0).unwrap();
    for (category, buff) in before {
        assert_eq!(
            (saved.buffs[&category].skill, saved.buffs[&category].until),
            (buff.skill, buff.until)
        );
    }
}

#[tokio::test]
async fn pus_hp_defence_swift_stat_and_attack_scrolls_use_owned_items_for_every_class() {
    let Some((lobby, mut s, mut sock, mut client, peer)) = setup().await else {
        return;
    };
    for class in [106, 108, 110, 112, 206, 208, 210, 212] {
        s.character.class = class;
        s.character.race = if class < 200 { 2 } else { 12 };
        for id in [
            500029, 500030, 500031, 500032, 500033, 500034, 500035, 500040, 500041, 500042, 500046,
            500047, 500048, 500053, 500054, 500055, 500056, 500057, 500061, 500063, 500064,
        ] {
            s.skills = skills::State::default();
            s.skills.set_limits(s.base_combat.hp, s.base_combat.mp);
            skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
            let catalog = lobby.skills.as_ref().unwrap();
            let spell = &catalog.spells[&id];
            let row = &catalog.effects[&(4, id)];
            let kind = combat::integer(row, 1) as u8;
            let duration = combat::integer(row, 3) as u64 * 1000;
            let item = spell.use_item;
            give(&lobby, &mut s, item, 2);
            utility::refresh(&mut s, Some(catalog), lobby.combat.as_ref());
            let base = s.combat;
            let packets = cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await;
            assert!(!failed(&packets), "class {class}, scroll {id}: {packets:?}");
            assert!(packets.iter().any(|p| p.starts_with(&[0x31, 3])));
            assert_eq!(uses(&lobby, &s), 1);
            let buff = &s.skills.buffs[&kind];
            assert!(
                (duration - 5000..=duration).contains(&buff.until.saturating_sub(skills::now_ms()))
            );
            match id {
                500029 | 500046 => assert_eq!(s.combat.defence, base.defence + 150),
                500030 => assert_eq!(s.combat.defence, base.defence + 300),
                500055 => assert_eq!(s.combat.defence, base.defence + 350),
                500056 => assert_eq!(s.combat.defence, base.defence + 400),
                500031 | 500047 => assert_eq!(s.combat.hp, base.hp + 500),
                500032 | 500048 => assert_eq!(s.combat.hp, base.hp + 1000),
                500033 | 500061 => assert_eq!(s.combat.hp, base.hp + 1500),
                500053 => assert_eq!(s.combat.hp, base.hp + 2000),
                500054 => assert_eq!(u32::from(s.combat.hp), u32::from(base.hp) * 160 / 100),
                500035 | 500057 | 500063 | 500064 => {
                    assert_eq!(s.skills.speed(Some(catalog), skills::now_ms()), 68)
                }
                500034 => assert_eq!(
                    u32::from(s.combat.attack),
                    u32::from(base.attack) * 120 / 100
                ),
                _ => assert!(
                    s.combat.bonus_stats.iter().sum::<u16>() > base.bonus_stats.iter().sum()
                ),
            }
            let stored = skills::load(&lobby.db.lock().unwrap(), "test", 0).unwrap();
            assert_eq!(stored.buffs[&kind].skill, id);
            assert_eq!(stored.max_hp, s.skills.max_hp);
            let replay = cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await;
            assert!(failed(&replay));
            assert_eq!(uses(&lobby, &s), 1);
            cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 6).await;
            assert!(!s.skills.buffs.contains_key(&kind));
        }
    }
}

#[tokio::test]
async fn scroll_failures_preserve_inventory_state_and_wallet_and_expiry_restores_attributes() {
    let Some((mut lobby, mut s, mut sock, mut client, peer)) = setup().await else {
        return;
    };
    give(&lobby, &mut s, 800013000, 2);
    let coins = s.coins;
    let hp = s.skills.hp;
    let wrong = cast(&lobby, &mut s, &mut sock, &mut client, peer, 500030, 3).await;
    assert!(failed(&wrong));
    assert_eq!(uses(&lobby, &s), 2);
    s.skills.hp = 0;
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 3).await
    ));
    s.skills.hp = hp;
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    // An otherwise valid skill cannot claim a different item's resources.
    lobby
        .skills
        .as_mut()
        .unwrap()
        .spells
        .get_mut(&500030)
        .unwrap()
        .use_item = 800013000;
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500030, 3).await
    ));
    let mut target = skills::effect(500033, s.player_id);
    target[8..10].copy_from_slice(&(s.player_id + 1).to_le_bytes());
    magic::handle(&mut sock, peer, &target, &lobby, Some("test"), Some(&mut s)).await;
    assert!(failed(&drain(&mut client).await));
    lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_scroll BEFORE INSERT ON character_skill_state BEGIN SELECT RAISE(ABORT,'rollback');END;").unwrap();
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 3).await
    ));
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[0].durability,
        2
    );
    assert!(s.skills.buffs.is_empty());
    assert_eq!(s.coins, coins);
    lobby
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_scroll;")
        .unwrap();
    assert!(!failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 3).await
    ));
    let deadline = s.skills.buffs[&1].until;
    crate::tick_character(&mut sock, peer, &lobby, "test", &mut s, deadline + 1).await;
    drain(&mut client).await;
    assert!(!s.skills.buffs.contains_key(&1));
    assert_eq!(s.combat.hp, s.base_combat.hp);
    assert_eq!(
        skills::load(&lobby.db.lock().unwrap(), "test", 0)
            .unwrap()
            .max_hp,
        s.base_combat.hp
    );
    assert_eq!(uses(&lobby, &s), 1);
    give(&lobby, &mut s, 800078000, 1);
    s.progress.level = 30;
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500053, 3).await
    ));
    assert_eq!(uses(&lobby, &s), 1);
}

#[tokio::test]
async fn direct_npc_scrolls_keep_nation_buff_and_zone_rules_and_persist_without_a_list() {
    let Some((lobby, mut s, mut sock, mut client, peer)) = setup().await else {
        return;
    };
    for id in 500505..=500512 {
        s.skills = skills::State::default();
        s.skills.set_limits(s.base_combat.hp, s.base_combat.mp);
        s.transformation_visual = 1;
        let item = lobby.skills.as_ref().unwrap().spells[&id].use_item;
        s.character.class = if id <= 500508 { 208 } else { 108 };
        s.character.race = if id <= 500508 { 12 } else { 2 };
        give(&lobby, &mut s, item, 2);
        skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
        s.position.zone = 71;
        let result = cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await;
        assert!(!failed(&result), "NPC {id}: {result:?}");
        assert_eq!(transformation::appearance(&s), id);
        assert_eq!(uses(&lobby, &s), 1);
        assert!(result.iter().any(|p| p.first() == Some(&0x29) && p[3] == 3));
        assert!(
            (3595000..=3600000).contains(
                &s.skills.buffs[&transformation::CATEGORY]
                    .until
                    .saturating_sub(skills::now_ms())
            )
        );
        assert!(failed(
            &cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await
        ));
        assert_eq!(uses(&lobby, &s), 1);
        s.skills = skills::load(&lobby.db.lock().unwrap(), "test", 0).unwrap();
        assert_eq!(transformation::appearance(&s), id);
        // NPC disguises, unlike monster disguises, allow ordinary buffs.
        give(&lobby, &mut s, 800015000, 2);
        assert!(!failed(
            &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500035, 3).await
        ));
        cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 6).await;
        assert_eq!(transformation::appearance(&s), 1);
        assert!(s.skills.buffs.contains_key(&6));
        give(&lobby, &mut s, item, 2);
        assert!(!failed(
            &cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await
        ));
        let until = s.skills.buffs[&transformation::CATEGORY].until;
        crate::tick_character(&mut sock, peer, &lobby, "test", &mut s, until + 1).await;
        transformation::refresh(&mut sock, peer, &lobby, "test", &mut s).await;
        drain(&mut client).await;
        assert_eq!(transformation::appearance(&s), 1);
        assert_eq!(uses(&lobby, &s), 1);
        s.character.class = if id <= 500508 { 108 } else { 208 };
        s.character.race = if id <= 500508 { 2 } else { 12 };
        assert!(failed(
            &cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await
        ));
        assert_eq!(uses(&lobby, &s), 1);
    }
}

#[tokio::test]
async fn scrolls_combine_and_direct_transformation_failures_never_charge() {
    let Some((lobby, mut s, mut sock, mut client, peer)) = setup().await else {
        return;
    };
    let base = s.combat;
    for (id, item) in [
        (500033, 800013000),
        (500030, 800010000),
        (500035, 800015000),
    ] {
        give(&lobby, &mut s, item, 2);
        assert!(!failed(
            &cast(&lobby, &mut s, &mut sock, &mut client, peer, id, 3).await
        ));
        assert_eq!(uses(&lobby, &s), 1);
    }
    assert_eq!(s.combat.hp, base.hp + 1500);
    assert_eq!(s.combat.defence, base.defence + 300);
    assert_eq!(s.skills.speed(lobby.skills.as_ref(), skills::now_ms()), 68);
    give(&lobby, &mut s, 800190000, 2);
    let before_hp = s.skills.hp;
    lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_npc BEFORE INSERT ON character_skill_state BEGIN SELECT RAISE(ABORT,'rollback'); END;").unwrap();
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500509, 3).await
    ));
    assert_eq!(s.skills.hp, before_hp);
    assert_eq!(transformation::appearance(&s), 1);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[0].durability,
        2
    );
    lobby
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_npc;")
        .unwrap();
    assert!(!failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500509, 3).await
    ));
    assert_eq!(uses(&lobby, &s), 1);
    assert_eq!(s.combat.hp, base.hp + 1500);
    assert_eq!(s.combat.defence, base.defence + 300);
    cast(&lobby, &mut s, &mut sock, &mut client, peer, 500509, 6).await;
    give(&lobby, &mut s, 381001000, 2);
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500509, 3).await
    ));
    assert_eq!(uses(&lobby, &s), 2);
    // Monster forms retain their existing restriction on cash-shop buffs.
    s.skills.buffs.insert(
        transformation::CATEGORY,
        skills::Buff {
            skill: 472010,
            until: skills::now_ms() + 60000,
        },
    );
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    give(&lobby, &mut s, 800003000, 2);
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500023, 3).await
    ));
    assert_eq!(uses(&lobby, &s), 2);
}

#[test]
fn original_npc_forms_match_primary_1453_model_duration_class_and_nation() {
    let reference = std::path::Path::new("../../logs/reference1453/MAGIC_TYPE6_NPC.json");
    if !reference.exists()
        || !reko_common::game_dir()
            .join("Data/Skill_Magic_6.tbl")
            .exists()
    {
        return;
    }
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
    assert_eq!(rows.len(), 8);
    let catalog = skills::Catalog::load(&reko_common::game_dir()).unwrap();
    for row in rows {
        let id = row["iNum"].as_u64().unwrap() as u32;
        assert!(transformation::npc_form(id));
        let native = &catalog.effects[&(6, id)];
        for (field, column) in [
            ("Size", 3),
            ("TransformID", 4),
            ("Duration", 5),
            ("Class", 21),
            ("UserSkillUse", 22),
            ("Nation", 28),
        ] {
            assert_eq!(
                i64::from(combat::integer(native, column)),
                row[field].as_i64().unwrap(),
                "{id} {field}"
            );
        }
        assert_eq!(row["MonsterFriendly"], 1);
        for field in [
            "MaxHp",
            "MaxMp",
            "Speed",
            "AttackSpeed",
            "TotalHit",
            "TotalAc",
            "TotalHitRate",
            "TotalEvasionRate",
            "TotalFireR",
            "TotalColdR",
            "TotalLightningR",
            "TotalMagicR",
            "TotalDiseaseR",
            "TotalPoisonR",
        ] {
            assert_eq!(row[field], 0);
        }
    }
}

#[tokio::test]
async fn purchased_thirty_use_pack_updates_native_counter_until_only_the_last_charge_removes_it() {
    let Some((mut lobby, mut s, mut sock, mut client, peer)) = setup().await else {
        return;
    };
    {
        let db = lobby.db.lock().unwrap();
        db.execute("INSERT INTO world_merchants VALUES(11000,27011,2040,'Test Sundries',255000,3,50,22,?1,?2,?3,?4)",rusqlite::params![s.position.zone,s.position.x,s.position.z,s.position.y]).unwrap();
        lobby.merchants = crate::shops::load(&db, &reko_common::game_dir()).unwrap();
    }
    s.merchant = Some(11000);
    let mut purchase = vec![0x21, 1];
    purchase.extend(255000u32.to_le_bytes());
    purchase.extend(11000u16.to_le_bytes());
    purchase.extend(800013000u32.to_le_bytes());
    purchase.push(0);
    purchase.extend(1u16.to_le_bytes());
    crate::shops::handle(&mut sock, peer, &lobby, "test", &mut s, &purchase).await;
    drain(&mut client).await;
    assert_eq!(
        (
            s.inventory[0].id,
            s.inventory[0].count,
            s.inventory[0].durability
        ),
        (800013000, 1, 30)
    );
    for remaining in (0..30u16).rev() {
        let packets = cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 3).await;
        assert!(!failed(&packets));
        let update = packets.iter().find(|p| p[0] == 0x3d).unwrap();
        assert_eq!(
            u16::from_le_bytes(update[14..16].try_into().unwrap()),
            remaining
        );
        assert_eq!(
            u32::from_le_bytes(update[9..13].try_into().unwrap()),
            u32::from(remaining > 0)
        );
        assert_eq!(s.inventory[0].durability, remaining);
        assert_eq!(s.inventory[0].count, u16::from(remaining > 0));
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            s.inventory
        );
        if remaining == 29 {
            let coins = s.coins;
            let mut repair = vec![0x3b, 2, 0];
            repair.extend(800013000u32.to_le_bytes());
            crate::shops::handle(&mut sock, peer, &lobby, "test", &mut s, &repair).await;
            drain(&mut client).await;
            assert_eq!(s.inventory[0].durability, 29);
            assert_eq!(s.coins, coins);
            let rejected = cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 3).await;
            assert!(failed(&rejected));
            assert_eq!(s.inventory[0].durability, 29);
            // A failed save must not spend a second charge either.
            cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 6).await;
            lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_charge BEFORE INSERT ON character_skill_state BEGIN SELECT RAISE(ABORT,'rollback');END;").unwrap();
            assert!(failed(
                &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 3).await
            ));
            assert_eq!(
                starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[0].durability,
                29
            );
            lobby
                .db
                .lock()
                .unwrap()
                .execute_batch("DROP TRIGGER fail_charge;")
                .unwrap();
        } else {
            cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 6).await;
        }
    }
    assert_eq!(s.inventory[0].id, 0);
    assert!(failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500033, 3).await
    ));
    give(&lobby, &mut s, 800015000, 30);
    assert!(!failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500035, 3).await
    ));
    assert_eq!((s.inventory[0].count, s.inventory[0].durability), (1, 29));
    give(&lobby, &mut s, 800190000, 20);
    assert!(!failed(
        &cast(&lobby, &mut s, &mut sock, &mut client, peer, 500509, 3).await
    ));
    assert_eq!((s.inventory[0].count, s.inventory[0].durability), (1, 19));
}
