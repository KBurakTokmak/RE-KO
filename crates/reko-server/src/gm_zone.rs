//! Admin zone selection over configured source towns and original client maps.
use crate::{admin, chat, combat, lobby::Lobby, movement, travel, world_entry::Session};
use reko_assets::{CellValue, Table};
use std::net::SocketAddr;
use tokio::net::TcpStream;

struct Zone {
    id: u8,
    name: String,
    aliases: Vec<String>,
}

fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse(text: &str) -> Option<&str> {
    let (command, args) = text
        .trim()
        .split_once(char::is_whitespace)
        .unwrap_or((text.trim(), ""));
    command
        .trim_start_matches(['/', '+'])
        .eq_ignore_ascii_case("zonechange")
        .then_some(args.trim().trim_matches('"'))
}

fn catalog(lobby: &Lobby) -> anyhow::Result<Vec<Zone>> {
    let game = reko_common::game_dir();
    let native = Table::from_encrypted_bytes(&std::fs::read(game.join("Data/Zones.tbl"))?)?;
    let db = lobby.db.lock().unwrap();
    let mut query = db.prepare("SELECT t.zone,r.data FROM world_town_starts t LEFT JOIN world_reference_rules r ON r.table_name='ZONE_INFO' AND r.rule_id=t.zone ORDER BY t.zone")?;
    let mut zones = Vec::new();
    for row in query.query_map([], |r| {
        Ok((r.get::<_, u8>(0)?, r.get::<_, Option<String>>(1)?))
    })? {
        let (id, metadata) = row?;
        let Some(row) = native
            .rows
            .iter()
            .find(|r| combat::integer(r, 0) == i32::from(id) * 10)
        else {
            continue;
        };
        let Some(CellValue::String(asset)) = row.get(1) else {
            continue;
        };
        let asset = asset.replace('\\', "/");
        let asset = std::path::Path::new(asset.trim());
        if asset.is_absolute()
            || asset
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
            || !game.join(asset).is_file()
        {
            continue;
        }
        let source = metadata
            .map(|v| serde_json::from_str::<serde_json::Value>(&v))
            .transpose()?
            .and_then(|v| v["bz"].as_str().map(str::to_owned));
        let (name, aliases) = match id {
            1 => ("Luferson Castle".to_owned(), vec!["Karus".to_owned()]),
            2 => ("El Morad Castle".to_owned(), vec!["Elmorad".to_owned()]),
            21 => ("Moradon".to_owned(), vec!["Maradon".to_owned()]),
            71 => ("Colony Zone".to_owned(), vec!["Ronark Land".to_owned()]),
            72 => ("Ardream".to_owned(), vec![]),
            _ => (source.unwrap_or_else(|| format!("Zone {id}")), vec![]),
        };
        zones.push(Zone { id, name, aliases });
    }
    Ok(zones)
}

fn exact<'a>(zones: &'a [Zone], args: &str) -> Option<&'a Zone> {
    if let Ok(id) = args.parse::<u8>() {
        return zones.iter().find(|z| z.id == id);
    }
    let key = normalized(args);
    zones.iter().find(|z| {
        std::iter::once(&z.name)
            .chain(&z.aliases)
            .any(|n| normalized(n) == key)
    })
}

fn suggestions(zones: &[Zone], args: &str) -> Vec<String> {
    let key = normalized(args);
    let matches: Vec<_> = zones
        .iter()
        .filter(|z| {
            std::iter::once(&z.name)
                .chain(&z.aliases)
                .any(|n| normalized(n).contains(&key))
        })
        .map(|z| format!("{}: {}", z.id, z.name))
        .collect();
    if matches.is_empty() {
        return vec!["No matching zone. Type zonechange to list available IDs.".into()];
    }
    matches
        .chunks(6)
        .map(|rows| format!("zonechange <ID>: {}", rows.join("; ")))
        .collect()
}

fn destination(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    zone: u8,
) -> anyhow::Result<movement::Position> {
    let db = lobby.db.lock().unwrap();
    anyhow::ensure!(
        admin::is_admin(&db, account, s.character.slot)?,
        "Zonechange requires an admin character."
    );
    anyhow::ensure!(
        s.entered && !s.zone_loading && s.skills.hp > 0,
        "Enter the world alive before changing zones."
    );
    let (x,z): (u16,u16) = db.query_row("SELECT CASE WHEN ?2=1 THEN karus_x ELSE elmo_x END,CASE WHEN ?2=1 THEN karus_z ELSE elmo_z END FROM world_town_starts WHERE zone=?1", rusqlite::params![zone,s.character.nation()], |r| Ok((r.get(0)?,r.get(1)?)))?;
    Ok(movement::Position {
        zone,
        x: x.checked_mul(10)
            .ok_or_else(|| anyhow::anyhow!("Invalid town X"))?,
        z: z.checked_mul(10)
            .ok_or_else(|| anyhow::anyhow!("Invalid town Z"))?,
        y: 0,
    })
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    text: &[u8],
) -> bool {
    let text: String = text.iter().map(|b| char::from(*b)).collect();
    let Some(args) = parse(&text) else {
        return false;
    };
    let result = (|| -> anyhow::Result<(Option<movement::Position>, Vec<String>)> {
        anyhow::ensure!(
            admin::is_admin(&lobby.db.lock().unwrap(), account, s.character.slot)?,
            "Zonechange requires an admin character."
        );
        let zones = catalog(lobby)?;
        if let Some(zone) = exact(&zones, args) {
            return Ok((Some(destination(lobby, account, s, zone.id)?), vec![]));
        }
        Ok((None, suggestions(&zones, args)))
    })();
    let notices = match result {
        Ok((Some(position), _)) => {
            let save = movement::save(
                &lobby.db.lock().unwrap(),
                account,
                s.character.slot,
                position,
            );
            match save {
                Ok(()) => {
                    let packet = travel::begin(lobby, s, position);
                    super::send_response(sock, peer, &None, &packet, "ADMIN_ZONE_CHANGE").await;
                    crate::social::refresh_region(lobby, account, s);
                    if s.entered {
                        crate::population::refresh(sock, peer, lobby, s).await;
                    }
                    tracing::info!(account, zone = position.zone, "admin zone change committed");
                    vec![]
                }
                Err(e) => vec![format!("Zonechange could not save position: {e}")],
            }
        }
        Ok((None, notices)) => notices,
        Err(e) => vec![format!("Zonechange: {e}")],
    };
    for notice in notices {
        let packet = chat::packet(7, s.character.nation(), u16::MAX, "", notice.as_bytes());
        super::send_response(sock, peer, &None, &packet, "ADMIN_ZONE_NOTICE").await;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_interior_substrings_suggest_without_moving() {
        let zones = vec![
            Zone {
                id: 21,
                name: "Moradon".into(),
                aliases: vec!["Maradon".into()],
            },
            Zone {
                id: 71,
                name: "Colony Zone".into(),
                aliases: vec!["Ronark Land".into()],
            },
        ];
        for text in ["aradon", "marad", "radon"] {
            assert!(exact(&zones, text).is_none());
            assert!(suggestions(&zones, text)[0].contains("21: Moradon"));
        }
        for text in ["21", "MORADON", "maradon"] {
            assert_eq!(exact(&zones, text).unwrap().id, 21);
        }
        assert_eq!(exact(&zones, "ronark land").unwrap().id, 71);
        assert!(exact(&zones, "255").is_none());
        assert_eq!(parse("/zonechange aradon"), Some("aradon"));
        assert_eq!(parse("zonechange"), Some(""));
    }

    #[tokio::test]
    async fn native_chat_zone_change_saves_then_uses_loading_handshake_and_rechecks_privilege() {
        use tokio::{io::AsyncReadExt, net::TcpListener};
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let start = s.position;
        assert!(destination(&lobby, "test", &s, 71).is_err());
        {
            let db = lobby.db.lock().unwrap();
            db.execute("INSERT INTO character_privileges VALUES('test',0,1)", [])
                .unwrap();
            db.execute(
                "INSERT INTO world_town_starts VALUES(71,1380,1090,630,920,0,0,'test')",
                [],
            )
            .unwrap();
        }
        s.zone_loading = true;
        assert!(destination(&lobby, "test", &s, 71).is_err());
        s.zone_loading = false;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER no_save BEFORE INSERT ON character_positions BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
        assert!(handle(&mut socket, peer, &lobby, "test", &mut s, b"zonechange 71").await);
        assert_eq!(s.position, start);
        let mut header = [0; 4];
        client.read_exact(&mut header).await.unwrap();
        let mut notice = vec![0; usize::from(u16::from_le_bytes([header[2], header[3]])) + 2];
        client.read_exact(&mut notice).await.unwrap();
        lobby
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER no_save;")
            .unwrap();
        let mut account = Some("test".to_owned());
        let mut session = Some(s);
        let text = b"zonechange 71";
        let mut packet = vec![0x10, 1];
        packet.extend((text.len() as u16).to_le_bytes());
        packet.extend(text);
        crate::handle_game_payload(
            &mut socket,
            peer,
            &packet,
            &lobby,
            &mut account,
            &mut session,
        )
        .await;
        let s = session.as_mut().unwrap();
        assert_eq!(
            (s.position.zone, s.position.x, s.position.z),
            (71, 13800, 10900)
        );
        assert!(s.zone_loading && !s.entered);
        let mut reply = [0; 17];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[4..8], &[0x27, 3, 71, 0]);
        assert_eq!(
            movement::load(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            s.position
        );
        travel::handle(&mut socket, peer, &lobby, "test", s, &[0x27, 1]).await;
        travel::handle(&mut socket, peer, &lobby, "test", s, &[0x27, 2]).await;
        assert!(s.entered && !s.zone_loading);
        lobby
            .db
            .lock()
            .unwrap()
            .execute("UPDATE character_privileges SET is_admin=0", [])
            .unwrap();
        assert!(destination(&lobby, "test", s, 71).is_err());
    }
}
