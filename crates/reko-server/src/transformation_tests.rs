use super::*;
use tokio::{io::AsyncReadExt, net::TcpListener};

fn setup() -> Option<(Lobby, Session)> {
    let reference = std::path::Path::new("../../logs/reference1453/MAGIC_TYPE6.json");
    if !reference.exists() {
        return None;
    }
    let (mut lobby, mut s) = crate::world_service_tests::setup()?;
    lobby.skills = Some(skills::Catalog::load(&reko_common::game_dir()).unwrap());
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
    assert_eq!(rows.len(), 46);
    {
        let db = lobby.db.lock().unwrap();
        for row in rows {
            db.execute(
                "INSERT INTO world_reference_rules VALUES('MAGIC_TYPE6',?1,?2)",
                rusqlite::params![row["iNum"].as_u64().unwrap(), row.to_string()],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO character_inventory VALUES('test',0,0,381001000,1,2)",
            [],
        )
        .unwrap();
        skills::save(&db, "test", 0, &s.skills).unwrap();
        s.inventory = crate::starter::inventory(&db, "test", 0).unwrap();
    }
    Some((lobby, s))
}
async fn socket() -> (TcpStream, TcpStream, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, peer) = listener.accept().await.unwrap();
    (server, client, peer)
}
async fn packet(client: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut h = [0; 4];
        client.read_exact(&mut h).await.unwrap();
        assert_eq!(&h[..2], &[0xaa, 0x55]);
        let mut p = vec![0; usize::from(u16::from_le_bytes([h[2], h[3]]))];
        client.read_exact(&mut p).await.unwrap();
        let mut tail = [0; 2];
        client.read_exact(&mut tail).await.unwrap();
        assert_eq!(tail, [0x55, 0xaa]);
        p
    })
    .await
    .unwrap()
}
async fn use_skill(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    s: &mut Session,
    id: u32,
    phase: u8,
) {
    let mut p = skills::effect(id, s.player_id);
    p[1] = phase;
    assert!(magic::handle(sock, peer, &p, lobby, Some("test"), Some(s)).await);
}
fn count(lobby: &Lobby) -> u16 {
    crate::starter::inventory(&lobby.db.lock().unwrap(), "test", 0)
        .unwrap()
        .iter()
        .filter(|i| i.id == SCROLL)
        .map(|i| i.count)
        .sum()
}
async fn transformed(client: &mut TcpStream, id: u32) {
    let mut state = false;
    for _ in 0..12 {
        let p = packet(client).await;
        if p[0] == 0x29 {
            if p[3] == 5 {
                assert_eq!(&p[4..], &1u32.to_le_bytes());
                continue;
            }
            assert_eq!(p[3], 3);
            assert_eq!(u32::from_le_bytes(p[4..8].try_into().unwrap()), id);
            state = true;
        }
        if p.starts_with(&[0x31, 3]) {
            assert!(state);
            assert_eq!(u32::from_le_bytes(p[2..6].try_into().unwrap()), id);
            assert!((3598..=3600).contains(&i16::from_le_bytes([p[16], p[17]])));
            return;
        }
    }
    panic!("Missing original transformation effect");
}

#[tokio::test]
async fn every_original_scroll_form_transforms_both_nations_with_its_primary_class_mask() {
    for nation in [1, 2] {
        let Some((lobby, mut s)) = setup() else {
            return;
        };
        s.character.race = if nation == 1 { 2 } else { 12 };
        s.character.class = if nation == 1 { 110 } else { 210 };
        s.is_admin = true;
        let ids:Vec<u32>=lobby.db.lock().unwrap().prepare("SELECT rule_id FROM world_reference_rules WHERE table_name='MAGIC_TYPE6' ORDER BY rule_id").unwrap().query_map([],|r|r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(ids.len(), 46);
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE character_inventory SET count=46 WHERE account='test' AND slot=0",
                [],
            )
            .unwrap();
        let (mut sock, mut client, peer) = socket().await;
        for id in ids {
            use_skill(&mut sock, peer, &lobby, &mut s, LIST, 3).await;
            assert_eq!(packet(&mut client).await[..2], [0x31, 9]);
            use_skill(&mut sock, peer, &lobby, &mut s, id, 3).await;
            transformed(&mut client, id).await;
            assert_eq!(appearance(&s), id);
            use_skill(&mut sock, peer, &lobby, &mut s, id, 6).await;
            assert_eq!(packet(&mut client).await, [0x31, 7]);
            let _ = packet(&mut client).await;
        }
        assert_eq!(count(&lobby), 0);
    }
}

#[tokio::test]
async fn transformation_scroll_list_selection_replay_cancel_reconnect_and_expiry_preserve_resources()
 {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let (mut sock, mut client, peer) = socket().await;
    use_skill(&mut sock, peer, &lobby, &mut s, LIST, 3).await;
    let mut expected = vec![0x31, 9];
    expected.extend(LIST.to_le_bytes());
    assert_eq!(packet(&mut client).await, expected);
    assert_eq!(count(&lobby), 2);
    let stats = (s.combat.attack, s.combat.defence, s.skills.max_hp);
    use_skill(&mut sock, peer, &lobby, &mut s, 472010, 3).await;
    transformed(&mut client, 472010).await;
    assert_eq!(appearance(&s), 472010);
    assert_eq!(count(&lobby), 1);
    assert_eq!(stats, (s.combat.attack, s.combat.defence, s.skills.max_hp));
    use_skill(&mut sock, peer, &lobby, &mut s, 472010, 3).await;
    assert_eq!(packet(&mut client).await[..2], [0x31, 4]);
    let _ = packet(&mut client).await;
    assert_eq!(count(&lobby), 1);
    s.skills = skills::load(&lobby.db.lock().unwrap(), "test", 0).unwrap();
    s.transformation_visual = 1;
    refresh(&mut sock, peer, &lobby, "test", &mut s).await;
    transformed(&mut client, 472010).await;
    use_skill(&mut sock, peer, &lobby, &mut s, 472010, 6).await;
    assert_eq!(packet(&mut client).await, [0x31, 7]);
    assert_eq!(&packet(&mut client).await[4..], &1u32.to_le_bytes());
    assert!(
        !skills::load(&lobby.db.lock().unwrap(), "test", 0)
            .unwrap()
            .buffs
            .contains_key(&CATEGORY)
    );
    use_skill(&mut sock, peer, &lobby, &mut s, LIST, 3).await;
    let _ = packet(&mut client).await;
    use_skill(&mut sock, peer, &lobby, &mut s, 472010, 3).await;
    transformed(&mut client, 472010).await;
    assert_eq!(count(&lobby), 0);
    s.skills.buffs.get_mut(&CATEGORY).unwrap().until = 0;
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    s.next_regen = u64::MAX;
    crate::tick_character(&mut sock, peer, &lobby, "test", &mut s, skills::now_ms()).await;
    for _ in 0..8 {
        if packet(&mut client).await == [0x31, 7] {
            break;
        }
    }
    assert_eq!(appearance(&s), 1);
    assert!(!s.skills.buffs.contains_key(&CATEGORY));
}

#[tokio::test]
async fn transformation_missing_menu_pvp_buff_class_and_database_failures_do_not_consume_scrolls() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let (mut sock, _client, peer) = socket().await;
    use_skill(&mut sock, peer, &lobby, &mut s, 472010, 3).await;
    assert_eq!(count(&lobby), 2);
    s.position.zone = 71;
    use_skill(&mut sock, peer, &lobby, &mut s, LIST, 3).await;
    assert!(s.pending_cast.is_none());
    s.position.zone = 21;
    s.skills.buffs.insert(
        6,
        skills::Buff {
            skill: 108010,
            until: skills::now_ms() + 60000,
        },
    );
    use_skill(&mut sock, peer, &lobby, &mut s, LIST, 3).await;
    assert!(s.pending_cast.is_none());
    s.skills.buffs.clear();
    use_skill(&mut sock, peer, &lobby, &mut s, LIST, 3).await;
    use_skill(&mut sock, peer, &lobby, &mut s, 472160, 3).await;
    assert_eq!(count(&lobby), 2);
    lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_transform BEFORE INSERT ON character_buffs WHEN NEW.category=96 BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    use_skill(&mut sock, peer, &lobby, &mut s, 472010, 3).await;
    assert_eq!(count(&lobby), 2);
    assert_eq!(appearance(&s), 1);
    assert!(
        !skills::load(&lobby.db.lock().unwrap(), "test", 0)
            .unwrap()
            .buffs
            .contains_key(&CATEGORY)
    );
    lobby
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_transform;")
        .unwrap();
    use_skill(&mut sock, peer, &lobby, &mut s, 472010, 3).await;
    assert_eq!(count(&lobby), 1);
    let destination = crate::movement::Position {
        zone: 72,
        ..s.position
    };
    let _ = crate::travel::begin(&lobby, &mut s, destination);
    assert_eq!(s.transformation_visual, 0);
    refresh(&mut sock, peer, &lobby, "test", &mut s).await;
    assert_eq!(appearance(&s), 1);
    assert_eq!(s.transformation_visual, 1);
    assert!(!s.skills.buffs.contains_key(&CATEGORY));
}
