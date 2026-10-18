//! Admin same-zone warps, using displayed map coordinates.
use crate::{admin, chat, lobby::Lobby, movement::Position, town, world_entry::Session};
use std::net::SocketAddr;
use tokio::net::TcpStream;

fn parse(text: &str) -> Option<&str> {
    let text = text.trim();
    let (command, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let command = command.trim_start_matches(['/', '+']);
    matches!(command.to_ascii_lowercase().as_str(), "teleport" | "tp").then_some(args.trim())
}

fn destination(lobby: &Lobby, account: &str, s: &Session, args: &str) -> anyhow::Result<Position> {
    anyhow::ensure!(
        admin::is_admin(&lobby.db.lock().unwrap(), account, s.character.slot)?,
        "Teleport requires an admin character."
    );
    anyhow::ensure!(
        s.entered && !s.zone_loading && s.skills.hp > 0,
        "Enter the world alive before teleporting."
    );
    let coordinates = args
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    anyhow::ensure!(
        coordinates.len() == 2,
        "Usage: teleport <x> <y> (current zone)."
    );
    let coordinate = |text: &str| -> anyhow::Result<u16> {
        let value: f64 = text.parse()?;
        anyhow::ensure!(
            value.is_finite() && (0.0..=6553.5).contains(&value),
            "Coordinates must be between 0 and 6553.5."
        );
        Ok((value * 10.0).round() as u16)
    };
    Ok(Position {
        zone: s.position.zone,
        x: coordinate(coordinates[0])?,
        z: coordinate(coordinates[1])?,
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
    let result = match destination(lobby, account, s, args) {
        Ok(position) => town::relocate(sock, peer, lobby, account, s, position)
            .await
            .map_err(anyhow::Error::from),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        let message = format!("Teleport: {error}");
        let packet = chat::packet(7, s.character.nation(), u16::MAX, "", message.as_bytes());
        super::send_response(sock, peer, &None, &packet, "ADMIN_TELEPORT_REFUSED").await;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::AsyncReadExt, net::TcpListener};

    #[tokio::test]
    async fn admin_chat_warp_is_persisted_refreshes_region_and_rejects_invalid_or_unprivileged_moves()
     {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let start = s.position;
        assert!(destination(&lobby, "test", &s, "445 555").is_err());
        lobby
            .db
            .lock()
            .unwrap()
            .execute("INSERT INTO character_privileges VALUES ('test',0,1)", [])
            .unwrap();
        for input in [
            "445",
            "445 555 666",
            "NaN 555",
            "inf 555",
            "-1 555",
            "6554 555",
            "no 555",
        ] {
            assert!(destination(&lobby, "test", &s, input).is_err(), "{input}");
        }
        for input in ["teleport 445 555", "/teleport 445 555", "+tp 445 555"] {
            assert_eq!(parse(input), Some("445 555"));
        }
        assert_eq!(parse("ordinary chat"), None);
        s.zone_loading = true;
        assert!(destination(&lobby, "test", &s, "445 555").is_err());
        s.zone_loading = false;
        s.skills.hp = 0;
        assert!(destination(&lobby, "test", &s, "445 555").is_err());
        s.skills.hp = s.skills.max_hp;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let target = destination(&lobby, "test", &s, "445 555").unwrap();
        lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_teleport BEFORE INSERT ON character_positions BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
        assert!(
            town::relocate(&mut socket, peer, &lobby, "test", &mut s, target)
                .await
                .is_err()
        );
        assert_eq!(s.position, start, "failed save cannot move the character");
        lobby
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_teleport;")
            .unwrap();
        // Exercise the actual All-chat dispatcher and displayed-coordinate conversion.
        let text = b"teleport 445.5,555";
        let mut packet = vec![0x10, 1];
        packet.extend((text.len() as u16).to_le_bytes());
        packet.extend(text);
        s.sitting = true;
        let mut account = Some("test".to_owned());
        let mut session = Some(s);
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
            s.position,
            Position {
                zone: start.zone,
                x: 4455,
                z: 5550,
                y: 0
            }
        );
        assert!(!s.sitting);
        assert!(s.entity_region.is_some());
        let mut header = [0u8; 4];
        client.read_exact(&mut header).await.unwrap();
        assert_eq!(u16::from_le_bytes([header[2], header[3]]), 5);
        let mut payload = [0u8; 5];
        client.read_exact(&mut payload).await.unwrap();
        assert_eq!(payload, [0x1e, 0x67, 0x11, 0xae, 0x15]);
        let db = lobby.db.lock().unwrap();
        assert_eq!(crate::movement::load(&db, "test", 0).unwrap(), s.position);
        db.execute("UPDATE character_privileges SET is_admin=0", [])
            .unwrap();
        drop(db);
        assert!(destination(&lobby, "test", s, "320 400").is_err());
    }
}
