//! Both native town controls use WIZ_HOME and the current zone's start row.
use crate::{chat, lobby::Lobby, movement, skills, world_entry::Session};
use rusqlite::OptionalExtension;
use std::net::SocketAddr;
use tokio::net::TcpStream;

fn destination(
    lobby: &Lobby,
    session: &Session,
    respawning: bool,
) -> Result<movement::Position, &'static str> {
    if !session.entered
        || (!respawning
            && (session.skills.hp == 0
                || u32::from(session.skills.hp) * 2 < u32::from(session.skills.max_hp)))
    {
        return Err("You need at least half your HP to return to town.");
    }
    if session.position.zone / 10 == 5 || matches!(session.position.zone, 84 | 85) {
        return Err("Town teleport is unavailable in this event zone.");
    }
    // TODO(re): Apply Kaul/freeze restrictions when those player states exist.
    let row: Option<(u16,u16,u16,u16,u16,u16)> = lobby.db.lock().unwrap().query_row(
        "SELECT karus_x,karus_z,elmo_x,elmo_z,range_x,range_z FROM world_town_starts WHERE zone=?1",[session.position.zone],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))
    ).optional().map_err(|_| "Town destination could not be loaded.")?;
    let (kx, kz, ex, ez, rx, rz) = row.ok_or("This zone has no configured town destination.")?;
    let (x, z) = if session.character.nation() == 1 {
        (kx, kz)
    } else {
        (ex, ez)
    };
    let random = skills::now_ms()
        .wrapping_mul(6364136223846793005)
        .wrapping_add(u64::from(session.player_id));
    let x = (u64::from(x) + random % (u64::from(rx) + 1)) * 10;
    let z = (u64::from(z) + random.rotate_left(31) % (u64::from(rz) + 1)) * 10;
    Ok(movement::Position {
        zone: session.position.zone,
        x: u16::try_from(x).map_err(|_| "Invalid town coordinates.")?,
        z: u16::try_from(z).map_err(|_| "Invalid town coordinates.")?,
        y: 0,
    })
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
) {
    let result = destination(lobby, session, false).and_then(|position| {
        movement::save(
            &lobby.db.lock().unwrap(),
            account,
            session.character.slot,
            position,
        )
        .map_err(|_| "Town position could not be saved.")?;
        Ok(position)
    });
    let position = match result {
        Ok(p) => p,
        Err(reason) => {
            let notice = chat::packet(
                7,
                session.character.nation(),
                u16::MAX,
                "",
                reason.as_bytes(),
            );
            super::send_response(sock, peer, &None, &notice, "TOWN_REFUSED").await;
            return;
        }
    };
    session.position = position;
    session.movement = movement::Validator::default();
    session.pending_cast = None;
    session.merchant = None;
    session.anvil = None;
    session.npc_dialog = None;
    session.sitting = false;
    let mut packet = vec![0x1e];
    packet.extend(position.x.to_le_bytes());
    packet.extend(position.z.to_le_bytes());
    super::send_response(sock, peer, &None, &packet, "TOWN_WARP").await;
    session.entity_region = None;
    crate::population::refresh(sock, peer, lobby, session).await;
    tracing::info!(%peer,account,x=position.x,z=position.z,zone=position.zone,"town teleport committed");
}

pub(crate) async fn respawn(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) {
    if !s.entered || s.skills.hp != 0 || p != [0x12, 1] {
        return;
    }
    let Ok(position) = destination(lobby, s, true) else {
        return;
    };
    let result = (|| -> rusqlite::Result<(skills::State, crate::combat::Stats)> {
        let db = lobby.db.lock().unwrap();
        let tx = db.unchecked_transaction()?;
        let mut next = s.skills.clone();
        next.buffs.clear();
        next.periodic.clear();
        let stats =
            crate::utility::effective(s, &next, lobby.skills.as_ref(), lobby.combat.as_ref());
        next.set_limits(stats.hp, stats.mp);
        next.hp = next.max_hp;
        skills::save_in(&tx, account, s.character.slot, &next)?;
        movement::save(&tx, account, s.character.slot, position)?;
        tx.commit()?;
        Ok((next, stats))
    })();
    let Ok((next, stats)) = result else {
        return;
    };
    s.skills = next;
    s.combat = stats;
    s.position = position;
    s.movement = movement::Validator::default();
    s.pending_cast = None;
    s.merchant = None;
    s.anvil = None;
    s.npc_dialog = None;
    s.sitting = false;
    let mut reply = vec![0x12];
    reply.extend(position.x.to_le_bytes());
    reply.extend(position.z.to_le_bytes());
    reply.extend(position.y.to_le_bytes());
    super::send_response(sock, peer, &None, &reply, "PLAYER_RESPAWN").await;
    super::send_response(sock, peer, &None, &s.skills.hp_packet(0), "RESPAWN_HP").await;
    super::send_response(
        sock,
        peer,
        &None,
        &crate::inventory::attributes(&s.combat, 2),
        "RESPAWN_ATTRIBUTES",
    )
    .await;
    super::send_response(sock, peer, &None, &s.skills.mana_packet(), "RESPAWN_MP").await;
    s.entity_region = None;
    crate::population::refresh(sock, peer, lobby, s).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::AsyncReadExt, net::TcpListener};
    #[tokio::test]
    async fn dead_player_respawns_at_reference_town_and_clears_buffs_atomically() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO world_town_starts VALUES(21,306,352,310,354,0,0,'test')",
                [],
            )
            .unwrap();
        s.skills.hp = 0;
        s.skills.mana = 50;
        s.skills.buffs.insert(
            6,
            skills::Buff {
                skill: 108725,
                until: skills::now_ms() + 60000,
            },
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        respawn(&mut socket, peer, &lobby, "test", &mut s, &[0x12, 1]).await;
        let mut reply = [0u8; 13];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[4..11], &[0x12, 0xf4, 0x0b, 0xc0, 0x0d, 0, 0]);
        assert_eq!(s.skills.hp, s.combat.hp);
        assert!(s.skills.buffs.is_empty());
        assert_eq!(s.skills.mana, 50);
        let db = lobby.db.lock().unwrap();
        assert_eq!(movement::load(&db, "test", 0).unwrap(), s.position);
        let saved = skills::load(&db, "test", 0).unwrap();
        assert_eq!(saved.hp, s.skills.hp);
        assert!(saved.buffs.is_empty());
    }

    #[tokio::test]
    async fn native_button_and_chat_town_share_validation_and_persist_position() {
        let lobby = Lobby::new(
            rusqlite::Connection::open_in_memory().unwrap(),
            crate::characters::CreationRules::fixture(),
            crate::world_entry::Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        let character = crate::characters::Character {
            slot: 0,
            name: "TownTest".into(),
            race: 2,
            class: 102,
            face: 0,
            hair: 0,
            stats: [60, 60, 80, 50, 50],
        };
        {
            let mut db = lobby.db.lock().unwrap();
            crate::characters::ensure_account(&db, "test").unwrap();
            crate::characters::set_nation(&db, "test", 1).unwrap();
            assert_eq!(
                crate::characters::create(&mut db, "test", &character).unwrap(),
                0
            );
            db.execute(
                "INSERT INTO world_town_starts VALUES (21,306,352,310,354,0,0,'test')",
                [],
            )
            .unwrap();
        }
        let mut s = Session::new(character).unwrap();
        s.entered = true;
        s.skills.hp = 100;
        s.skills.max_hp = 100;
        s.skills.mana = 77;
        s.merchant = Some(11000);
        s.sitting = true;
        s.pending_cast = Some(crate::magic::Cast {
            skill: 1,
            target: 1,
            expires_at: skills::now_ms() + 1000,
        });
        let mut world = Some(s);
        let mut account = Some("test".into());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        for request in [
            vec![0x48, 0],
            vec![0x10, 1, 5, 0, b'/', b't', b'o', b'w', b'n'],
        ] {
            world.as_mut().unwrap().position.x = 4500;
            crate::handle_game_payload(
                &mut socket,
                peer,
                &request,
                &lobby,
                &mut account,
                &mut world,
            )
            .await;
            let mut frame = [0; 11];
            client.read_exact(&mut frame).await.unwrap();
            assert_eq!(
                frame,
                [0xaa, 0x55, 5, 0, 0x1e, 0xf4, 0x0b, 0xc0, 0x0d, 0x55, 0xaa]
            );
            let s = world.as_ref().unwrap();
            assert_eq!(
                (s.position.x, s.position.z, s.skills.mana),
                (3060, 3520, 77)
            );
            assert!(s.merchant.is_none() && s.pending_cast.is_none() && !s.sitting);
            assert_eq!(
                movement::load(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
                s.position
            );
        }
        world.as_mut().unwrap().position.x = 4200;
        world.as_mut().unwrap().skills.hp = 49;
        crate::handle_game_payload(
            &mut socket,
            peer,
            &[0x48, 0],
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert_eq!(
            world.as_ref().unwrap().position.x,
            4200,
            "below half HP cannot town"
        );
        world.as_mut().unwrap().skills.hp = 50;
        assert_eq!(
            destination(&lobby, world.as_ref().unwrap(), false)
                .unwrap()
                .x,
            3060
        );
        let mut unauthorized = Session::new(world.as_ref().unwrap().character.clone()).unwrap();
        assert!(destination(&lobby, &unauthorized, false).is_err());
        unauthorized.entered = true;
        unauthorized.position.zone = 84;
        assert!(destination(&lobby, &unauthorized, false).is_err());
    }
}
