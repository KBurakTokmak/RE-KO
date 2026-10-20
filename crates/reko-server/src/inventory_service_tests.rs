use crate::{starter, storage, upgrading};
use rusqlite::params;
use tokio::{io::AsyncReadExt, net::TcpStream};

async fn updates(client: &mut TcpStream) -> Vec<Vec<u8>> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut packets = Vec::new();
        loop {
            let mut header = [0; 4];
            client.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[..2], &[0xaa, 0x55]);
            let len = usize::from(u16::from_le_bytes([header[2], header[3]]));
            let mut body = vec![0; len + 2];
            client.read_exact(&mut body).await.unwrap();
            assert_eq!(&body[len..], &[0x55, 0xaa]);
            body.truncate(len);
            let done = body[0] == 0x1f;
            packets.push(body);
            if done {
                return packets;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn trina_upgrade_then_bank_transfers_leave_icon_ownership_with_the_native_windows() {
    let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
        return;
    };
    s.anvil = Some((5001, 0, s.position));
    {
        let db = lobby.db.lock().unwrap();
        crate::population::tests::seed(&db);
        db.execute(
            "UPDATE world_spawns SET monster=0,kind=31,x0=3200,x1=3200,z0=4000,z1=4000",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO world_upgrade_item_types VALUES(110110000,1,0)",
            [],
        )
        .unwrap();
        for (pos, id, count) in [
            (0, 110110001, 1),
            (1, 700002000, 1),
            (2, 700002000, 1),
            (3, 379021000, 5),
        ] {
            db.execute(
                "INSERT INTO character_inventory VALUES('test',0,?1,?2,1,?3)",
                params![pos, id, count],
            )
            .unwrap();
        }
        let mut row = serde_json::json!({"nIndex":100000,"nNPCNum":5001,"nOriginType":-1,"nOriginItem":10001,"nReqItem1":379021000,"nReqNoah":100,"bRateType":3,"nGenRate":10000,"nGiveItem":1});
        for i in 2..=8 {
            row[format!("nReqItem{i}")] = 0.into();
        }
        db.execute(
            "INSERT INTO world_reference_rules VALUES('ITEM_UPGRADE',100000,?1)",
            [row.to_string()],
        )
        .unwrap();
        s.inventory = starter::inventory(&db, "test", 0).unwrap();
    }
    lobby.merchants =
        crate::shops::load_population(&lobby.db.lock().unwrap(), &reko_common::game_dir()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut socket, peer) = listener.accept().await.unwrap();
    let mut request = vec![0x5b, 2, 1, 0, 0];
    for (id, pos) in [(110110001u32, 0), (379021000, 3), (700002000, 1)]
        .into_iter()
        .chain(std::iter::repeat_n((0, 255), 7))
    {
        request.extend(id.to_le_bytes());
        request.push(pos);
    }
    upgrading::handle(&mut socket, peer, &lobby, "test", &mut s, &request).await;
    let packets = updates(&mut client).await;
    assert!(packets.iter().any(|p| p[..4] == [0x5b, 2, 1, 1]));
    assert!(!packets.iter().any(|p| p[0] == 0x3d));
    assert_eq!(
        (
            s.inventory[0].id,
            s.inventory[1].id,
            s.inventory[2].id,
            s.inventory[3].count
        ),
        (110110002, 0, 700002000, 4)
    );
    s.position.x = 3200;
    s.position.z = 4000;
    s.merchant = Some(12000);
    s.anvil = None;
    for (action, id, src, dst, count) in [
        (2, 700002000u32, 2, 0, 1),
        (3, 700002000, 0, 4, 1),
        (2, 379021000, 3, 1, 2),
        (3, 379021000, 1, 3, 1),
    ] {
        let mut p = vec![0x45, action];
        p.extend(id.to_le_bytes());
        p.extend([0, src, dst]);
        p.extend(u32::to_le_bytes(count));
        storage::handle(&mut socket, peer, &lobby, "test", &mut s, &p).await;
        let replies = updates(&mut client).await;
        assert!(replies.iter().any(|p| p == &[0x45, action, 1]));
        assert!(!replies.iter().any(|p| p[0] == 0x3d));
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            s.inventory
        );
    }
    assert_eq!(
        (s.inventory[2].id, s.inventory[4].id, s.inventory[3].count),
        (0, 700002000, 3)
    );
    assert_eq!(s.coins, 999900);
}
