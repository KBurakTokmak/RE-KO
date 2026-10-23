//! Persistent item moves with an acknowledgement for every request.

use crate::{combat, lobby::Lobby, starter, world_entry::Session};
use rusqlite::{Connection, params};

pub(crate) fn attributes(stats: &combat::Stats, result: u8) -> Vec<u8> {
    let mut packet = vec![0x1f, result];
    for value in [
        stats.attack,
        stats.defence,
        stats.max_weight,
        stats.hp,
        stats.mp,
    ] {
        packet.extend(value.to_le_bytes());
    }
    for value in stats.bonus_stats {
        packet.extend(value.to_le_bytes());
    }
    for value in stats.resistance {
        packet.extend(value.to_le_bytes());
    }
    packet
}

fn place_equipment(
    gear: &mut [starter::Item; 14],
    incoming: starter::Item,
    destination: usize,
    catalog: &combat::Catalog,
) -> anyhow::Result<starter::Item> {
    anyhow::ensure!(incoming.count == 1, "equipment must be a single item");
    let footprint = catalog
        .equipment_mask(incoming.id, destination)
        .ok_or_else(|| anyhow::anyhow!("invalid equipment destination"))?;
    let mut displaced = None;
    for (slot, item) in gear.iter().enumerate().filter(|(_, item)| item.id != 0) {
        let occupied = catalog
            .equipment_mask(item.id, slot)
            .ok_or_else(|| anyhow::anyhow!("invalid existing equipment"))?;
        if footprint & occupied != 0 {
            anyhow::ensure!(
                displaced.is_none(),
                "more than one equipped item would be displaced"
            );
            displaced = Some(slot);
        }
    }
    let returned = displaced.map_or(starter::Item::default(), |slot| {
        std::mem::take(&mut gear[slot])
    });
    gear[destination] = incoming;
    Ok(returned)
}

fn validate_equipment(gear: &[starter::Item; 14], catalog: &combat::Catalog) -> anyhow::Result<()> {
    let mut occupied = 0;
    for (slot, item) in gear.iter().enumerate().filter(|(_, item)| item.id != 0) {
        anyhow::ensure!(item.count == 1, "equipment must be a single item");
        let mask = catalog
            .equipment_mask(item.id, slot)
            .ok_or_else(|| anyhow::anyhow!("invalid equipment slot or item"))?;
        anyhow::ensure!(occupied & mask == 0, "equipment occupies conflicting slots");
        occupied |= mask;
    }
    Ok(())
}

fn move_items(
    db: &mut Connection,
    account: &str,
    session: &Session,
    catalog: &combat::Catalog,
    payload: &[u8],
) -> anyhow::Result<([starter::Item; 14], [starter::Item; 28], combat::Stats)> {
    anyhow::ensure!(
        payload.len() == 8 && payload[0] == 0x1f,
        "invalid item move"
    );
    let direction = payload[1];
    let id = u32::from_le_bytes(payload[2..6].try_into()?);
    let source = usize::from(payload[6]);
    let destination = usize::from(payload[7]);
    let tx = db.transaction()?;
    let mut gear = starter::equipment(&tx, account, session.character.slot)?;
    let mut bag = starter::inventory(&tx, account, session.character.slot)?;
    let (from, to) = match direction {
        1 => (bag.get(source), gear.get(destination)),
        2 => (gear.get(source), bag.get(destination)),
        3 => (bag.get(source), bag.get(destination)),
        4 => (gear.get(source), gear.get(destination)),
        _ => (None, None),
    };
    let (Some(&from), Some(&to)) = (from, to) else {
        anyhow::bail!("invalid slots");
    };
    anyhow::ensure!(id != 0 && from.id == id, "item ownership mismatch");
    if matches!(direction, 1 | 4) {
        catalog.eligible(from.id, &session.character, session.progress.level)?;
        anyhow::ensure!(
            catalog.equip_slot(from.id, destination),
            "invalid equipment destination"
        );
    }
    if matches!(direction, 2 | 4) && to.id != 0 {
        catalog.eligible(to.id, &session.character, session.progress.level)?;
        anyhow::ensure!(catalog.equip_slot(to.id, source), "invalid equipment swap");
    }
    match direction {
        1 => {
            bag[source] = place_equipment(&mut gear, from, destination, catalog)?;
        }
        2 => {
            gear[source] = to;
            bag[destination] = from;
        }
        3 => bag.swap(source, destination),
        4 => gear.swap(source, destination),
        _ => unreachable!(),
    }
    validate_equipment(&gear, catalog)?;
    let mut stats = catalog.calculate(&session.character, &session.progress, &gear)?;
    stats.weight = stats.weight.saturating_add(catalog.bag_weight(&bag));
    for (table, column, items) in [
        ("character_equipment", "equipment_slot", gear.as_slice()),
        ("character_inventory", "inventory_slot", bag.as_slice()),
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE account=?1 AND slot=?2"),
            params![account, session.character.slot],
        )?;
        for (slot, item) in items.iter().enumerate().filter(|(_, item)| item.id != 0) {
            if table == "character_inventory" {
                tx.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?1,?2,?3,?4,?5,?6)",params![account,session.character.slot,slot,item.id,item.durability,item.count])?;
            } else {
                tx.execute(&format!("INSERT INTO {table}(account,slot,{column},item_id,durability) VALUES (?1,?2,?3,?4,?5)"),params![account,session.character.slot,slot,item.id,item.durability])?;
            }
        }
    }
    tx.commit()?;
    Ok((gear, bag, stats))
}

pub(crate) fn consume(
    db: &Connection,
    account: &str,
    slot: u8,
    id: u32,
    quantity: u16,
) -> rusqlite::Result<Vec<(usize, starter::Item)>> {
    if id == 0 || quantity == 0 {
        return Ok(Vec::new());
    }
    let bag = starter::inventory(db, account, slot)?;
    if bag
        .iter()
        .filter(|item| item.id == id)
        .map(|item| u32::from(item.count))
        .sum::<u32>()
        < u32::from(quantity)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut remaining = quantity;
    let mut changed = Vec::new();
    for (index, mut item) in bag
        .into_iter()
        .enumerate()
        .filter(|(_, item)| item.id == id)
    {
        let taken = remaining.min(item.count);
        item.count -= taken;
        remaining -= taken;
        if item.count == 0 {
            db.execute("DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",params![account,slot,index])?;
        } else {
            db.execute("UPDATE character_inventory SET count=?4 WHERE account=?1 AND slot=?2 AND inventory_slot=?3",params![account,slot,index,item.count])?;
        }
        changed.push((index, item));
        if remaining == 0 {
            break;
        }
    }
    Ok(changed)
}

pub(crate) fn count_packet(changed: &[(usize, starter::Item)]) -> Vec<u8> {
    let mut packet = vec![0x3d];
    packet.extend((changed.len() as u16).to_le_bytes());
    for (index, item) in changed {
        packet.extend([1, *index as u8]);
        packet.extend(item.id.to_le_bytes());
        packet.extend(u32::from(item.count).to_le_bytes());
        packet.push(0);
        packet.extend(item.durability.to_le_bytes());
        packet.extend([0; 8]);
    }
    packet
}

pub(crate) fn reward_packet(
    before: &[starter::Item; 28],
    after: &[starter::Item; 28],
) -> Option<Vec<u8>> {
    let mut changes = Vec::new();
    let mut received = Vec::new();
    for (index, (&old, &item)) in before.iter().zip(after).enumerate() {
        if old == item {
            continue;
        }
        // Native replacement leaves a new item without its icon. Remove the old
        // item first so the following entry takes the complete creation path.
        if old.id != 0 && item.id != 0 && old.id != item.id {
            changes.push((index, starter::Item { count: 0, ..old }));
            received.push(false);
        }
        changes.push((index, item));
        received.push(item.id != 0 && (old.id != item.id || item.count > old.count));
    }
    if changes.is_empty() {
        return None;
    }
    let mut packet = count_packet(&changes);
    for (entry, new) in received.into_iter().enumerate() {
        packet[3 + entry * 21 + 10] = if new { 100 } else { 0 };
    }
    Some(packet)
}

pub(crate) fn handle(
    lobby: &Lobby,
    account: Option<&str>,
    session: Option<&mut Session>,
    payload: &[u8],
) -> Vec<u8> {
    let (Some(account), Some(session), Some(catalog)) = (
        account,
        session.filter(|s| s.entered),
        lobby.combat.as_ref(),
    ) else {
        return vec![0x1f, 0];
    };
    if payload
        .get(1)
        .is_some_and(|direction| matches!(direction, 7 | 8))
    {
        return crate::costumes::handle(lobby, account, session, payload);
    }
    let result = move_items(
        &mut lobby.db.lock().unwrap(),
        account,
        session,
        catalog,
        payload,
    );
    match result {
        Ok((gear, bag, stats)) => {
            session.equipment = gear;
            session.inventory = bag;
            session.combat = stats;
            session.base_combat = stats;
            crate::utility::refresh(session, lobby.skills.as_ref(), lobby.combat.as_ref());
            session
                .skills
                .set_limits(session.combat.hp, session.combat.mp);
            if let Err(error) = crate::skills::save(
                &lobby.db.lock().unwrap(),
                account,
                session.character.slot,
                &session.skills,
            ) {
                tracing::error!(%error,"equipment vitals save failed");
            }
            tracing::info!(account,name=%session.character.name,direction=payload[1],source=payload[6],destination=payload[7],"item move committed");
            attributes(&session.combat, 1)
        }
        Err(error) => {
            tracing::warn!(account,%error,"item move refused");
            vec![0x1f, 0]
        }
    }
}

pub(crate) fn remove(
    lobby: &Lobby,
    account: Option<&str>,
    session: Option<&mut Session>,
    payload: &[u8],
) -> Vec<Vec<u8>> {
    let (Some(account), Some(session), Some(catalog)) = (
        account,
        session.filter(|s| s.entered),
        lobby.combat.as_ref(),
    ) else {
        return vec![vec![0x3f, 0]];
    };
    let before = (
        session.equipment,
        session.inventory,
        session.base_combat,
        session.combat,
        session.skills.clone(),
    );
    let result = (|| -> anyhow::Result<()> {
        anyhow::ensure!(
            payload.len() == 7 && payload[0] == 0x3f,
            "invalid item removal"
        );
        anyhow::ensure!(session.skills.hp > 0, "character is dead");
        let index = usize::from(payload[2]);
        let id = u32::from_le_bytes(payload[3..7].try_into()?);
        let mut db = lobby.db.lock().unwrap();
        let tx = db.transaction()?;
        let mut gear = starter::equipment(&tx, account, session.character.slot)?;
        let mut bag = starter::inventory(&tx, account, session.character.slot)?;
        let (item, table, column) = match payload[1] {
            1 => (gear.get_mut(index), "character_equipment", "equipment_slot"),
            0 | 2 => (bag.get_mut(index), "character_inventory", "inventory_slot"),
            _ => anyhow::bail!("invalid item district"),
        };
        let item = item.ok_or_else(|| anyhow::anyhow!("invalid item slot"))?;
        anyhow::ensure!(id != 0 && item.id == id, "item ownership mismatch");
        *item = starter::Item::default();
        let changed = tx.execute(
            &format!(
                "DELETE FROM {table} WHERE account=?1 AND slot=?2 AND {column}=?3 AND item_id=?4"
            ),
            params![account, session.character.slot, index, id],
        )?;
        anyhow::ensure!(changed == 1, "missing item");
        session.equipment = gear;
        session.inventory = bag;
        let mut base = catalog.calculate(&session.character, &session.progress, &gear)?;
        base.weight = base.weight.saturating_add(catalog.bag_weight(&bag));
        session.base_combat = base;
        crate::utility::refresh(session, lobby.skills.as_ref(), Some(catalog));
        session
            .skills
            .set_limits(session.combat.hp, session.combat.mp);
        crate::skills::save_in(&tx, account, session.character.slot, &session.skills)?;
        tx.commit()?;
        tracing::info!(account, name=%session.character.name, district=payload[1], index, item_id=id, "item removal committed");
        Ok(())
    })();
    if let Err(error) = result {
        (
            session.equipment,
            session.inventory,
            session.base_combat,
            session.combat,
            session.skills,
        ) = before;
        tracing::warn!(account, %error, "item removal refused");
        return vec![vec![0x3f, 0]];
    }
    // The acknowledgement releases the client's pending icon before other updates.
    let mut weight = vec![0x54];
    weight.extend(session.combat.weight.to_le_bytes());
    let mut packets = vec![vec![0x3f, 1], weight];
    if payload[1] == 1 {
        packets.push(attributes(&session.combat, 2));
        packets.push(session.skills.hp_packet_from(0xffff));
        packets.push(session.skills.mana_packet());
    }
    packets
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_acknowledges_native_request_and_preserves_unrelated_state() {
        let Some((lobby, mut session)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO character_inventory VALUES ('test',0,13,205001001,100,7)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO character_inventory VALUES ('test',0,14,391010000,1,99)",
                [],
            )
            .unwrap();
        }
        let request = [0x3f, 2, 13, 0x29, 0x11, 0x38, 0x0c];
        for invalid in [
            vec![0x3f],
            vec![0x3f, 3, 13, 0x29, 0x11, 0x38, 0x0c],
            vec![0x3f, 2, 28, 0x29, 0x11, 0x38, 0x0c],
            vec![0x3f, 2, 13, 0x28, 0x11, 0x38, 0x0c],
        ] {
            assert_eq!(
                remove(&lobby, Some("test"), Some(&mut session), &invalid),
                vec![vec![0x3f, 0]]
            );
            assert_eq!(
                starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[13].count,
                7
            );
        }
        assert_eq!(
            remove(&lobby, None, Some(&mut session), &request),
            vec![vec![0x3f, 0]]
        );
        let packets = remove(&lobby, Some("test"), Some(&mut session), &request);
        assert_eq!(packets[0], [0x3f, 1]);
        assert_eq!(packets[1][0], 0x54);
        assert!(!packets.iter().any(|p| p[0] == 0x3d));
        let db = lobby.db.lock().unwrap();
        let bag = starter::inventory(&db, "test", 0).unwrap();
        assert_eq!(bag, session.inventory);
        assert_eq!(bag[13], starter::Item::default());
        assert_eq!((bag[14].id, bag[14].count), (391010000, 99));
        assert_eq!(starter::coins(&db, "test", 0).unwrap(), 1000000);
        drop(db);
        assert_eq!(
            remove(&lobby, Some("test"), Some(&mut session), &request),
            vec![vec![0x3f, 0]]
        );
    }

    #[test]
    fn equipped_deletion_refreshes_stats_and_failed_commit_restores_session() {
        let Some((lobby, mut session)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO character_equipment VALUES ('test',0,6,110110001,4000)",
                [],
            )
            .unwrap();
            session.equipment = starter::equipment(&db, "test", 0).unwrap();
        }
        crate::durability::refresh(&mut session, lobby.combat.as_ref().unwrap(), None);
        let before_attack = session.combat.attack;
        let mut request = vec![0x3f, 1, 6];
        request.extend(110110001u32.to_le_bytes());
        lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_delete BEFORE INSERT ON character_skill_state BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert_eq!(
            remove(&lobby, Some("test"), Some(&mut session), &request),
            vec![vec![0x3f, 0]]
        );
        assert_eq!(session.equipment[6].id, 110110001);
        assert_eq!(session.combat.attack, before_attack);
        assert_eq!(
            starter::equipment(&lobby.db.lock().unwrap(), "test", 0).unwrap()[6].id,
            110110001
        );
        lobby
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_delete;")
            .unwrap();
        let packets = remove(&lobby, Some("test"), Some(&mut session), &request);
        assert_eq!(packets[0], [0x3f, 1]);
        assert_eq!(packets[2], attributes(&session.combat, 2));
        assert_eq!(session.equipment[6].id, 0);
        assert!(session.combat.attack < before_attack);
        let db = lobby.db.lock().unwrap();
        assert_eq!(
            starter::equipment(&db, "test", 0).unwrap(),
            session.equipment
        );
        let saved = crate::skills::load(&db, "test", 0).unwrap();
        assert_eq!(
            (saved.hp, saved.mana),
            (session.skills.hp, session.skills.mana)
        );
    }
    #[test]
    fn attachment_footprints_cover_equipment_pairs() {
        let game = reko_common::game_dir();
        let rules = std::path::Path::new("../../logs/reference-coefficients.csv");
        if !game.join("Data/item_org_us.tbl").exists() || !rules.exists() {
            return;
        }
        let catalog = combat::Catalog::load(&game, rules).unwrap();
        let table = reko_assets::Table::from_encrypted_bytes(
            &std::fs::read(game.join("Data/item_org_us.tbl")).unwrap(),
        )
        .unwrap();
        let mut representatives = std::collections::BTreeMap::new();
        for row in &table.rows {
            let attachment = combat::integer(row, 12);
            let id = combat::integer(row, 0) as u32;
            if let Some(id) = [id, id + 1]
                .into_iter()
                .find(|id| (0..14).any(|slot| catalog.equip_slot(*id, slot)))
            {
                representatives.entry(attachment).or_insert(id);
            }
        }
        let expected: [&[usize]; 15] = [
            &[6, 8],
            &[6],
            &[8],
            &[6],
            &[8],
            &[4],
            &[10],
            &[1],
            &[12],
            &[13],
            &[0, 2],
            &[3],
            &[9, 11],
            &[5],
            &[7],
        ];
        for (&attachment, &id) in &representatives {
            let accepted: Vec<_> = (0..14)
                .filter(|&slot| catalog.equip_slot(id, slot))
                .collect();
            assert_eq!(
                accepted, expected[attachment as usize],
                "attachment {attachment}, item {id}"
            );
        }
        // No ordinary shoulder/cape prototype in this install's item table.
        for attachment in (0..=14).filter(|attachment| *attachment != 13) {
            assert!(
                representatives.contains_key(&attachment),
                "attachment {attachment} lacks a test item"
            );
        }
        let candidates: Vec<_> = representatives
            .values()
            .flat_map(|&id| {
                let catalog = &catalog;
                (0..14).filter_map(move |slot| {
                    catalog
                        .equipment_mask(id, slot)
                        .map(|mask| (id, slot, mask))
                })
            })
            .collect();
        for &(existing_id, existing_slot, existing_mask) in &candidates {
            for &(incoming_id, destination, incoming_mask) in &candidates {
                let existing = starter::Item {
                    id: existing_id,
                    durability: 137,
                    count: 1,
                };
                let incoming = starter::Item {
                    id: incoming_id,
                    durability: 251,
                    count: 1,
                };
                let mut gear = [starter::Item::default(); 14];
                gear[existing_slot] = existing;
                let returned = place_equipment(&mut gear, incoming, destination, &catalog).unwrap();
                validate_equipment(&gear, &catalog).unwrap();
                assert_eq!(gear[destination], incoming);
                if existing_mask & incoming_mask != 0 {
                    assert_eq!(returned, existing);
                    assert_eq!(gear.iter().filter(|item| item.id != 0).count(), 1);
                } else {
                    assert_eq!(returned, starter::Item::default());
                    assert_eq!(gear[existing_slot], existing);
                }
            }
        }
        let mut gear = [starter::Item::default(); 14];
        let stacked = starter::Item {
            id: 110110001,
            durability: 137,
            count: 2,
        };
        assert!(place_equipment(&mut gear, stacked, 6, &catalog).is_err());
        assert_eq!(gear, [starter::Item::default(); 14]);
        assert!(catalog.equipment_mask(110110999, 6).is_none());
    }

    #[tokio::test]
    async fn pants_both_rings_and_move_ack_follow_native_slots() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute("INSERT INTO character_inventory VALUES ('test',0,0,246002001,2000,1),('test',0,1,330110002,1,1),('test',0,2,330110002,1,1)",[]).unwrap();
            s.inventory = starter::inventory(&db, "test", 0).unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut account = Some("test".into());
        let mut world = Some(s);
        use tokio::io::AsyncReadExt;
        for (id, source, dest) in [
            (246002001u32, 0u8, 10u8),
            (330110002, 1, 9),
            (330110002, 2, 11),
        ] {
            let mut p = vec![0x1f, 1];
            p.extend(id.to_le_bytes());
            p.extend([source, dest]);
            crate::handle_game_payload(&mut socket, peer, &p, &lobby, &mut account, &mut world)
                .await;
            let mut ack = [0u8; 40];
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.read_exact(&mut ack),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(&ack[4..6], &[0x1f, 1]);
            let mut unexpected = [0u8; 1];
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(30),
                    client.read_exact(&mut unexpected)
                )
                .await
                .is_err(),
                "item move must use the native recovery acknowledgement only"
            );
            let s = world.as_ref().unwrap();
            assert_eq!(s.equipment[dest as usize].id, id);
            assert_eq!(s.inventory[source as usize].id, 0);
        }
        let db = lobby.db.lock().unwrap();
        let gear = starter::equipment(&db, "test", 0).unwrap();
        assert_eq!(
            (gear[10].id, gear[9].id, gear[11].id),
            (246002001, 330110002, 330110002)
        );
        assert!(
            starter::inventory(&db, "test", 0)
                .unwrap()
                .iter()
                .all(|i| i.id == 0)
        );
    }

    #[test]
    fn failed_equip_requirements_acknowledge_and_preserve_every_item_and_wallet() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO character_inventory VALUES ('test',0,0,206002001,2000,1)",
                [],
            )
            .unwrap();
            s.inventory = starter::inventory(&db, "test", 0).unwrap();
        }
        let before = s.inventory;
        let mut p = vec![0x1f, 1];
        p.extend(206002001u32.to_le_bytes());
        p.extend([0, 10]);
        assert_eq!(handle(&lobby, Some("test"), Some(&mut s), &p), [0x1f, 0]);
        let db = lobby.db.lock().unwrap();
        assert_eq!(starter::inventory(&db, "test", 0).unwrap(), before);
        assert_eq!(s.inventory, before);
        assert_eq!(starter::equipment(&db, "test", 0).unwrap()[10].id, 0);
        assert_eq!(starter::coins(&db, "test", 0).unwrap(), 1_000_000);
    }

    #[test]
    fn offhand_moves_are_owned_atomic_and_persistent() {
        let game = reko_common::game_dir();
        let rules = std::path::Path::new("../../logs/reference-coefficients.csv");
        if !game.join("Data/item_org_us.tbl").exists() || !rules.exists() {
            return;
        }
        let catalog = combat::Catalog::load(&game, rules).unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        starter::initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "admin").unwrap();
        crate::characters::set_nation(&db, "admin", 1).unwrap();
        let character = crate::characters::Character {
            slot: 0,
            name: "InventoryTest".into(),
            race: 2,
            class: 108,
            face: 0,
            hair: 0,
            stats: [60, 150, 255, 50, 50],
        };
        crate::characters::create(&mut db, "admin", &character).unwrap();
        let mut session = Session::new(character).unwrap();
        session.progress.level = 80;
        for slot in [6, 8] {
            db.execute(
                "INSERT INTO character_equipment VALUES ('admin',0,?1,110110001,4000)",
                [slot],
            )
            .unwrap();
        }
        let request = |direction, source, destination| {
            let mut p = vec![0x1f, direction];
            p.extend(110110001u32.to_le_bytes());
            p.extend([source, destination]);
            p
        };
        let (_, bag, one_hand) =
            move_items(&mut db, "admin", &session, &catalog, &request(2, 8, 0)).unwrap();
        assert_eq!(bag[0].id, 110110001);
        assert_eq!(starter::equipment(&db, "admin", 0).unwrap()[8].id, 0);
        assert!(move_items(&mut db, "other", &session, &catalog, &request(1, 0, 8)).is_err());
        assert!(move_items(&mut db, "admin", &session, &catalog, &request(1, 0, 7)).is_err());
        assert_eq!(
            starter::inventory(&db, "admin", 0).unwrap()[0].id,
            110110001
        );
        let (gear, bag, dual) =
            move_items(&mut db, "admin", &session, &catalog, &request(1, 0, 8)).unwrap();
        assert_eq!(gear[8].id, 110110001);
        assert_eq!(bag[0].id, 0);
        assert!(dual.attack > one_hand.attack);
        assert_eq!(attributes(&dual, 1).len(), 34);
        assert!(move_items(&mut db, "admin", &session, &catalog, &request(4, 8, 7)).is_err());
        db.execute(
            "INSERT INTO character_inventory VALUES ('admin',0,1,160100001,5000,1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO character_inventory VALUES ('admin',0,2,391010000,1,99)",
            [],
        )
        .unwrap();
        let item_request = |direction, id: u32, source, destination| {
            let mut packet = vec![0x1f, direction];
            packet.extend(id.to_le_bytes());
            packet.extend([source, destination]);
            packet
        };
        let bow = item_request(1, 160100001, 1, 8);
        assert!(move_items(&mut db, "admin", &session, &catalog, &bow).is_err());
        assert_eq!(
            starter::inventory(&db, "admin", 0).unwrap()[1].id,
            160100001
        );
        move_items(&mut db, "admin", &session, &catalog, &request(2, 8, 0)).unwrap();
        let (gear, _, _) = move_items(&mut db, "admin", &session, &catalog, &bow).unwrap();
        assert_eq!(gear[8].id, 160100001);
        assert_eq!(gear[6].id, 0);
        let bag = starter::inventory(&db, "admin", 0).unwrap();
        assert_eq!(
            (bag[1].id, bag[1].durability, bag[1].count),
            (110110001, 4000, 1)
        );
        move_items(
            &mut db,
            "admin",
            &session,
            &catalog,
            &item_request(2, 160100001, 8, 4),
        )
        .unwrap();
        move_items(&mut db, "admin", &session, &catalog, &request(1, 1, 8)).unwrap();
        let (gear, bag, _) = move_items(
            &mut db,
            "admin",
            &session,
            &catalog,
            &item_request(1, 160100001, 4, 8),
        )
        .unwrap();
        assert_eq!(gear[8].id, 160100001);
        assert_eq!(gear[6].id, 0);
        assert_eq!(bag[4].id, 110110001);
        let (gear, bag, _) =
            move_items(&mut db, "admin", &session, &catalog, &request(1, 4, 6)).unwrap();
        assert_eq!(gear[6].id, 110110001);
        assert_eq!(gear[8].id, 0);
        assert_eq!(
            (bag[4].id, bag[4].durability, bag[4].count),
            (160100001, 5000, 1)
        );
        assert_eq!(starter::equipment(&db, "admin", 0).unwrap(), gear);
        move_items(
            &mut db,
            "admin",
            &session,
            &catalog,
            &item_request(1, 160100001, 4, 8),
        )
        .unwrap();
        let (gear, bag, _) =
            move_items(&mut db, "admin", &session, &catalog, &request(1, 4, 8)).unwrap();
        assert_eq!(gear[8].id, 110110001);
        assert_eq!(gear[6].id, 0);
        assert_eq!(bag[4].id, 160100001);
        move_items(
            &mut db,
            "admin",
            &session,
            &catalog,
            &item_request(3, 391010000, 2, 3),
        )
        .unwrap();
        let bag = starter::inventory(&db, "admin", 0).unwrap();
        assert_eq!(bag[2].id, 0);
        assert_eq!((bag[3].id, bag[3].count), (391010000, 99));
    }
}
