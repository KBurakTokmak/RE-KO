//! Map-object gate lists and the native zone-loading handshake.
use crate::{lobby::Lobby, movement, world_entry::Session};
use rusqlite::params;
use std::net::SocketAddr;
use tokio::net::TcpStream;

#[derive(Clone)]
pub(crate) struct Gate {
    source: movement::Position,
    object: u16,
    group: u16,
}
#[derive(serde::Deserialize)]
struct Warp {
    id: u16,
    name: String,
    text: String,
    coins: u32,
    zone: u8,
    x: f32,
    y: f32,
    z: f32,
    radius: f32,
    nation: u8,
}
fn near(a: movement::Position, b: movement::Position) -> bool {
    a.zone == b.zone
        && u32::from(a.x.abs_diff(b.x)).pow(2) + u32::from(a.z.abs_diff(b.z)).pow(2) <= 10000
}
pub(crate) fn is_gate(lobby: &Lobby, s: &Session, p: &[u8]) -> bool {
    if p.len() != 5 {
        return false;
    }
    let id = u16::from_le_bytes([p[1], p[2]]);
    lobby.db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM world_reference_rules WHERE table_name='K_OBJECTPOS' AND rule_id=?1 AND CAST(json_extract(data,'$.Type') AS INTEGER)=5)",[u32::from(s.position.zone)*100000+u32::from(id)],|r|r.get(0)).unwrap_or(false)
}
fn entries(db: &rusqlite::Connection, s: &Session, gate: &Gate) -> anyhow::Result<Vec<Warp>> {
    let mut q = db.prepare("SELECT data FROM world_reference_rules WHERE table_name='HISTORICAL_WARP' AND rule_id/100000=?1 ORDER BY rule_id")?;
    let rows = q.query_map([gate.source.zone], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        let w: Warp = serde_json::from_str(&row?)?;
        let configured: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM world_town_starts WHERE zone=?1)",
            [w.zone],
            |r| r.get(0),
        )?;
        if w.id / 10 == gate.group
            && configured
            && (w.nation == 0 || w.nation == s.character.nation())
            && (w.zone > 2 || w.zone == s.character.nation())
        {
            out.push(w);
        }
    }
    Ok(out)
}
fn open(lobby: &Lobby, s: &Session, p: &[u8]) -> anyhow::Result<(Gate, Vec<u8>)> {
    anyhow::ensure!(
        s.entered && s.skills.hp > 0 && !s.zone_loading && p.len() == 5,
        "gate unavailable"
    );
    let index = u16::from_le_bytes([p[1], p[2]]);
    let db = lobby.db.lock().unwrap();
    let text: String = db.query_row(
        "SELECT data FROM world_reference_rules WHERE table_name='K_OBJECTPOS' AND rule_id=?1",
        [u32::from(s.position.zone) * 100000 + u32::from(index)],
        |r| r.get(0),
    )?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let n = |key: &str| {
        v[key]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| v[key].as_f64())
            .unwrap_or(0.0)
    };
    anyhow::ensure!(
        n("Type") == 5.0 && (n("Belong") == 0.0 || n("Belong") == f64::from(s.character.nation())),
        "wrong nation or object"
    );
    let gate = Gate {
        source: movement::Position {
            zone: s.position.zone,
            x: (n("PosX") * 10.0) as u16,
            z: (n("PosZ") * 10.0) as u16,
            y: (n("PosY") * 10.0) as i16,
        },
        object: index,
        group: n("ControlNpcID") as u16,
    };
    anyhow::ensure!(near(s.position, gate.source), "gate out of range");
    let warps = entries(&db, s, &gate)?;
    anyhow::ensure!(!warps.is_empty(), "gate destinations unavailable");
    let mut packet = vec![0x4b, 1];
    packet.extend((warps.len() as u16).to_le_bytes());
    for w in warps {
        packet.extend(w.id.to_le_bytes());
        for text in [&w.name, &w.text] {
            packet.extend((text.len() as u16).to_le_bytes());
            packet.extend(text.as_bytes());
        }
        packet.extend(u16::from(w.zone).to_le_bytes());
        packet.extend(150u16.to_le_bytes());
        packet.extend(w.coins.to_le_bytes());
    }
    Ok((gate, packet))
}
fn select(
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) -> anyhow::Result<(Vec<u8>, u32)> {
    anyhow::ensure!(
        s.entered && s.skills.hp > 0 && !s.zone_loading && p.len() == 3,
        "invalid gate selection"
    );
    let gate = s
        .gate
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("open a gate first"))?;
    anyhow::ensure!(near(s.position, gate.source), "gate out of range");
    let id = u16::from_le_bytes([p[1], p[2]]);
    let db = lobby.db.lock().unwrap();
    let w = entries(&db, s, gate)?
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| anyhow::anyhow!("destination not offered"))?;
    let mut seed = crate::skills::now_ms()
        .wrapping_mul(6364136223846793005)
        .wrapping_add(u64::from(s.player_id));
    let mut offset = || {
        seed ^= seed >> 29;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed % (2 * w.radius.max(0.0) as u64 + 1)) as f32 - w.radius
    };
    let position = movement::Position {
        zone: w.zone,
        x: ((w.x + offset()) * 10.0).clamp(0.0, 65535.0) as u16,
        z: ((w.z + offset()) * 10.0).clamp(0.0, 65535.0) as u16,
        y: (w.y * 10.0) as i16,
    };
    let tx = db.unchecked_transaction()?;
    let coins = crate::starter::coins(&tx, account, s.character.slot)?;
    anyhow::ensure!(coins >= w.coins, "insufficient coins");
    anyhow::ensure!(
        tx.execute(
            "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
            params![account, s.character.slot, coins - w.coins]
        )? == 1,
        "wallet missing"
    );
    movement::save(&tx, account, s.character.slot, position)?;
    tx.commit()?;
    tracing::info!(
        account,
        object = gate.object,
        destination = w.zone,
        warp = id,
        cost = w.coins,
        "gate travel committed"
    );
    s.coins = coins - w.coins;
    Ok((begin(lobby, s, position), w.coins))
}

fn begin(lobby: &Lobby, s: &mut Session, position: movement::Position) -> Vec<u8> {
    let same_zone = s.position.zone == position.zone;
    s.position = position;
    s.zone_loading = !same_zone;
    s.entered = same_zone;
    s.gate = None;
    s.movement = movement::Validator::default();
    s.pending_cast = None;
    s.arrows.clear();
    s.magic_projectiles.clear();
    s.merchant = None;
    s.npc_dialog = None;
    s.anvil = None;
    s.sitting = false;
    s.visible_entities.clear();
    s.entity_region = None;
    if let Some(world) = &lobby.monsters {
        world.forget(s.player_id);
    }
    let mut packet = if same_zone {
        vec![0x1e]
    } else {
        vec![0x27, 3, position.zone, 0]
    };
    packet.extend(position.x.to_le_bytes());
    packet.extend(position.z.to_le_bytes());
    packet.extend(position.y.to_le_bytes());
    if !same_zone {
        packet.push(0);
    }
    packet
}

fn tile_crossing(a: movement::Position, b: movement::Position, x: u16, z: u16) -> Option<f64> {
    if a.zone != b.zone {
        return None;
    }
    let mut entry = 0.0f64;
    let mut exit = 1.0f64;
    for (origin, end, cell) in [(a.x, b.x, x), (a.z, b.z, z)] {
        let lo = f64::from(cell) * 40.0;
        let hi = lo + 40.0 - 0.000001;
        let origin = f64::from(origin);
        let delta = f64::from(end) - origin;
        if delta == 0.0 {
            if origin < lo || origin > hi {
                return None;
            }
        } else {
            let t1 = (lo - origin) / delta;
            let t2 = (hi - origin) / delta;
            entry = entry.max(t1.min(t2));
            exit = exit.min(t1.max(t2));
            if entry > exit {
                return None;
            }
        }
    }
    Some(entry)
}

fn movement_destination(
    lobby: &Lobby,
    s: &Session,
    from: movement::Position,
) -> anyhow::Result<Option<movement::Position>> {
    if !s.entered || s.zone_loading || s.skills.hp == 0 || from.zone != s.position.zone {
        return Ok(None);
    }
    let db = lobby.db.lock().unwrap();
    let min_x = from.x.min(s.position.x) / 40;
    let max_x = from.x.max(s.position.x) / 40;
    let min_z = from.z.min(s.position.z) / 40;
    let max_z = from.z.max(s.position.z) / 40;
    let mut q=db.prepare("SELECT data FROM world_reference_rules WHERE table_name='MAP_EVENT_TILE' AND rule_id BETWEEN ?1 AND ?2")?;
    let base = u32::from(from.zone) * 1000000;
    let rows = q.query_map(
        params![
            base + u32::from(min_x) * 1024 + u32::from(min_z),
            base + u32::from(max_x) * 1024 + u32::from(max_z)
        ],
        |r| r.get::<_, String>(0),
    )?;
    let mut first = None;
    for row in rows {
        let tile: serde_json::Value = serde_json::from_str(&row?)?;
        let x = tile["x"].as_u64().unwrap_or(1024) as u16;
        let z = tile["z"].as_u64().unwrap_or(1024) as u16;
        let Some(t) = tile_crossing(from, s.position, x, z) else {
            continue;
        };
        let event = tile["event"].as_u64().unwrap_or(0) as u32;
        let text: String = db.query_row(
            "SELECT data FROM world_reference_rules WHERE table_name='MAP_EVENT' AND rule_id=?1",
            [u32::from(from.zone) * 100000 + event],
            |r| r.get(0),
        )?;
        let action: serde_json::Value = serde_json::from_str(&text)?;
        let n = |key: &str| {
            action[key]
                .as_str()
                .and_then(|s| s.trim().parse::<u16>().ok())
                .unwrap_or(0)
        };
        let zone = n("Exec1") as u8;
        // War/event entry requires a running event controller.
        if n("Type") != 1 || (61..=69).contains(&zone) {
            continue;
        }
        let nation = match zone {
            1 | 11 => 1,
            2 | 12 => 2,
            _ => 0,
        };
        if nation != 0 && nation != s.character.nation() {
            continue;
        }
        if matches!(zone, 11 | 12) && s.progress.level < 60 {
            continue;
        }
        let configured: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM world_town_starts WHERE zone=?1)",
            [zone],
            |r| r.get(0),
        )?;
        if !configured {
            continue;
        }
        let destination = movement::Position {
            zone,
            x: n("Exec2")
                .checked_mul(10)
                .ok_or_else(|| anyhow::anyhow!("invalid event X"))?,
            z: n("Exec3")
                .checked_mul(10)
                .ok_or_else(|| anyhow::anyhow!("invalid event Z"))?,
            y: 0,
        };
        if first.as_ref().is_none_or(|&(time, _)| t < time) {
            first = Some((t, destination));
        }
    }
    Ok(first.map(|(_, p)| p))
}

pub(crate) async fn movement_event(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    from: movement::Position,
) -> bool {
    let result = movement_destination(lobby, s, from).and_then(|position| {
        if let Some(position) = position {
            movement::save(
                &lobby.db.lock().unwrap(),
                account,
                s.character.slot,
                position,
            )?;
        }
        Ok(position)
    });
    match result {
        Ok(Some(position)) => {
            tracing::info!(
                account,
                source = from.zone,
                destination = position.zone,
                x = position.x,
                z = position.z,
                "walk-through map event committed"
            );
            let packet = begin(lobby, s, position);
            super::send_response(sock, peer, &None, &packet, "MAP_EVENT_TELEPORT").await;
            if s.entered {
                crate::population::refresh(sock, peer, lobby, s).await;
            }
            true
        }
        Ok(None) => false,
        Err(error) => {
            tracing::error!(%error,"map movement event failed");
            false
        }
    }
}
pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) {
    let result = match p.first() {
        Some(0x33) => open(lobby, s, p).map(|(gate, packet)| {
            s.gate = Some(gate);
            (packet, 0)
        }),
        Some(0x4b) => select(lobby, account, s, p),
        Some(0x27) if s.zone_loading && p == [0x27, 1] => Ok((vec![0x27, 2], 0)),
        Some(0x27) if s.zone_loading && p == [0x27, 2] => {
            s.zone_loading = false;
            s.entered = true;
            crate::population::refresh(sock, peer, lobby, s).await;
            if let Some(world) = &lobby.monsters {
                world.observe(s, crate::skills::now_ms());
            }
            return;
        }
        _ => Err(anyhow::anyhow!("unexpected travel request")),
    };
    match result {
        Ok((packet, cost)) => {
            super::send_response(sock, peer, &None, &packet, "GATE_TRAVEL").await;
            if cost > 0 {
                let gold = crate::wallet::change(s.coins + cost, s.coins);
                super::send_response(sock, peer, &None, &gold, "GATE_COINS").await;
            }
            if p.first() == Some(&0x4b) && !s.zone_loading {
                super::send_response(sock, peer, &None, &[0x4b, 2, 1], "GATE_ARRIVED").await;
                crate::population::refresh(sock, peer, lobby, s).await;
            }
        }
        Err(error) => {
            tracing::info!(%peer,%error,"gate request refused");
            let reply = if p.first() == Some(&0x4b) {
                [0x4b, 2, 0]
            } else {
                [0x33, 5, 0]
            };
            super::send_response(sock, peer, &None, &reply, "GATE_FAILURE").await;
            let notice = crate::chat::packet(
                7,
                s.character.nation(),
                u16::MAX,
                "",
                error.to_string().as_bytes(),
            );
            super::send_response(sock, peer, &None, &notice, "GATE_REFUSED").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::AsyncReadExt, net::TcpListener};
    #[tokio::test]
    async fn eslant_walkthrough_uses_primary_action_and_intersects_fast_crossing() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        s.position = movement::Position {
            zone: 11,
            x: 4450,
            z: 5550,
            y: 0,
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO world_town_starts VALUES(1,354,1610,354,1610,0,0,'test')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO world_reference_rules VALUES('MAP_EVENT',1101131,?1)",
                [r#"{"Type":"1","Exec1":"1","Exec2":"1365","Exec3":"1841"}"#],
            )
            .unwrap();
            db.execute(
                "INSERT INTO world_reference_rules VALUES('MAP_EVENT_TILE',11113802,?1)",
                [r#"{"zone":11,"x":111,"z":138,"event":1131}"#],
            )
            .unwrap();
        }
        let from = s.position;
        assert_eq!(
            movement_destination(&lobby, &s, from)
                .unwrap()
                .unwrap()
                .zone,
            1
        );
        let outside = movement::Position {
            zone: 11,
            x: 4300,
            z: 5550,
            y: 0,
        };
        s.position.x = 4500;
        assert!(movement_destination(&lobby, &s, outside).unwrap().is_some());
        s.position.z = 5800;
        assert!(
            movement_destination(&lobby, &s, movement::Position { z: 5800, ..outside })
                .unwrap()
                .is_none()
        );
        s.position = from;
        s.character.race = 12;
        assert!(movement_destination(&lobby, &s, from).unwrap().is_none());
        s.character.race = 2;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut server, peer) = listener.accept().await.unwrap();
        assert!(movement_event(&mut server, peer, &lobby, "test", &mut s, from).await);
        assert_eq!(
            (s.position.zone, s.position.x, s.position.z),
            (1, 13650, 18410)
        );
        assert_eq!(s.coins, 1000000);
        assert!(s.zone_loading && !s.entered);
        let mut reply = [0; 17];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[4..8], &[0x27, 3, 1, 0]);
        assert!(!movement_event(&mut server, peer, &lobby, "test", &mut s, from).await);
    }
    #[tokio::test]
    async fn gates_scope_destinations_charge_once_and_complete_loading() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        s.position = movement::Position {
            zone: 21,
            x: 2820,
            z: 3740,
            y: 0,
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO world_town_starts VALUES(1,354,1610,354,1610,0,0,'test')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO world_town_starts VALUES(2,1670,370,1670,370,0,0,'test')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO world_reference_rules VALUES('K_OBJECTPOS',2104014,?1)",
                [r#"{"Type":"5","Belong":"1","PosX":"282","PosZ":"374","ControlNpcID":"211"}"#],
            )
            .unwrap();
            for (id, zone, nation) in [(2111, 1, 1), (2112, 2, 2), (2121, 1, 1), (2113, 201, 0)] {
                let v = serde_json::json!({"id":id,"name":"Castle","text":"Travel","coins":5000,"zone":zone,"x":445,"y":0,"z":1628,"radius":0,"nation":nation});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('HISTORICAL_WARP',?1,?2)",
                    params![2100000 + id, v.to_string()],
                )
                .unwrap();
            }
        }
        let request = [0x33, 0xae, 0x0f, 0, 0];
        assert!(is_gate(&lobby, &s, &request));
        let (gate, list) = open(&lobby, &s, &request).unwrap();
        assert_eq!(&list[..4], &[0x4b, 1, 1, 0]);
        assert_eq!(&list[4..6], &2111u16.to_le_bytes());
        s.gate = Some(gate);
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE character_wallets SET coins=0 WHERE account='test' AND slot=0",
                [],
            )
            .unwrap();
        assert!(select(&lobby, "test", &mut s, &[0x4b, 0x3f, 8]).is_err());
        assert_eq!(s.position.zone, 21);
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE character_wallets SET coins=1000000 WHERE account='test' AND slot=0",
                [],
            )
            .unwrap();
        assert!(select(&lobby, "test", &mut s, &[0x4b, 0x42, 8]).is_err());
        let coins = s.coins;
        let (packet, spent) = select(&lobby, "test", &mut s, &[0x4b, 0x3f, 8]).unwrap();
        assert_eq!(&packet[..4], &[0x27, 3, 1, 0]);
        assert_eq!((s.position.x, s.position.z), (4450, 16280));
        assert_eq!(spent, 5000);
        assert_eq!(s.coins, coins - 5000);
        assert!(s.zone_loading && !s.entered);
        assert!(select(&lobby, "test", &mut s, &[0x4b, 0x3f, 8]).is_err());
        assert_eq!(
            crate::starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            coins - 5000
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut server, peer) = listener.accept().await.unwrap();
        handle(&mut server, peer, &lobby, "test", &mut s, &[0x27, 1]).await;
        let mut reply = [0; 8];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, crate::frame_packet(&[0x27, 2]).as_slice());
        handle(&mut server, peer, &lobby, "test", &mut s, &[0x27, 2]).await;
        assert!(s.entered && !s.zone_loading);
        s.position = movement::Position {
            zone: 21,
            x: 2820,
            z: 3740,
            y: 0,
        };
        s.character.race = 12;
        assert!(open(&lobby, &s, &request).is_err());
        s.character.race = 2;
        s.position.x = 0;
        assert!(open(&lobby, &s, &request).is_err());
        s.position = movement::Position {
            zone: 1,
            x: 4450,
            z: 16280,
            y: 0,
        };
        s.gate = Some(Gate {
            source: s.position,
            object: 1,
            group: 11,
        });
        let row = serde_json::json!({"id":111,"name":"Village","text":"Travel","coins":100,"zone":1,"x":379,"y":0,"z":693,"radius":0,"nation":1});
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO world_reference_rules VALUES('HISTORICAL_WARP',100111,?1)",
                [row.to_string()],
            )
            .unwrap();
        let (packet, spent) = select(&lobby, "test", &mut s, &[0x4b, 111, 0]).unwrap();
        assert_eq!(packet[0], 0x1e);
        assert_eq!(spent, 100);
        assert!(s.entered && !s.zone_loading);
    }
}
