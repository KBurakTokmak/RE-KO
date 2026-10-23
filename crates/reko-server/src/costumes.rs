//! Permanent original-client Valkyrie equipment, separate from ordinary armour.
use crate::{combat, inventory, lobby::Lobby, starter, world_entry::Session};
use rusqlite::{Connection, params};

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Bonuses {
    pub hp: u16,
    pub defence: u16,
    pub experience: u16,
    pub coins: u16,
}

fn variant(id: u32) -> Option<(usize, usize)> {
    match id {
        508013318..=508013321 => Some((0, (id - 508013318) as usize)),
        508011441..=508011444 => Some((1, (id - 508011441) as usize)),
        _ => None,
    }
}

pub(crate) fn bonuses(items: &[starter::Item; 2]) -> Bonuses {
    let mut result = Bonuses::default();
    for (slot, item) in items.iter().enumerate() {
        if let Some((expected, choice)) = variant(item.id)
            && expected == slot
            && item.count == 1
        {
            let values = if slot == 0 {
                [100, 20, 2, 3]
            } else {
                [200, 30, 4, 6]
            };
            let target = match choice {
                0 => &mut result.hp,
                1 => &mut result.defence,
                2 => &mut result.experience,
                _ => &mut result.coins,
            };
            *target += values[choice];
        }
    }
    result
}

pub(crate) fn gain(amount: u32, percent: u16) -> u32 {
    (u64::from(amount) * (100 + u64::from(percent)) / 100).min(u64::from(u32::MAX)) as u32
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS character_costumes (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        costume_slot INTEGER NOT NULL CHECK(costume_slot BETWEEN 0 AND 1),
        item_id INTEGER NOT NULL, durability INTEGER NOT NULL,
        PRIMARY KEY(account,slot,costume_slot),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
    );",
    )
}

pub(crate) fn load(
    db: &Connection,
    account: &str,
    slot: u8,
) -> rusqlite::Result<[starter::Item; 2]> {
    let mut result = [starter::Item::default(); 2];
    let mut query = db.prepare("SELECT costume_slot,item_id,durability FROM character_costumes WHERE account=?1 AND slot=?2")?;
    for row in query.query_map(params![account, slot], |r| {
        Ok((
            r.get::<_, usize>(0)?,
            starter::Item {
                id: r.get(1)?,
                durability: r.get(2)?,
                count: 1,
            },
        ))
    })? {
        let (index, item) = row?;
        if variant(item.id).is_none_or(|(expected, _)| expected != index) || index >= 2 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        result[index] = item;
    }
    Ok(result)
}

fn index(native: u8) -> anyhow::Result<usize> {
    match native {
        1 => Ok(0),
        4 => Ok(1),
        _ => anyhow::bail!("unsupported costume slot"),
    }
}

fn move_item(lobby: &Lobby, account: &str, s: &Session, p: &[u8]) -> anyhow::Result<Session> {
    anyhow::ensure!(
        p.len() == 8 && p[0] == 0x1f && matches!(p[1], 7 | 8),
        "invalid costume move"
    );
    anyhow::ensure!(
        s.entered && !s.zone_loading && s.skills.hp > 0,
        "character cannot equip costumes"
    );
    let catalog = lobby
        .combat
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing item catalog"))?;
    let id = u32::from_le_bytes(p[2..6].try_into()?);
    let (bag_slot, slot) = if p[1] == 7 {
        (usize::from(p[6]), index(p[7])?)
    } else {
        (usize::from(p[7]), index(p[6])?)
    };
    anyhow::ensure!(bag_slot < 28, "invalid inventory slot");
    let mut db = lobby.db.lock().unwrap();
    let tx = db.transaction()?;
    let mut costumes = load(&tx, account, s.character.slot)?;
    let mut bag = starter::inventory(&tx, account, s.character.slot)?;
    let (from, to) = if p[1] == 7 {
        (bag[bag_slot], costumes[slot])
    } else {
        (costumes[slot], bag[bag_slot])
    };
    anyhow::ensure!(
        id != 0 && from.id == id && from.count == 1,
        "costume ownership mismatch"
    );
    let incoming = if p[1] == 7 { from } else { to };
    if incoming.id != 0 {
        anyhow::ensure!(
            incoming.count == 1
                && variant(incoming.id).is_some_and(|(expected, _)| expected == slot),
            "invalid costume swap"
        );
        anyhow::ensure!(
            catalog.item(incoming.id).is_some(),
            "unsupported costume item"
        );
        catalog.eligible(incoming.id, &s.character, s.progress.level)?;
    }
    std::mem::swap(&mut bag[bag_slot], &mut costumes[slot]);
    tx.execute(
        "DELETE FROM character_costumes WHERE account=?1 AND slot=?2",
        params![account, s.character.slot],
    )?;
    for (i, item) in costumes.iter().enumerate().filter(|(_, i)| i.id != 0) {
        tx.execute(
            "INSERT INTO character_costumes VALUES(?1,?2,?3,?4,?5)",
            params![account, s.character.slot, i, item.id, item.durability],
        )?;
    }
    tx.execute(
        "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
        params![account, s.character.slot, bag_slot],
    )?;
    let item = bag[bag_slot];
    if item.id != 0 {
        tx.execute(
            "INSERT INTO character_inventory VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                account,
                s.character.slot,
                bag_slot,
                item.id,
                item.durability,
                item.count
            ],
        )?;
    }
    let mut next = s.clone();
    next.costumes = costumes;
    next.inventory = bag;
    crate::durability::refresh(&mut next, catalog, lobby.skills.as_ref());
    next.skills.set_limits(next.combat.hp, next.combat.mp);
    crate::skills::save_in(&tx, account, s.character.slot, &next.skills)?;
    tx.commit()?;
    tracing::info!(
        account,
        item_id = id,
        direction = p[1],
        costume_slot = slot,
        "costume move committed"
    );
    Ok(next)
}

pub(crate) fn handle(lobby: &Lobby, account: &str, s: &mut Session, p: &[u8]) -> Vec<u8> {
    match move_item(lobby, account, s, p) {
        Ok(next) => {
            *s = next;
            inventory::attributes(&s.combat, 1)
        }
        Err(error) => {
            tracing::warn!(account,%error,"costume move refused");
            vec![0x1f, 0]
        }
    }
}

pub(crate) fn apply(stats: &mut combat::Stats, items: &[starter::Item; 2]) {
    let bonus = bonuses(items);
    stats.hp = stats.hp.saturating_add(bonus.hp).min(14000);
    stats.defence = stats.defence.saturating_add(bonus.defence).min(32767);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(direction: u8, id: u32, source: u8, destination: u8) -> Vec<u8> {
        let mut p = vec![0x1f, direction];
        p.extend(id.to_le_bytes());
        p.extend([source, destination]);
        p
    }

    #[tokio::test]
    async fn original_costumes_equip_swap_relog_and_ack_without_inventory_deltas() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let original = s.combat;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut account = Some("test".to_owned());
        for (slot, first, native) in [(0, 508013318, 1), (1, 508011441, 4)] {
            for choice in 0..4 {
                let id = first + choice;
                lobby
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "INSERT INTO character_inventory VALUES('test',0,0,?1,3000,1)",
                        [id],
                    )
                    .unwrap();
                s.inventory = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
                let before = s.costumes[slot];
                let mut world = Some(s);
                crate::handle_game_payload(
                    &mut socket,
                    peer,
                    &request(7, id, 0, native),
                    &lobby,
                    &mut account,
                    &mut world,
                )
                .await;
                s = world.unwrap();
                use tokio::io::AsyncReadExt;
                let mut frames = Vec::new();
                for _ in 0..4 {
                    let mut header = [0; 4];
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        client.read_exact(&mut header),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(&header[..2], &[0xaa, 0x55]);
                    let size = u16::from_le_bytes(header[2..4].try_into().unwrap()) as usize;
                    let mut packet = vec![0; size + 2];
                    client.read_exact(&mut packet).await.unwrap();
                    frames.push(packet[..size].to_vec());
                }
                assert_eq!(
                    frames.iter().map(|p| p[0]).collect::<Vec<_>>(),
                    [0x1f, 0x54, 0x17, 0x18]
                );
                assert_eq!(frames[0][1], 1);
                assert_eq!(s.costumes[slot].id, id);
                assert_eq!(s.inventory[0], before);
                let db = lobby.db.lock().unwrap();
                assert_eq!(load(&db, "test", 0).unwrap(), s.costumes);
                assert_eq!(starter::inventory(&db, "test", 0).unwrap(), s.inventory);
                drop(db);
                if choice == 1 {
                    assert_eq!(
                        s.combat.defence,
                        original.defence + if slot == 0 { 20 } else { 30 }
                    );
                }
                let mut hub = crate::social::Hub::default();
                hub.players.insert(
                    s.player_id,
                    crate::social::Player {
                        account: "test".into(),
                        s: s.clone(),
                    },
                );
                let appearance = hub.user_in(s.player_id);
                let tail = &appearance[appearance.len() - 15..];
                assert_eq!(&tail[..4], &s.costumes[1].id.to_le_bytes());
                assert_eq!(&tail[7..11], &s.costumes[0].id.to_le_bytes());
                assert_eq!(tail[14], s.position.zone);
                let bonus = bonuses(&s.costumes);
                assert_eq!(s.combat.hp, original.hp + bonus.hp);
                assert_eq!(s.combat.defence, original.defence + bonus.defence);
                let mut reloaded = s.clone();
                reloaded.costumes = load(&lobby.db.lock().unwrap(), "test", 0).unwrap();
                crate::durability::refresh(&mut reloaded, lobby.combat.as_ref().unwrap(), None);
                assert_eq!(reloaded.combat.hp, s.combat.hp);
                assert_eq!(reloaded.combat.defence, s.combat.defence);
                // Login has exactly two permanent 15-byte auxiliary records.
                let info = s.my_info(&lobby.bootstrap);
                let offset = info.len() - 9 - 28 * 19 - 30;
                assert_eq!(&info[offset..offset + 4], &s.costumes[0].id.to_le_bytes());
                assert_eq!(
                    &info[offset + 15..offset + 19],
                    &s.costumes[1].id.to_le_bytes()
                );
                assert_eq!(&info[offset + 8..offset + 15], &[0; 7]);
                lobby
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "DELETE FROM character_inventory WHERE account='test' AND slot=0",
                        [],
                    )
                    .unwrap();
            }
        }
        let id = s.costumes[1].id;
        assert_eq!(handle(&lobby, "test", &mut s, &request(8, id, 4, 0))[1], 1);
        assert_eq!(s.costumes[1].id, 0);
        assert_eq!(s.inventory[0].id, id);
    }

    #[test]
    fn failures_rollback_bag_costume_and_vitals_and_hp_stacks_without_healing() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let base = s.combat;
        s.skills.hp = 100;
        lobby.db.lock().unwrap().execute("INSERT INTO character_inventory VALUES('test',0,0,508011441,3000,1),('test',0,1,508013318,3000,1),('test',0,2,391010000,1,99)",[]).unwrap();
        s.inventory = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
        let bag = s.inventory;
        for p in [
            request(7, 508011441, 0, 1),
            request(7, 508011442, 0, 4),
            request(7, 508011441, 28, 4),
            request(7, 391010000, 2, 4),
            vec![0x1f, 7],
        ] {
            assert_eq!(handle(&lobby, "test", &mut s, &p), [0x1f, 0]);
            assert_eq!(s.inventory, bag);
        }
        assert_eq!(
            handle(&lobby, "other", &mut s, &request(7, 508011441, 0, 4)),
            [0x1f, 0]
        );
        lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_costume BEFORE INSERT ON character_skill_state BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
        assert_eq!(
            handle(&lobby, "test", &mut s, &request(7, 508011441, 0, 4)),
            [0x1f, 0]
        );
        assert_eq!(s.inventory, bag);
        assert_eq!(s.costumes, [starter::Item::default(); 2]);
        let db = lobby.db.lock().unwrap();
        assert_eq!(starter::inventory(&db, "test", 0).unwrap(), bag);
        assert_eq!(load(&db, "test", 0).unwrap(), s.costumes);
        db.execute_batch("DROP TRIGGER fail_costume;").unwrap();
        drop(db);
        for p in [request(7, 508011441, 0, 4), request(7, 508013318, 1, 1)] {
            assert_eq!(handle(&lobby, "test", &mut s, &p)[1], 1);
        }
        assert_eq!(s.combat.hp, base.hp + 300);
        assert_eq!(s.skills.hp, 100);
        assert_eq!(
            handle(&lobby, "test", &mut s, &request(8, 508011441, 4, 2)),
            [0x1f, 0]
        );
        s.skills.hp = s.combat.hp;
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO character_inventory VALUES('test',0,3,110110001,4000,1)",
                [],
            )
            .unwrap();
        assert_eq!(
            inventory::handle(
                &lobby,
                Some("test"),
                Some(&mut s),
                &request(1, 110110001, 3, 6)
            )[1],
            1
        );
        assert_eq!(s.skills.hp, base.hp + 300);
        assert_eq!(
            handle(&lobby, "test", &mut s, &request(8, 508011441, 4, 0))[1],
            1
        );
        let mut deletion = vec![0x3f, 2, 0];
        deletion.extend(508011441u32.to_le_bytes());
        assert_eq!(
            inventory::remove(&lobby, Some("test"), Some(&mut s), &deletion)[0],
            [0x3f, 1]
        );
        assert_eq!(s.combat.hp, base.hp + 100);
        assert_eq!(s.skills.hp, s.combat.hp);
        assert_eq!(
            load(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            s.costumes
        );
        assert_eq!(s.inventory[2].count, 99);
    }
}
