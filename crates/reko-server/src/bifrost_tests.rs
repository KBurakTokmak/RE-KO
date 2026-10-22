use super::*;
use crate::movement;

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
            "(zone=71 AND prototype=602) OR (zone=31 AND prototype=16094)",
        ),
        (
            "world_spawn_conditions",
            "entity_id IN (SELECT id FROM world_spawns WHERE (zone=71 AND prototype=602) OR (zone=31 AND prototype=16094))",
        ),
        ("world_town_starts", "zone IN(1,2,31,71)"),
        (
            "world_reference_rules",
            "table_name='QUEST_HELPER' AND rule_id IN(4243,4260)",
        ),
    ] {
        let mut query = reference
            .prepare(&format!("SELECT * FROM {table} WHERE {filter}"))
            .unwrap();
        let columns = query.column_count();
        let rows = query
            .query_map([], |r| {
                (0..columns)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap();
        for row in rows {
            db.execute(
                &format!(
                    "INSERT INTO {table} VALUES({})",
                    vec!["?"; columns].join(",")
                ),
                rusqlite::params_from_iter(row.unwrap()),
            )
            .unwrap();
        }
    }
    db.execute(
        "INSERT INTO world_reference_rules VALUES('BIFROST_ACCESS',31,?1)",
        [r#"{"mode":"always_open"}"#],
    )
    .unwrap();
    lobby.merchants = crate::shops::load_population(&db, &reko_common::game_dir()).unwrap();
    assert_eq!(lobby.merchants.len(), 3);
    drop(db);
    Some((lobby, s))
}

async fn packet(client: &mut TcpStream) -> Vec<u8> {
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
    .unwrap()
}

async fn click(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    s: &mut Session,
    prototype: u16,
) {
    let npc = lobby
        .merchants
        .iter()
        .find(|n| n.prototype == prototype)
        .unwrap();
    s.position = npc.position;
    s.entered = true;
    s.zone_loading = false;
    s.npc_dialog = None;
    let mut request = vec![0x20];
    request.extend(npc.id.to_le_bytes());
    crate::shops::handle(sock, peer, lobby, "test", s, &request).await;
    assert!(s.npc_dialog.is_some());
}

#[tokio::test]
async fn both_nations_enter_bifrost_and_return_with_payment_replay_and_loading_guards() {
    for nation in [1, 2] {
        let Some((lobby, mut s)) = setup() else {
            return;
        };
        s.character.class = if nation == 1 { 108 } else { 208 };
        s.character.race = if nation == 1 { 2 } else { 12 };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut sock, peer) = listener.accept().await.unwrap();
        let before = s.inventory;
        let coins = s.coins;
        click(&mut sock, peer, &lobby, &mut s, 602).await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 166);
        handle(&mut sock, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        let mut saw_warp = false;
        assert_eq!(s.position.zone, 31);
        for _ in 0..8 {
            let p = packet(&mut client).await;
            if p.first() == Some(&0x27) {
                assert_eq!(&p[..3], &[0x27, 3, 31]);
                saw_warp = true;
                break;
            }
        }
        assert!(saw_warp);
        assert_eq!(
            (s.position.x, s.position.z),
            if nation == 1 {
                (780, 7300)
            } else {
                (2450, 9500)
            }
        );
        assert!(s.zone_loading && !s.entered);
        assert!(s.visible_entities.is_empty());
        assert!(s.npc_dialog.is_none());
        assert_eq!(
            movement::load(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            s.position
        );
        let destination = s.position;
        handle(&mut sock, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        assert_eq!(s.position, destination);
        crate::travel::handle(&mut sock, peer, &lobby, "test", &mut s, &[0x27, 1]).await;
        crate::travel::handle(&mut sock, peer, &lobby, "test", &mut s, &[0x27, 2]).await;
        assert!(s.entered && !s.zone_loading);
        assert!(!s.visible_entities.is_empty());
        click(&mut sock, peer, &lobby, &mut s, 16094).await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 501);
        let prompt = loop {
            let p = packet(&mut client).await;
            if p.first() == Some(&0x55) {
                break p;
            }
        };
        assert_eq!(u16::from_le_bytes([prompt[1], prompt[2]]), 16094);
        assert_eq!(i32::from_le_bytes(prompt[8..12].try_into().unwrap()), 4071);
        let talks = reko_assets::Table::from_encrypted_bytes(
            &std::fs::read(reko_common::game_dir().join("Data/Quest_Talk_us.tbl")).unwrap(),
        )
        .unwrap();
        assert!(talks.rows.iter().any(|r|combat::integer(r,0)==4071 && matches!(&r[1],reko_assets::CellValue::String(text) if text=="Do you want to leave?")));
        handle(&mut sock, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
        assert_eq!(s.position.zone, 71);
        assert_eq!(
            (s.position.x, s.position.z),
            if nation == 1 {
                (10140, 10200)
            } else {
                (10260, 9840)
            }
        );
        assert_eq!(s.inventory, before);
        assert_eq!(s.coins, coins);
    }
}

#[tokio::test]
async fn bifrost_policy_changes_wrong_nation_coordinates_and_distant_replies_fail_closed() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut sock, peer) = listener.accept().await.unwrap();
    click(&mut sock, peer, &lobby, &mut s, 602).await;
    let before = s.position;
    assert!(
        crate::bifrost::destination(
            &lobby.db.lock().unwrap(),
            &s,
            &[i64::from(s.player_id), 31, 245, 950]
        )
        .is_err()
    );
    lobby
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE world_reference_rules SET data=?1 WHERE table_name='BIFROST_ACCESS'",
            [r#"{"mode":"event"}"#],
        )
        .unwrap();
    handle(&mut sock, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
    assert_eq!(s.position, before);
    assert_eq!(
        crate::bifrost::victory(&lobby.db.lock().unwrap()).unwrap(),
        0
    );
    lobby
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE world_reference_rules SET data=?1 WHERE table_name='BIFROST_ACCESS'",
            [r#"{"mode":"always_open"}"#],
        )
        .unwrap();
    click(&mut sock, peer, &lobby, &mut s, 602).await;
    s.position.x += 1000;
    let distant = s.position;
    handle(&mut sock, peer, &lobby, "test", &mut s, &[0x55, 0, 0]).await;
    assert_eq!(s.position, distant);
    let uid = i64::from(s.player_id);
    s.position.zone = 31;
    s.progress.level = 20;
    assert_eq!(
        crate::bifrost::destination(&lobby.db.lock().unwrap(), &s, &[uid, 1, 441, 1625])
            .unwrap()
            .zone,
        1
    );
    assert!(
        crate::bifrost::destination(&lobby.db.lock().unwrap(), &s, &[uid + 1, 1, 441, 1625])
            .is_err()
    );
}
