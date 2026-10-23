use super::*;
use rusqlite::{Connection, OpenFlags, params};
use tokio::io::AsyncReadExt;

fn setup() -> Option<(Lobby, Session)> {
    let source = std::path::Path::new("../../reko.db");
    if !source.exists() {
        return None;
    }
    let (mut lobby, s) = crate::world_service_tests::setup()?;
    let reference = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let db = lobby.db.lock().unwrap();
    for (table, filter) in [
        ("world_spawns", "zone=21 AND prototype IN(603,16085,27011)"),
        (
            "world_spawn_conditions",
            "entity_id IN(SELECT id FROM world_spawns WHERE zone=21 AND prototype IN(603,16085,27011))",
        ),
        (
            "world_reference_rules",
            "(table_name='QUEST_HELPER' AND rule_id IN(4255,27011)) OR (table_name='ITEM_EXCHANGE' AND CAST(json_extract(data,'$.nOriginItemNum1') AS INTEGER) BETWEEN 389160000 AND 389166000 AND CAST(json_extract(data,'$.bRandomFlag') AS INTEGER)=101)",
        ),
        (
            "world_upgrade_item_types",
            "item_id IN (SELECT CAST(json_extract(data,'$.nExchangeItemNum1') AS INTEGER)/1000*1000 FROM world_reference_rules WHERE table_name='ITEM_EXCHANGE' AND CAST(json_extract(data,'$.nOriginItemNum1') AS INTEGER) BETWEEN 389160000 AND 389166000)",
        ),
    ] {
        let mut q = reference
            .prepare(&format!("SELECT * FROM {table} WHERE {filter}"))
            .unwrap();
        let width = q.column_count();
        let rows = q
            .query_map([], |r| {
                (0..width)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap();
        for row in rows {
            db.execute(
                &format!("INSERT INTO {table} VALUES({})", vec!["?"; width].join(",")),
                rusqlite::params_from_iter(row.unwrap()),
            )
            .unwrap();
        }
    }
    lobby.merchants = crate::shops::load_population(&db, &reko_common::game_dir()).unwrap();
    assert_eq!(lobby.merchants.len(), 3);
    drop(db);
    Some((lobby, s))
}
fn item(lobby: &Lobby, s: &mut Session, id: u32, count: u16) {
    let db = lobby.db.lock().unwrap();
    db.execute(
        "DELETE FROM character_inventory WHERE account='test' AND slot=0",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO character_inventory VALUES('test',0,0,?1,1,?2)",
        params![id, count],
    )
    .unwrap();
    s.inventory = starter::inventory(&db, "test", 0).unwrap();
}
fn request(s: &Session, id: u32) -> Vec<u8> {
    let mut p = vec![0x5b, 5];
    p.extend(s.merchant.unwrap().to_le_bytes());
    p.extend(id.to_le_bytes());
    p.push(0);
    p
}

#[tokio::test]
async fn captured_fragment_request_uses_its_selected_slot_and_rejects_malformed_or_forged_slots() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let npc = at(&lobby, &mut s, 603);
    assert_eq!(npc, 12228);
    item(&lobby, &mut s, 389160000, 2);
    {
        let db = lobby.db.lock().unwrap();
        db.execute("UPDATE character_inventory SET inventory_slot=5 WHERE account='test' AND slot=0 AND inventory_slot=0",[]).unwrap();
        db.execute(
            "INSERT INTO character_inventory VALUES('test',0,0,389160000,1,3)",
            [],
        )
        .unwrap();
        s.inventory = starter::inventory(&db, "test", 0).unwrap();
    }
    // Unmodified1453 capture: NPC12228, Fragment of Arrogance, bag slot5.
    let captured = [0x5b, 0x05, 0xc4, 0x2f, 0x40, 0x1c, 0x32, 0x17, 0x05];
    let (mut sock, mut client, peer) = socket().await;
    crate::upgrading::handle(&mut sock, peer, &lobby, "test", &mut s, &captured).await;
    assert_eq!(read(&mut client).await[0], 0x3d);
    let reply = read(&mut client).await;
    assert_eq!(&reply[..3], &[0x5b, 5, 1]);
    assert_eq!(reply[12], 5);
    assert_eq!(read(&mut client).await[..2], [0x33, 9]);
    assert_eq!(read(&mut client).await[0], 0x54);
    assert_eq!(s.inventory[0].count, 3);
    assert_eq!((s.inventory[5].id, s.inventory[5].count), (389160000, 1));
    let before = s.inventory;
    let mut extended = captured.to_vec();
    extended.push(0);
    let mut bad_slot = captured;
    bad_slot[8] = 28;
    let mut empty_slot = captured;
    empty_slot[8] = 4;
    let mut wrong_item = captured;
    wrong_item[4..8].copy_from_slice(&389161000u32.to_le_bytes());
    for invalid in [
        captured[..8].to_vec(),
        extended,
        bad_slot.to_vec(),
        empty_slot.to_vec(),
        wrong_item.to_vec(),
    ] {
        crate::upgrading::handle(&mut sock, peer, &lobby, "test", &mut s, &invalid).await;
        let reply = read(&mut client).await;
        assert_eq!(&reply[..3], &[0x5b, 5, 0]);
        assert_eq!(read(&mut client).await[0], 0x10);
        assert_eq!(s.inventory, before);
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            before
        );
    }
}
fn at(lobby: &Lobby, s: &mut Session, prototype: u16) -> u16 {
    let npc = lobby
        .merchants
        .iter()
        .find(|n| n.prototype == prototype)
        .unwrap();
    s.position = npc.position;
    s.entered = true;
    s.zone_loading = false;
    s.merchant = Some(npc.id);
    s.npc_dialog = None;
    npc.id
}
async fn socket() -> (TcpStream, TcpStream, SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, peer) = listener.accept().await.unwrap();
    (server, client, peer)
}
async fn read(client: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut header = [0; 4];
        client.read_exact(&mut header).await.unwrap();
        assert_eq!(&header[..2], &[0xaa, 0x55]);
        let mut p = vec![0; usize::from(u16::from_le_bytes([header[2], header[3]]))];
        client.read_exact(&mut p).await.unwrap();
        let mut tail = [0; 2];
        client.read_exact(&mut tail).await.unwrap();
        assert_eq!(tail, [0x55, 0xaa]);
        p
    })
    .await
    .unwrap()
}
async fn click(sock: &mut TcpStream, peer: SocketAddr, lobby: &Lobby, s: &mut Session, id: u16) {
    let mut p = vec![0x20];
    p.extend(id.to_le_bytes());
    crate::shops::handle(sock, peer, lobby, "test", s, &p).await;
}
async fn choose(sock: &mut TcpStream, peer: SocketAddr, lobby: &Lobby, s: &mut Session, index: u8) {
    crate::npc_scripts::handle(sock, peer, lobby, "test", s, &[0x55, index, 0]).await;
}

#[test]
fn fragment_rewards_prefer_existing_stacks_before_empty_slots_and_respect_stack_limits() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    at(&lobby, &mut s, 603);
    let catalog = lobby.combat.as_ref().unwrap();
    let choices = pool(&lobby.db.lock().unwrap(), catalog, 389160000).unwrap();
    let mut offset = 0;
    let mut stackable = None;
    let mut equipment = None;
    for (id, weight) in choices {
        if combat::integer(catalog.item(id).unwrap().0, 23) != 0 {
            stackable.get_or_insert((id, offset));
        } else {
            equipment.get_or_insert((id, offset));
        }
        offset += weight;
    }
    let (reward, roll) = stackable.expect("Primary fragment pool includes stackable rewards");
    for (fragments, existing, earlier_full, destination, expected_count) in [
        (1, 7, false, 10, 8),
        (2, 7, false, 10, 8),
        (1, 9998, false, 10, 9999),
        (1, 9999, false, 0, 1),
        (1, 7, true, 10, 8),
    ] {
        item(&lobby, &mut s, 389160000, fragments);
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO character_inventory VALUES('test',0,10,?1,123,?2)",
                params![reward, existing],
            )
            .unwrap();
            if earlier_full {
                db.execute(
                    "INSERT INTO character_inventory VALUES('test',0,2,?1,123,9999)",
                    [reward],
                )
                .unwrap();
            }
        }
        let result = attempt(&lobby, "test", &s, &request(&s, 389160000), roll).unwrap();
        assert_eq!(result.reply[7], destination as u8);
        assert_eq!(result.bag[destination].id, reward);
        assert_eq!(result.bag[destination].count, expected_count);
        if destination == 10 {
            assert_eq!(result.bag[10].durability, 123);
            assert_eq!(result.bag[0].count, fragments - 1);
        } else {
            assert_eq!(result.bag[10].count, 9999);
        }
        if earlier_full {
            assert_eq!(result.bag[2].count, 9999);
        }
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            result.bag
        );
    }
    let (reward, roll) = equipment.expect("Primary fragment pool includes equipment");
    item(&lobby, &mut s, 389160000, 1);
    lobby
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO character_inventory VALUES('test',0,10,?1,123,1)",
            [reward],
        )
        .unwrap();
    let result = attempt(&lobby, "test", &s, &request(&s, 389160000), roll).unwrap();
    assert_eq!(result.reply[7], 0);
    assert_eq!((result.bag[0].id, result.bag[0].count), (reward, 1));
    assert_eq!((result.bag[10].durability, result.bag[10].count), (123, 1));
}

#[test]
fn all_seven_fragment_pools_use_complete_primary_weights_and_original_rewards() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    at(&lobby, &mut s, 603);
    let catalog = lobby.combat.as_ref().unwrap();
    for fragment in (389160000..=389166000).step_by(1000) {
        let choices = pool(&lobby.db.lock().unwrap(), catalog, fragment).unwrap();
        assert_eq!(choices.iter().map(|c| c.1).sum::<u32>(), 10000);
        for roll in [0, 9999] {
            item(&lobby, &mut s, fragment, 1);
            let exchange = attempt(&lobby, "test", &s, &request(&s, fragment), roll).unwrap();
            assert_eq!(
                exchange.bag.iter().map(|i| u32::from(i.count)).sum::<u32>(),
                1
            );
            assert!(!exchange.bag.iter().any(|i| i.id == fragment));
            assert!(
                choices
                    .iter()
                    .any(|(id, _)| *id
                        == u32::from_le_bytes(exchange.reply[3..7].try_into().unwrap()))
            );
            let before = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
            assert!(attempt(&lobby, "test", &s, &request(&s, fragment), roll).is_err());
            assert_eq!(
                starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
                before
            );
        }
    }
}

#[tokio::test]
async fn chaotic_generator_native_window_and_exchange_refresh_reused_inventory_slot() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let id = at(&lobby, &mut s, 603);
    item(&lobby, &mut s, 389160000, 1);
    let (mut sock, mut client, peer) = socket().await;
    click(&mut sock, peer, &lobby, &mut s, id).await;
    assert_eq!(
        read(&mut client).await,
        [0x5b, 4, id as u8, (id >> 8) as u8]
    );
    let p = request(&s, 389160000);
    crate::upgrading::handle(&mut sock, peer, &lobby, "test", &mut s, &p).await;
    let update = read(&mut client).await;
    assert_eq!(update[0], 0x3d);
    assert_eq!(u16::from_le_bytes([update[1], update[2]]), 2);
    let result = read(&mut client).await;
    assert_eq!(&result[..3], &[0x5b, 5, 1]);
    assert_eq!(result.len(), 14);
    assert_eq!(read(&mut client).await[..2], [0x33, 9]);
    assert_eq!(read(&mut client).await[0], 0x54);
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
        s.inventory
    );
    let before = s.inventory;
    let coins = s.coins;
    let unsold = lobby
        .combat
        .as_ref()
        .unwrap()
        .item_ids()
        .find(|id| {
            lobby
                .combat
                .as_ref()
                .unwrap()
                .item(*id)
                .is_some_and(|(row, _)| combat::integer(row, 35) == 0)
        })
        .unwrap();
    let mut forged = vec![0x21, 1];
    forged.extend(0u32.to_le_bytes());
    forged.extend(id.to_le_bytes());
    forged.extend(unsold.to_le_bytes());
    forged.push(1);
    forged.extend(1u16.to_le_bytes());
    crate::shops::handle(&mut sock, peer, &lobby, "test", &mut s, &forged).await;
    assert_eq!(s.inventory, before);
    assert_eq!(s.coins, coins);
}

#[test]
fn fragment_exchange_rejects_full_bags_foreign_npcs_distance_and_database_failure_atomically() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    at(&lobby, &mut s, 603);
    item(&lobby, &mut s, 389160000, 2);
    let p = request(&s, 389160000);
    {
        let db = lobby.db.lock().unwrap();
        for index in 1..28 {
            db.execute(
                "INSERT INTO character_inventory VALUES('test',0,?1,111210001,1000,1)",
                [index],
            )
            .unwrap();
        }
    }
    let before = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
    assert!(
        attempt(&lobby, "test", &s, &p, 0)
            .err()
            .unwrap()
            .to_string()
            .contains("full")
    );
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
        before
    );
    item(&lobby, &mut s, 389160000, 2);
    let before = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
    s.position.x += 101;
    assert!(attempt(&lobby, "test", &s, &p, 0).is_err());
    s.position.x -= 101;
    let mut foreign = p.clone();
    foreign[2..4].copy_from_slice(&12001u16.to_le_bytes());
    assert!(attempt(&lobby, "test", &s, &foreign, 0).is_err());
    lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_reward BEFORE INSERT ON character_inventory BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(attempt(&lobby, "test", &s, &p, 0).is_err());
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
        before
    );
}

#[tokio::test]
async fn all_eight_valkyrie_choices_use_english_native_menus_and_one_owned_certificate() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let (mut sock, _client, peer) = socket().await;
    for (root, certificate, first_reward) in [(0, 800180000, 508011441), (1, 800170000, 508013318)]
    {
        for variant in 0..4 {
            item(&lobby, &mut s, certificate, 1);
            let id = at(&lobby, &mut s, 16085);
            click(&mut sock, peer, &lobby, &mut s, id).await;
            assert_eq!(s.npc_dialog.as_ref().unwrap().choices[..2], [1008, 1009]);
            choose(&mut sock, peer, &lobby, &mut s, root).await;
            choose(&mut sock, peer, &lobby, &mut s, variant).await;
            choose(&mut sock, peer, &lobby, &mut s, 0).await;
            assert_eq!(s.inventory[0].id, first_reward + u32::from(variant));
            assert_eq!(s.inventory[0].count, 1);
            let before = s.inventory;
            choose(&mut sock, peer, &lobby, &mut s, 0).await;
            assert_eq!(s.inventory, before);
        }
    }
    item(&lobby, &mut s, 800180000, 2);
    for index in 1..28 {
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO character_inventory VALUES('test',0,?1,111210001,1000,1)",
                [index],
            )
            .unwrap();
    }
    s.inventory = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
    let before = s.inventory;
    let id = at(&lobby, &mut s, 16085);
    click(&mut sock, peer, &lobby, &mut s, id).await;
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_eq!(
        starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
        before
    );
}

#[tokio::test]
async fn kc_hemes_uses_existing_sundries_stock_and_normal_coin_transactions() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let id = at(&lobby, &mut s, 27011);
    let (mut sock, mut client, peer) = socket().await;
    click(&mut sock, peer, &lobby, &mut s, id).await;
    let mut expected = vec![0x3a];
    expected.extend(255000u32.to_le_bytes());
    assert_eq!(read(&mut client).await, expected);
    let mut buy = vec![0x21, 1];
    buy.extend(255000u32.to_le_bytes());
    buy.extend(id.to_le_bytes());
    buy.extend(381001000u32.to_le_bytes());
    buy.push(0);
    buy.extend(1u16.to_le_bytes());
    crate::shops::handle(&mut sock, peer, &lobby, "test", &mut s, &buy).await;
    assert_eq!(s.inventory[0].id, 381001000);
    assert!(s.coins < 1000000);
}
