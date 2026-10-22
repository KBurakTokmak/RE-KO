use crate::{bosses, monsters, movement::Position, population, shops};
use rusqlite::{Connection, OpenFlags};
use std::{collections::HashSet, sync::Arc};

#[tokio::test]
async fn repaired_population_keeps_one_live_rare_boss_and_streams_both_ardream_towns_and_mobs() {
    let fixture = std::path::Path::new("../../logs/combat-zones-test.db");
    let game = reko_common::game_dir();
    if !fixture.exists() || !game.join("Data/NPC_Looks.tbl").exists() {
        return;
    }
    let db = Connection::open_with_flags(fixture, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let definitions = population::load(&db, &game).unwrap();
    let mut scheduler = bosses::Scheduler::load(&db, &definitions, 1000).unwrap();
    let active: Vec<_> = scheduler
        .active()
        .into_iter()
        .filter(|s| s.position.zone == 71)
        .collect();
    assert_eq!(active.len(), 14);
    assert_eq!(
        active
            .iter()
            .map(|s| s.name.to_lowercase())
            .collect::<HashSet<_>>()
            .len(),
        14
    );
    let members: HashSet<u16> = db
        .prepare("SELECT entity_id FROM world_boss_members WHERE pool_id LIKE '71:%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(members.len(), 63);
    for boss in &active {
        assert!(scheduler.death(boss.id, 2000).unwrap());
    }
    assert!(scheduler.tick(2000 + 21600000 - 1).is_empty());
    assert_eq!(scheduler.tick(2000 + 21600000).len(), 14);
    let world = Arc::new(monsters::World::load(&game).unwrap());
    world.load_population(&db, &game).unwrap();
    world.load_ai(&db).unwrap();
    let live = world.spawns(71);
    let live_ids: HashSet<_> = live
        .iter()
        .map(|p| u16::from_le_bytes([p[2], p[3]]))
        .collect();
    assert_eq!(live_ids.intersection(&members).count(), 14);
    for mob in definitions
        .iter()
        .filter(|s| s.monster && s.position.zone == 71 && !members.contains(&s.id))
    {
        assert!(
            live_ids.contains(&mob.id),
            "Ordinary monster {} must remain alive",
            mob.name
        );
    }
    assert_eq!(world.spawns(72).len(), 379);
    let merchants = shops::load_population(&db, &game).unwrap();
    assert_eq!(
        merchants.iter().filter(|m| m.position.zone == 72).count(),
        63
    );
    let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
        return;
    };
    lobby.monsters = Some(world);
    lobby.merchants = merchants;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut sock, peer) = listener.accept().await.unwrap();
    let monster = definitions
        .iter()
        .find(|s| s.monster && s.position.zone == 72)
        .unwrap();
    for position in [
        Position {
            zone: 72,
            x: 8430,
            z: 1400,
            y: 0,
        },
        Position {
            zone: 72,
            x: 1900,
            z: 8970,
            y: 0,
        },
        monster.position,
    ] {
        s.position = position;
        s.entity_region = None;
        s.visible_entities.clear();
        s.entered = true;
        population::refresh(&mut sock, peer, &lobby, &mut s).await;
        assert!(!s.visible_entities.is_empty());
        if position == monster.position {
            assert!(s.visible_entities.contains(&monster.id));
        } else {
            for kind in [21, 22, 31, 62] {
                assert!(
                    lobby
                        .merchants
                        .iter()
                        .any(|m| m.kind == kind && s.visible_entities.contains(&m.id)),
                    "Both Ardream towns need potion, sundries, storage and tower population"
                );
            }
        }
        use tokio::io::AsyncReadExt;
        let mut received = HashSet::new();
        loop {
            let mut header = [0; 4];
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.read_exact(&mut header),
            )
            .await
            .unwrap()
            .unwrap();
            let mut body = vec![0; usize::from(u16::from_le_bytes([header[2], header[3]]))];
            client.read_exact(&mut body).await.unwrap();
            let mut tail = [0; 2];
            client.read_exact(&mut tail).await.unwrap();
            if body[0] == 0x0a && body[1] == 1 {
                received.insert(u16::from_le_bytes([body[2], body[3]]));
            }
            if body[0] == 0x1c {
                break;
            }
        }
        assert_eq!(received, s.visible_entities);
    }
}
