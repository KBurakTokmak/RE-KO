//! Imported spawn rows retain source rectangles, counts and original timers.
use crate::movement::Position;
use reko_assets::{CellValue, Table};
use rusqlite::Connection;
use std::{collections::HashSet, path::Path};

pub(crate) fn same_region_view(a: Position, b: Position) -> bool {
    a.zone == b.zone && (a.x / 480).abs_diff(b.x / 480) <= 1 && (a.z / 480).abs_diff(b.z / 480) <= 1
}
fn nearby_packets(lobby: &crate::lobby::Lobby, position: Position) -> Vec<Vec<u8>> {
    let mut packets = lobby
        .monsters
        .as_ref()
        .map_or_else(Vec::new, |w| w.nearby_spawns(position));
    packets.extend(
        lobby
            .merchants
            .iter()
            .filter(|n| n.visible_from(position))
            .map(|n| n.spawn()),
    );
    packets.sort_unstable_by_key(|p| u16::from_le_bytes([p[2], p[3]]));
    packets
}
pub(crate) async fn request(
    sock: &mut tokio::net::TcpStream,
    peer: std::net::SocketAddr,
    lobby: &crate::lobby::Lobby,
    s: &mut crate::world_entry::Session,
    p: &[u8],
) {
    if !s.entered || p.len() < 3 {
        return;
    }
    let count = usize::from(u16::from_le_bytes([p[1], p[2]]));
    if count >= 1000 || p.len() != 3 + count * 2 {
        return;
    }
    let requested: HashSet<_> = p[3..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let packets = nearby_packets(lobby, s.position)
        .into_iter()
        .filter(|p| requested.contains(&u16::from_le_bytes([p[2], p[3]])))
        .collect::<Vec<_>>();
    let mut reply = vec![0x1d];
    reply.extend((packets.len() as u16).to_le_bytes());
    for packet in packets {
        s.visible_entities
            .insert(u16::from_le_bytes([packet[2], packet[3]]));
        reply.extend(&packet[2..]);
    }
    super::send_response(sock, peer, &None, &reply, "NPC_REQUESTED_DETAILS").await;
}

pub(crate) async fn refresh(
    sock: &mut tokio::net::TcpStream,
    peer: std::net::SocketAddr,
    lobby: &crate::lobby::Lobby,
    session: &mut crate::world_entry::Session,
) {
    let region = (
        session.position.zone,
        session.position.x / 480,
        session.position.z / 480,
    );
    if session.entity_region == Some(region) {
        return;
    }
    let packets = nearby_packets(lobby, session.position);
    let current: HashSet<_> = packets
        .iter()
        .map(|p| u16::from_le_bytes([p[2], p[3]]))
        .collect();
    let mut leaving: Vec<_> = session
        .visible_entities
        .difference(&current)
        .copied()
        .collect();
    leaving.sort_unstable();
    for id in leaving {
        let mut out = vec![0x0a, 2];
        out.extend(id.to_le_bytes());
        super::send_response(sock, peer, &None, &out, "REGION_NPC_OUT").await;
    }
    for packet in packets {
        let id = u16::from_le_bytes([packet[2], packet[3]]);
        if !session.visible_entities.contains(&id) {
            super::send_response(sock, peer, &None, &packet, "REGION_NPC_IN").await;
        }
    }
    session.visible_entities = current;
    session.entity_region = Some(region);
    let mut ids: Vec<_> = session.visible_entities.iter().copied().collect();
    ids.sort_unstable();
    if !ids.is_empty() && ids.len() < 1000 {
        let mut list = vec![0x1c];
        list.extend((ids.len() as u16).to_le_bytes());
        for id in ids {
            list.extend(id.to_le_bytes());
        }
        super::send_response(sock, peer, &None, &list, "NPC_REGION_LIST").await;
    }
}

pub(crate) fn event_packet(
    event: &crate::monsters::Event,
    lobby: &crate::lobby::Lobby,
    s: &mut crate::world_entry::Session,
) -> Option<Vec<u8>> {
    if event.packet.first() == Some(&0x0b) && event.packet.len() == 11 {
        if !s.entered || s.position.zone != event.zone {
            return None;
        }
        let id = u16::from_le_bytes([event.packet[1], event.packet[2]]);
        let visible = lobby
            .monsters
            .as_ref()
            .is_some_and(|w| w.visible_from(id, s.position));
        if !visible {
            return s.visible_entities.remove(&id).then(|| {
                let mut p = vec![0x0a, 2];
                p.extend(id.to_le_bytes());
                p
            });
        }
        if s.visible_entities.insert(id) {
            return lobby
                .monsters
                .as_ref()?
                .nearby_spawns(s.position)
                .into_iter()
                .find(|p| u16::from_le_bytes([p[2], p[3]]) == id);
        }
        return Some(event.packet.clone());
    }
    event_visible(event, lobby, s).then(|| event.packet.clone())
}

pub(crate) fn event_visible(
    event: &crate::monsters::Event,
    lobby: &crate::lobby::Lobby,
    session: &mut crate::world_entry::Session,
) -> bool {
    if !session.entered || session.position.zone != event.zone {
        return false;
    }
    let p = &event.packet;
    if p.first() == Some(&0x23) && p.len() == 7 {
        let id = u32::from_le_bytes(p[3..7].try_into().unwrap());
        return lobby
            .monsters
            .as_ref()
            .and_then(|w| w.loot.as_ref())
            .is_some_and(|book| book.visible(id, &session.character.name));
    }
    if p.first() == Some(&0x0a) && p.len() >= 4 {
        let id = u16::from_le_bytes([p[2], p[3]]);
        if p[1] == 2 {
            return session.visible_entities.remove(&id);
        }
        if p[1] == 1
            && lobby
                .monsters
                .as_ref()
                .is_some_and(|w| w.visible_from(id, session.position))
        {
            session.visible_entities.insert(id);
            return true;
        }
        return false;
    }
    if matches!(p.first(), Some(0x22 | 0x11)) && p.len() >= 3 {
        return session
            .visible_entities
            .contains(&u16::from_le_bytes([p[1], p[2]]));
    }
    true
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(include_str!("population.sql"))
}

#[derive(Debug, Clone)]
pub(crate) struct Spawn {
    pub id: u16,
    pub monster: bool,
    pub position: Position,
    pub prototype: u16,
    pub appearance: u16,
    pub name: String,
    pub kind: u8,
    pub group: u32,
    pub nation: u8,
    pub level: u8,
    pub size: u16,
    pub weapons: [u32; 2],
    pub direction: u8,
    pub bounds: [u16; 4],
    pub hp: u32,
    pub armor: u16,
    pub resistance: [u16; 6],
    pub respawn_ms: u64,
    pub regen_type: u8,
    pub move_type: u8,
}

pub(crate) fn coordinate(a: u16, b: u16, random: u64) -> u16 {
    if a.abs_diff(b) <= 10 {
        a
    } else {
        a.min(b) + (random % (u64::from(a.abs_diff(b)) / 10 + 1)) as u16 * 10
    }
}

pub(crate) fn load(db: &Connection, game: &Path) -> anyhow::Result<Vec<Spawn>> {
    let looks = Table::from_encrypted_bytes(&std::fs::read(game.join("Data/NPC_Looks.tbl"))?)?;
    let appearances: HashSet<_> = looks
        .rows
        .iter()
        .filter_map(|r| match r.first() {
            Some(CellValue::U32(id)) => Some(*id),
            _ => None,
        })
        .collect();
    let mut query = db.prepare("SELECT id,monster,zone,prototype,appearance,name,kind,selling_group,nation,level,size,weapon1,weapon2,direction,x0,z0,x1,z1,hp,armor,fire_r,cold_r,lightning_r,magic_r,disease_r,poison_r,respawn_ms,regen_type,move_type FROM world_spawns WHERE id NOT IN (SELECT entity_id FROM world_spawn_conditions WHERE enabled=0) ORDER BY id")?;
    let rows = query
        .query_map([], |r| {
            let bounds = [r.get(14)?, r.get(15)?, r.get(16)?, r.get(17)?];
            let id: u16 = r.get(0)?;
            let seed = u64::from(id)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(crate::skills::now_ms());
            Ok(Spawn {
                id,
                monster: r.get(1)?,
                position: Position {
                    zone: r.get(2)?,
                    x: coordinate(bounds[0], bounds[2], seed),
                    z: coordinate(bounds[1], bounds[3], seed.rotate_left(29)),
                    y: 0,
                },
                prototype: r.get(3)?,
                appearance: r.get(4)?,
                name: r.get(5)?,
                kind: r.get(6)?,
                group: r.get(7)?,
                nation: r.get(8)?,
                level: r.get(9)?,
                size: r.get(10)?,
                weapons: [r.get(11)?, r.get(12)?],
                direction: r.get(13)?,
                bounds,
                hp: r.get(18)?,
                armor: r.get(19)?,
                resistance: [
                    r.get(20)?,
                    r.get(21)?,
                    r.get(22)?,
                    r.get(23)?,
                    r.get(24)?,
                    r.get(25)?,
                ],
                respawn_ms: r.get(26)?,
                regen_type: r.get(27)?,
                move_type: r.get(28)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for row in &rows {
        anyhow::ensure!(
            appearances.contains(&u32::from(row.appearance)),
            "spawn {} ({}) has no original1453 appearance {}",
            row.id,
            row.name,
            row.appearance
        );
        anyhow::ensure!(
            row.name.chars().all(|c| u32::from(c) <= 255)
                && row.name.chars().count() <= 255
                && row.hp > 0
                && row.respawn_ms > 0
                && row.size > 0,
            "invalid imported spawn {}",
            row.id
        );
    }
    Ok(rows)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn all_zone_import_preserves_event_conditions_and_native_names() {
        let path = std::path::Path::new("../../logs/allzones-import-test.db");
        let game = reko_common::game_dir();
        if !path.exists() || !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM world_spawns", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            6568
        );
        assert_eq!(
            db.query_row("SELECT COUNT(DISTINCT zone) FROM world_spawns", [], |r| r
                .get::<_, u32>(
                0
            ))
            .unwrap(),
            25
        );
        let rows = load(&db, &game).unwrap();
        assert_eq!(rows.len(), 6568);
        assert_eq!(rows.iter().filter(|s| s.position.zone == 21).count(), 533);
        let world = crate::monsters::World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        let packet = world.spawns(1);
        assert!(!packet.is_empty());
        let catalog = crate::combat::Catalog::load(
            &game,
            std::path::Path::new("../../logs/reference-coefficients.csv"),
        )
        .unwrap();
        let book = crate::loot::Book::load(&db, &catalog).unwrap();
        book.register_owner("Test", 1);
        for s in rows.iter().filter(|s| s.monster) {
            let _ = book.create(s.prototype, s.id, s.position, "Test");
        }
    }
    pub fn seed(db: &Connection) {
        initialize(db).unwrap();
        db.execute("INSERT INTO world_spawns VALUES (12000,'test',1,0,1,21,100,100,'Spawn Regression',0,0,0,10,100,0,0,0,3200,4000,3300,3900,100,500,100,100,100,100,100,100,20000,0,1,'')",[]).unwrap();
    }
    #[test]
    fn regions_and_reversed_rectangles_follow_reference_units() {
        let position = Position::default();
        assert!(same_region_view(
            position,
            Position {
                x: 3359,
                ..position
            }
        ));
        assert!(!same_region_view(
            position,
            Position {
                x: 3840,
                ..position
            }
        ));
        assert!(!same_region_view(
            position,
            Position {
                zone: 1,
                ..position
            }
        ));
        for random in 0..1000 {
            assert!((3200..=3300).contains(&coordinate(3300, 3200, random)));
            assert_eq!(coordinate(3300, 3290, random), 3300);
        }
    }
    #[test]
    fn imported_entities_load_with_original_models_and_spawn_fields() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let db = Connection::open_in_memory().unwrap();
        seed(&db);
        let rows = load(&db, &game).unwrap();
        assert_eq!(
            (
                rows.len(),
                rows[0].prototype,
                rows[0].appearance,
                rows[0].respawn_ms
            ),
            (1, 100, 100, 20000)
        );
        db.execute("UPDATE world_spawns SET appearance=65535", [])
            .unwrap();
        assert!(
            load(&db, &game).is_err(),
            "unknown client models must fail instead of entering the world"
        );
    }
    #[tokio::test]
    async fn region_streaming_updates_known_entities_and_filters_world_events() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let mut lobby = crate::lobby::Lobby::new(
            Connection::open_in_memory().unwrap(),
            crate::characters::CreationRules::fixture(),
            crate::world_entry::Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        let world = std::sync::Arc::new(crate::monsters::World::load(&game).unwrap());
        lobby.monsters = Some(world.clone());
        let c = crate::characters::Character {
            slot: 0,
            name: "RegionTest".into(),
            race: 2,
            class: 102,
            face: 0,
            hair: 0,
            stats: [60, 60, 80, 50, 50],
        };
        let mut s = crate::world_entry::Session::new(c).unwrap();
        s.entered = true;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        refresh(&mut socket, peer, &lobby, &mut s).await;
        assert_eq!(s.visible_entities.len(), 3);
        assert!(s.visible_entities.contains(&10000));
        let health = crate::monsters::Event {
            zone: 21,
            packet: world.health(10000, 21).unwrap(),
        };
        assert!(event_visible(&health, &lobby, &mut s));
        s.position.x = 500;
        refresh(&mut socket, peer, &lobby, &mut s).await;
        assert!(s.visible_entities.is_empty());
        assert!(!event_visible(&health, &lobby, &mut s));
        s.position = Position::default();
        let packet = world.spawns(21).remove(0);
        assert!(event_visible(
            &crate::monsters::Event { zone: 21, packet },
            &lobby,
            &mut s
        ));
        assert!(s.visible_entities.contains(&10000));
    }
    #[tokio::test]
    async fn native_missing_entity_request_resends_details_and_rejects_foreign_ids() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let world =
            std::sync::Arc::new(crate::monsters::World::load(&reko_common::game_dir()).unwrap());
        let expected = world.nearby_spawns(s.position).remove(0);
        lobby.monsters = Some(world);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut p = vec![0x1d, 3, 0];
        for id in [10000u16, 65535, 10000] {
            p.extend(id.to_le_bytes());
        }
        for _ in 0..2 {
            s.visible_entities.clear();
            request(&mut socket, peer, &lobby, &mut s, &p).await;
            use tokio::io::AsyncReadExt;
            let mut frame = vec![0; expected.len() + 7];
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.read_exact(&mut frame),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(&frame[4..7], &[0x1d, 1, 0]);
            assert_eq!(&frame[7..frame.len() - 2], &expected[2..]);
            assert_eq!(s.visible_entities.len(), 1);
        }
    }

    #[test]
    fn full_moradon_import_resolves_without_replacing_existing_fixtures_when_available() {
        let game = reko_common::game_dir();
        let path = std::path::Path::new("../../logs/moradon-import-test.db");
        if !game.join("Data/NPC_Looks.tbl").exists() || !path.exists() {
            return;
        }
        let db =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let definitions = load(&db, &game).unwrap();
        assert_eq!(definitions.len(), 533);
        assert_eq!(definitions.iter().filter(|s| s.monster).count(), 474);
        let original = crate::shops::load(&db, &game).unwrap();
        assert!(original.iter().any(|n| n.id == 11000));
        assert!(original.iter().any(|n| n.id == 11001));
        let npcs = crate::shops::load_population(&db, &game).unwrap();
        assert_eq!(npcs.len(), 59);
        for npc in &npcs {
            let packet = npc.spawn();
            let definition = definitions.iter().find(|s| s.id == npc.id).unwrap();
            assert_eq!(
                u16::from_le_bytes([packet[4], packet[5]]),
                definition.prototype
            );
            assert_eq!(
                u16::from_le_bytes([packet[6], packet[7]]),
                definition.appearance
            );
            assert_eq!(packet.len(), 42 + definition.name.len());
        }
        let world = crate::monsters::World::load(&game).unwrap();
        world.load_population(&db, &game).unwrap();
        assert_eq!(world.spawns(21).len(), 477);
        assert!(world.nearby_spawns(Position::default()).len() < 100);
    }
}
