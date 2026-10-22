use super::*;
use crate::{movement, skills, social};

fn setup() -> Option<(Lobby, Session)> {
    let source = std::path::Path::new("../../reko.db");
    if !source.exists() {
        return None;
    }
    let (mut lobby, s) = crate::world_service_tests::setup()?;
    let reference =
        Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let db = lobby.db.lock().unwrap();
    for (table, filter) in [
        (
            "world_spawns",
            "prototype=7001 AND kind=28 AND zone BETWEEN32 AND34",
        ),
        (
            "world_spawn_conditions",
            "entity_id IN (SELECT id FROM world_spawns WHERE prototype=7001 AND zone BETWEEN32 AND34)",
        ),
        ("world_town_starts", "zone IN (30,32,33,34)"),
        (
            "world_reference_rules",
            "table_name='EVENT_TRIGGER' OR (table_name='QUEST_HELPER' AND rule_id=4056)",
        ),
    ] {
        let filter = filter.replace("BETWEEN32 AND34", "BETWEEN 32 AND 34");
        let mut q = reference
            .prepare(&format!("SELECT * FROM {table} WHERE {filter}"))
            .unwrap();
        let columns = q.column_count();
        let rows = q
            .query_map([], |r| {
                (0..columns)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap();
        let placeholders = vec!["?"; columns].join(",");
        for row in rows {
            db.execute(
                &format!("INSERT INTO {table} VALUES ({placeholders})"),
                rusqlite::params_from_iter(row.unwrap()),
            )
            .unwrap();
        }
    }
    lobby.merchants = crate::shops::load_population(&db, &reko_common::game_dir()).unwrap();
    assert_eq!(lobby.merchants.len(), 37);
    drop(db);
    Some((lobby, s))
}

fn at_gate(lobby: &Lobby, s: &mut Session, zone: u8, trap: u16) -> u16 {
    let id=lobby.db.lock().unwrap().query_row("SELECT s.id FROM world_spawns s JOIN world_spawn_conditions c ON c.entity_id=s.id WHERE s.zone=?1 AND c.trap=?2",params![zone,trap],|r|r.get::<_,u16>(0)).unwrap();
    s.position = lobby
        .merchants
        .iter()
        .find(|m| m.id == id)
        .unwrap()
        .position;
    s.entered = true;
    s.zone_loading = false;
    s.npc_dialog = None;
    movement::save(&lobby.db.lock().unwrap(), "test", 0, s.position).unwrap();
    id
}

fn key(lobby: &Lobby, account: &str, id: u32, count: u16) {
    let db = lobby.db.lock().unwrap();
    db.execute(
        "DELETE FROM character_inventory WHERE account=?1 AND slot=0",
        [account],
    )
    .unwrap();
    if count > 0 {
        db.execute(
            "INSERT INTO character_inventory VALUES(?1,0,0,?2,1,?3)",
            params![account, id, count],
        )
        .unwrap();
    }
}
fn count(lobby: &Lobby, account: &str, id: u32) -> u32 {
    starter::inventory(&lobby.db.lock().unwrap(), account, 0)
        .unwrap()
        .iter()
        .filter(|i| i.id == id)
        .map(|i| u32::from(i.count))
        .sum()
}
async fn socket() -> (TcpStream, TcpStream, SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (sock, peer) = listener.accept().await.unwrap();
    (sock, client, peer)
}
async fn open(sock: &mut TcpStream, peer: SocketAddr, lobby: &Lobby, s: &mut Session, id: u16) {
    let mut p = vec![0x20];
    p.extend(id.to_le_bytes());
    crate::shops::handle(sock, peer, lobby, "test", s, &p).await;
    assert_eq!(s.npc_dialog.as_ref().unwrap().helper, 4056);
}
async fn choose(sock: &mut TcpStream, peer: SocketAddr, lobby: &Lobby, s: &mut Session, index: u8) {
    handle(sock, peer, lobby, "test", s, &[0x55, index, 0]).await;
}

async fn read_packet(client: &mut TcpStream) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut header = [0; 4];
        client.read_exact(&mut header).await.unwrap();
        assert_eq!(&header[..2], &[0xaa, 0x55]);
        let mut body = vec![0; usize::from(u16::from_le_bytes([header[2], header[3]]))];
        client.read_exact(&mut body).await.unwrap();
        let mut tail = [0; 2];
        client.read_exact(&mut tail).await.unwrap();
        assert_eq!(tail, [0x55, 0xaa]);
        body
    })
    .await
    .expect("Destination population must arrive without a movement packet")
}

async fn arrival(client: &mut TcpStream, s: &Session, expected: &std::collections::HashSet<u16>) {
    let mut warped = false;
    let mut spawned = std::collections::HashSet::new();
    loop {
        let packet = read_packet(client).await;
        match packet[0] {
            0x1e => {
                assert_eq!(&packet[1..3], &s.position.x.to_le_bytes());
                assert_eq!(&packet[3..5], &s.position.z.to_le_bytes());
                warped = true;
            }
            0x0a if packet[1] == 1 => {
                assert!(warped, "Destination NPC details must follow the warp");
                spawned.insert(u16::from_le_bytes([packet[2], packet[3]]));
            }
            0x1c => {
                assert!(warped);
                let listed = packet[3..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes(*b))
                    .collect::<std::collections::HashSet<_>>();
                assert_eq!(&spawned, expected);
                assert_eq!(&listed, expected);
                assert_eq!(&s.visible_entities, expected);
                break;
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn abyss_floor_and_queued_party_arrivals_stream_npcs_and_monsters_without_moving() {
    let Some((mut lobby, mut s)) = setup() else {
        return;
    };
    let destination_gate = at_gate(&lobby, &mut s, 32, 2);
    let destination = s.position;
    {
        let db = lobby.db.lock().unwrap();
        crate::population::tests::seed(&db);
        db.execute(
            "UPDATE world_spawns SET zone=32,x0=?1,x1=?1,z0=?2,z1=?2 WHERE id=12000",
            params![destination.x, destination.z],
        )
        .unwrap();
    }
    let world =
        std::sync::Arc::new(crate::monsters::World::load(&reko_common::game_dir()).unwrap());
    world
        .load_population(&lobby.db.lock().unwrap(), &reko_common::game_dir())
        .unwrap();
    lobby.monsters = Some(world.clone());
    let mut expected = world
        .nearby_spawns(destination)
        .iter()
        .map(|p| u16::from_le_bytes([p[2], p[3]]))
        .collect::<std::collections::HashSet<_>>();
    expected.extend(
        lobby
            .merchants
            .iter()
            .filter(|m| m.visible_from(destination))
            .map(|m| m.id),
    );
    assert!(expected.contains(&destination_gate));
    assert!(expected.contains(&12000));
    let (mut sock, mut client, peer) = socket().await;
    let id = at_gate(&lobby, &mut s, 32, 1);
    s.visible_entities.insert(id);
    s.entity_region = Some((32, s.position.x / 480, s.position.z / 480));
    open(&mut sock, peer, &lobby, &mut s, id).await;
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    arrival(&mut client, &s, &expected).await;

    at_gate(&lobby, &mut s, 32, 1);
    lobby
        .social
        .lock()
        .unwrap()
        .transfers
        .insert(s.player_id, destination);
    crate::clan_promotion::transfer(&mut sock, peer, &lobby, &mut s).await;
    arrival(&mut client, &s, &expected).await;

    let other_zone = movement::Position {
        zone: 33,
        ..destination
    };
    lobby
        .social
        .lock()
        .unwrap()
        .transfers
        .insert(s.player_id, other_zone);
    crate::clan_promotion::transfer(&mut sock, peer, &lobby, &mut s).await;
    assert_eq!(&read_packet(&mut client).await[..3], &[0x27, 3, 33]);
    assert!(s.zone_loading && !s.entered);
    assert!(s.visible_entities.is_empty());
    assert!(s.entity_region.is_none());
    use tokio::io::AsyncReadExt;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), client.read_u8())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn every_source_abyss_teleporter_opens_and_all_forward_floors_resolve_without_imports() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let (mut sock, _client, peer) = socket().await;
    for (zone, traps) in [(32, 1..=20), (33, 21..=36), (34, 37..=37)] {
        for trap in traps {
            let id = at_gate(&lobby, &mut s, zone, trap);
            open(&mut sock, peer, &lobby, &mut s, id).await;
            assert!(
                s.npc_dialog
                    .as_ref()
                    .unwrap()
                    .choices
                    .iter()
                    .any(|i| *i >= 0)
            );
        }
    }
    for (zone, traps) in [(32, 1..=19), (33, 21..=35)] {
        for trap in traps {
            key(&lobby, "test", 379080000, 1);
            let id = at_gate(&lobby, &mut s, zone, trap);
            s.inventory = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
            let before = s.position;
            open(&mut sock, peer, &lobby, &mut s, id).await;
            assert!(matches!(
                s.npc_dialog.as_ref().unwrap().choices[0],
                30010 | 30011 | 30110 | 30111
            ));
            choose(&mut sock, peer, &lobby, &mut s, 0).await;
            assert_eq!(s.position.zone, zone);
            assert_ne!(
                s.position, before,
                "Forward travel failed at zone{zone} trap{trap}"
            );
            assert!(s.npc_dialog.is_none());
            let after = s.position;
            choose(&mut sock, peer, &lobby, &mut s, 0).await;
            assert_eq!(s.position, after);
            assert_eq!(
                movement::load(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
                after
            );
            assert_eq!(
                starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
                1000000
            );
        }
    }
}

#[tokio::test]
async fn abyss_keys_previous_exit_hell_and_dragon_connections_keep_native_choice_checks() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let (mut sock, _client, peer) = socket().await;
    let id = at_gate(&lobby, &mut s, 32, 12);
    let before = s.position;
    open(&mut sock, peer, &lobby, &mut s, id).await;
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_eq!(s.position, before);
    assert_eq!(count(&lobby, "test", 379080000), 0);
    let id = at_gate(&lobby, &mut s, 32, 2);
    open(&mut sock, peer, &lobby, &mut s, id).await;
    choose(&mut sock, peer, &lobby, &mut s, 2).await;
    assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 30395);
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_eq!(
        (s.position.zone, s.position.x, s.position.z),
        (32, 500, 740)
    );
    let id = at_gate(&lobby, &mut s, 32, 1);
    open(&mut sock, peer, &lobby, &mut s, id).await;
    choose(&mut sock, peer, &lobby, &mut s, 2).await;
    assert_eq!(
        (s.position.zone, s.position.x, s.position.z),
        (30, 5000, 2500)
    );
    assert!(s.zone_loading);
    key(&lobby, "test", 379111000, 1);
    let id = at_gate(&lobby, &mut s, 32, 1);
    open(&mut sock, peer, &lobby, &mut s, id).await;
    assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 30075);
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_eq!(
        (s.position.zone, s.position.x, s.position.z),
        (33, 620, 910)
    );
    assert_eq!(count(&lobby, "test", 379111000), 0);
    let id = at_gate(&lobby, &mut s, 33, 21);
    open(&mut sock, peer, &lobby, &mut s, id).await;
    let before = s.position;
    choose(&mut sock, peer, &lobby, &mut s, 4).await;
    assert_eq!(s.position, before);
    assert_eq!(count(&lobby, "test", 389200000), 0);
    key(&lobby, "test", 389200000, 1);
    let id = at_gate(&lobby, &mut s, 33, 21);
    open(&mut sock, peer, &lobby, &mut s, id).await;
    choose(&mut sock, peer, &lobby, &mut s, 4).await;
    assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 30112);
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_eq!(
        (s.position.zone, s.position.x, s.position.z),
        (34, 1120, 150)
    );
    assert_eq!(count(&lobby, "test", 389200000), 0);
    let id = at_gate(&lobby, &mut s, 34, 37);
    open(&mut sock, peer, &lobby, &mut s, id).await;
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_eq!(
        (s.position.zone, s.position.x, s.position.z),
        (33, 4450, 4590)
    );
}

#[tokio::test]
async fn abyss_party_key_failure_rolls_back_every_member_and_success_moves_once() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let id = at_gate(&lobby, &mut s, 32, 12);
    let mut a = social::tests::player(&lobby, "AbyssFriendA", 1, 80);
    let mut b = social::tests::player(&lobby, "AbyssFriendB", 1, 80);
    for (name, p) in [
        ("test", &mut s),
        ("AbyssFriendA", &mut a),
        ("AbyssFriendB", &mut b),
    ] {
        p.position = lobby
            .merchants
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .position;
        movement::save(&lobby.db.lock().unwrap(), name, 0, p.position).unwrap();
        skills::save(&lobby.db.lock().unwrap(), name, 0, &p.skills).unwrap();
        social::synchronize(&lobby, name, p);
    }
    lobby
        .social
        .lock()
        .unwrap()
        .parties
        .insert(s.player_id, vec![s.player_id, a.player_id, b.player_id]);
    key(&lobby, "test", 379080000, 1);
    key(&lobby, "AbyssFriendA", 379080000, 1);
    let (mut sock, _client, peer) = socket().await;
    open(&mut sock, peer, &lobby, &mut s, id).await;
    let before = s.position;
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_eq!(s.position, before);
    for name in ["test", "AbyssFriendA"] {
        assert_eq!(count(&lobby, name, 379080000), 1);
    }
    assert!(lobby.social.lock().unwrap().transfers.is_empty());
    key(&lobby, "AbyssFriendB", 379080000, 1);
    choose(&mut sock, peer, &lobby, &mut s, 0).await;
    assert_ne!(s.position, before);
    for name in ["test", "AbyssFriendA", "AbyssFriendB"] {
        assert_eq!(count(&lobby, name, 379080000), 0);
        assert_eq!(
            movement::load(&lobby.db.lock().unwrap(), name, 0).unwrap(),
            s.position
        );
    }
    let hub = lobby.social.lock().unwrap();
    assert_eq!(hub.transfers[&a.player_id], s.position);
    assert_eq!(hub.transfers[&b.player_id], s.position);
}

#[tokio::test]
async fn abyss_shortcut_keys_keep_source_destinations_and_wrong_zone_triggers_fail_closed() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let (mut sock, _client, peer) = socket().await;
    for (zone, trap, item, x, z) in [
        (32, 1, 379081000, 2500, 1920),
        (32, 1, 379082000, 4500, 4370),
        (33, 21, 379083000, 1890, 4580),
        (33, 21, 379084000, 4440, 4580),
    ] {
        key(&lobby, "test", item, 1);
        let id = at_gate(&lobby, &mut s, zone, trap);
        open(&mut sock, peer, &lobby, &mut s, id).await;
        choose(&mut sock, peer, &lobby, &mut s, 1).await;
        choose(&mut sock, peer, &lobby, &mut s, 0).await;
        assert_eq!((s.position.zone, s.position.x, s.position.z), (zone, x, z));
        assert_eq!(count(&lobby, "test", item), 0);
    }
    let id = at_gate(&lobby, &mut s, 32, 1);
    {
        let db = lobby.db.lock().unwrap();
        db.execute("UPDATE world_reference_rules SET data=?1 WHERE table_name='EVENT_TRIGGER' AND rule_id=1",
            [r#"{"bNpcType":28,"sNpcID":1,"nTriggerNum":42001}"#]).unwrap();
    }
    let mut p = vec![0x20];
    p.extend(id.to_le_bytes());
    crate::shops::handle(&mut sock, peer, &lobby, "test", &mut s, &p).await;
    assert!(s.npc_dialog.is_none());
    assert_eq!(s.position.zone, 32);
}
