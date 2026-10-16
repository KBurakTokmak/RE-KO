//! Persisted unspent attribute points and authoritative one-point allocation.

use crate::{characters::Character, lobby::Lobby, skills, world_entry::Session};
use rusqlite::{Connection, params};
use std::net::SocketAddr;
use tokio::net::TcpStream;

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS character_stat_points (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        remaining INTEGER NOT NULL CHECK(remaining BETWEEN 0 AND 65535),
        PRIMARY KEY(account,slot),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);",
    )
}

pub(crate) fn load(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<u16> {
    db.query_row("SELECT COALESCE((SELECT remaining FROM character_stat_points WHERE account=?1 AND slot=?2),0)", params![account,slot], |row| row.get(0))
}

fn allocate(
    db: &Connection,
    account: &str,
    character: &Character,
    kind: u8,
    amount: i16,
) -> rusqlite::Result<Option<(Character, u16)>> {
    if !(1..=5).contains(&kind) || amount != 1 {
        return Ok(None);
    }
    let tx = db.unchecked_transaction()?;
    let remaining = load(&tx, account, character.slot)?;
    let stats: Vec<u8> = tx.query_row(
        "SELECT stats FROM lobby_characters WHERE account=?1 AND slot=?2",
        params![account, character.slot],
        |row| row.get(0),
    )?;
    let mut next = character.clone();
    next.stats = stats
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let stat = &mut next.stats[usize::from(kind - 1)];
    if remaining == 0 || *stat == 255 {
        return Ok(None);
    }
    *stat += 1;
    tx.execute(
        "UPDATE lobby_characters SET stats=?3 WHERE account=?1 AND slot=?2",
        params![account, character.slot, next.stats.as_slice()],
    )?;
    tx.execute(
        "UPDATE character_stat_points SET remaining=remaining-1 WHERE account=?1 AND slot=?2",
        params![account, character.slot],
    )?;
    tx.commit()?;
    Ok(Some((next, remaining - 1)))
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    payload: &[u8],
) {
    if payload.len() != 4 || !session.entered || session.skills.hp == 0 {
        return;
    }
    let Some(catalog) = lobby.combat.as_ref() else {
        return;
    };
    let result = allocate(
        &lobby.db.lock().unwrap(),
        account,
        &session.character,
        payload[1],
        i16::from_le_bytes([payload[2], payload[3]]),
    );
    match result {
        Ok(Some((character, remaining))) => {
            let current = (session.skills.hp, session.skills.mana);
            session.character = character;
            session.stat_points = remaining;
            let Ok(mut base) =
                catalog.calculate(&session.character, &session.progress, &session.equipment)
            else {
                return;
            };
            base.weight = base
                .weight
                .saturating_add(catalog.bag_weight(&session.inventory));
            session.base_combat = base;
            crate::utility::refresh(session, lobby.skills.as_ref(), lobby.combat.as_ref());
            session
                .skills
                .set_limits(session.combat.hp, session.combat.mp);
            session.skills.hp = current.0.min(session.skills.max_hp);
            session.skills.mana = current.1.min(session.skills.max_mana);
            if skills::save(
                &lobby.db.lock().unwrap(),
                account,
                session.character.slot,
                &session.skills,
            )
            .is_err()
            {
                return;
            }
            let mut packet = vec![0x28, payload[1]];
            for value in [
                u16::from(session.character.stats[usize::from(payload[1] - 1)]),
                session.skills.max_hp,
                session.skills.max_mana,
                session.combat.attack,
                session.combat.max_weight,
            ] {
                packet.extend(value.to_le_bytes());
            }
            super::send_response(sock, peer, &None, &packet, "STAT_POINT").await;
            super::send_response(
                sock,
                peer,
                &None,
                &session.skills.hp_packet(0),
                "STAT_POINT_HEALTH",
            )
            .await;
            super::send_response(
                sock,
                peer,
                &None,
                &session.skills.mana_packet(),
                "STAT_POINT_MANA",
            )
            .await;
        }
        Ok(None) => tracing::info!(%peer,"stat allocation refused"),
        Err(error) => tracing::error!(%peer,%error,"stat allocation failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_preserves_budget_and_rejects_negative_or_overflow() {
        let db = Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "test").unwrap();
        crate::characters::set_nation(&db, "test", 1).unwrap();
        let c = Character {
            slot: 0,
            name: "Warrior".into(),
            race: 1,
            class: 106,
            face: 0,
            hair: 0,
            stats: [254, 65, 60, 50, 50],
        };
        db.execute(
            "INSERT INTO lobby_characters VALUES ('test',0,'Warrior',1,106,0,0,?1)",
            [c.stats.as_slice()],
        )
        .unwrap();
        db.execute("INSERT INTO character_stat_points VALUES ('test',0,2)", [])
            .unwrap();
        assert!(allocate(&db, "test", &c, 1, -1).unwrap().is_none());
        assert!(allocate(&db, "test", &c, 0, 1).unwrap().is_none());
        let (next, points) = allocate(&db, "TEST", &c, 1, 1).unwrap().unwrap();
        assert_eq!((next.stats[0], points), (255, 1));
        assert!(allocate(&db, "test", &c, 1, 1).unwrap().is_none());
        assert_eq!(load(&db, "test", 0).unwrap(), 1);
    }
}
