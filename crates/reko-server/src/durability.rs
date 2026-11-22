//! Authoritative equipment wear and repair.

use crate::{
    combat, inventory, lobby::Lobby, resources, skills, starter, utility, world_entry::Session,
};
use rusqlite::params;
use std::net::SocketAddr;
use tokio::net::TcpStream;

fn wear(
    db: &rusqlite::Connection,
    account: &str,
    slot: u8,
    catalog: &combat::Catalog,
    damage: u16,
) -> anyhow::Result<([starter::Item; 14], Vec<Vec<u8>>)> {
    let amount = (f64::from(damage) / 10.0).sqrt() as u16;
    let tx = db.unchecked_transaction()?;
    let mut gear = starter::equipment(&tx, account, slot)?;
    let mut packets = Vec::new();
    for pos in [6, 8] {
        let item = &mut gear[pos];
        if amount == 0 || item.id == 0 || item.durability == 0 {
            continue;
        }
        let (row, _) = catalog
            .item(item.id)
            .ok_or_else(|| anyhow::anyhow!("unknown worn item"))?;
        if combat::integer(row, 12) == 2 {
            continue;
        }
        item.durability = item.durability.saturating_sub(amount);
        let updated = tx.execute("UPDATE character_equipment SET durability=?4 WHERE account=?1 AND slot=?2 AND equipment_slot=?3 AND item_id=?5",
            params![account, slot, pos, item.durability, item.id])?;
        anyhow::ensure!(updated == 1, "missing worn item");
        let mut p = vec![0x38, pos as u8];
        p.extend(item.durability.to_le_bytes());
        packets.push(p);
    }
    tx.commit()?;
    Ok((gear, packets))
}

pub(crate) async fn attack(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    damage: u16,
) {
    let Some(catalog) = lobby.combat.as_ref() else {
        return;
    };
    let result = wear(
        &lobby.db.lock().unwrap(),
        account,
        session.character.slot,
        catalog,
        damage,
    );
    match result {
        Ok((gear, packets)) => {
            let before = (session.combat.attack, session.combat.defence);
            session.equipment = gear;
            for p in packets {
                super::send_response(sock, peer, &None, &p, "ITEM_DURABILITY").await;
            }
            refresh(session, catalog, lobby.skills.as_ref());
            if before != (session.combat.attack, session.combat.defence) {
                attributes(sock, peer, session).await;
            }
        }
        Err(error) => tracing::error!(%peer,%error,"weapon wear persistence failed"),
    }
}

pub(crate) async fn hammer(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    spell: &skills::Spell,
) -> Option<Result<(), &'static str>> {
    let effect = lobby.skills.as_ref()?.effects.get(&(3, spell.id))?;
    if spell.types != [3, 0]
        || spell.skill_group != 0
        || spell.use_item == 0
        || combat::integer(effect, 2) != 4
        || combat::integer(effect, 3) <= 0
    {
        return None;
    }
    let Some(catalog) = lobby.combat.as_ref() else {
        return Some(Err("The repair item catalog is unavailable."));
    };
    let result = (|| {
        let db = lobby.db.lock().unwrap();
        let tx = db
            .unchecked_transaction()
            .map_err(|_| "Unable to start repair.")?;
        let slot = session.character.slot;
        let paid = resources::commit_in(&tx, account, slot, spell, 1, Some(catalog), |_| Ok(()))?;
        let mut gear =
            starter::equipment(&tx, account, slot).map_err(|_| "Unable to load equipped items.")?;
        let mut repaired = Vec::new();
        for (position, item) in gear.iter_mut().enumerate() {
            let Some((base, Some(extension))) = catalog.item(item.id) else {
                continue;
            };
            if combat::integer(base, 10) == 255 {
                continue;
            }
            let maximum = u16::try_from(combat::integer(base, 19) + combat::integer(extension, 12))
                .map_err(|_| "Invalid maximum durability.")?;
            if maximum <= 1 || item.durability >= maximum {
                continue;
            }
            let updated = tx.execute(
                "UPDATE character_equipment SET durability=?4 WHERE account=?1 AND slot=?2 AND equipment_slot=?3 AND item_id=?5",
                params![account, slot, position, maximum, item.id],
            ).map_err(|_| "Unable to save repaired equipment.")?;
            if updated != 1 {
                return Err("An equipped item changed during repair.");
            }
            item.durability = maximum;
            repaired.push((position as u8, maximum));
        }
        tx.commit()
            .map_err(|_| "Unable to commit Magic Hammer repair.")?;
        Ok((paid, gear, repaired))
    })();
    let (paid, gear, repaired) = match result {
        Ok(result) => result,
        Err(reason) => return Some(Err(reason)),
    };
    session.equipment = gear;
    resources::sync(sock, peer, lobby, account, session, paid).await;
    for (position, maximum) in repaired {
        let mut packet = vec![0x38, position];
        packet.extend(maximum.to_le_bytes());
        super::send_response(sock, peer, &None, &packet, "ITEM_DURABILITY").await;
    }
    refresh(session, catalog, lobby.skills.as_ref());
    attributes(sock, peer, session).await;
    super::send_response(
        sock,
        peer,
        &None,
        &skills::effect(spell.id, session.player_id),
        "MAGIC_HAMMER",
    )
    .await;
    Some(Ok(()))
}

pub(crate) fn refresh(
    session: &mut Session,
    catalog: &combat::Catalog,
    skills: Option<&crate::skills::Catalog>,
) {
    if let Ok(mut base) =
        catalog.calculate(&session.character, &session.progress, &session.equipment)
    {
        base.weight = base
            .weight
            .saturating_add(catalog.bag_weight(&session.inventory));
        session.base_combat = base;
        utility::refresh(session, skills, Some(catalog));
    }
}

pub(crate) async fn attributes(sock: &mut TcpStream, peer: SocketAddr, session: &Session) {
    super::send_response(
        sock,
        peer,
        &None,
        &inventory::attributes(&session.combat, 2),
        "DURABILITY_ATTRIBUTES",
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wear_survives_reload_clamps_and_ignores_shields_and_small_damage() {
        let game = reko_common::game_dir();
        let rules = std::path::Path::new("../../logs/reference-coefficients.csv");
        if !game.join("Data/item_org_us.tbl").exists() || !rules.exists() {
            return;
        }
        let catalog = combat::Catalog::load(&game, rules).unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "test").unwrap();
        crate::characters::set_nation(&db, "test", 1).unwrap();
        db.execute(
            "INSERT INTO lobby_characters VALUES ('test',0,'WearTest',2,108,0,0,?1)",
            [[60u8, 60, 70, 50, 50].as_slice()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO character_equipment VALUES ('test',0,6,110110001,3)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO character_equipment VALUES ('test',0,8,170150001,100)",
            [],
        )
        .unwrap();
        assert!(wear(&db, "test", 0, &catalog, 9).unwrap().1.is_empty());
        let (gear, packets) = wear(&db, "test", 0, &catalog, 100).unwrap();
        assert_eq!(gear[6].durability, 0);
        assert_eq!(gear[8].durability, 100);
        assert_eq!(packets, vec![vec![0x38, 6, 0, 0]]);
        assert_eq!(starter::equipment(&db, "test", 0).unwrap(), gear);
        assert!(wear(&db, "test", 0, &catalog, 100).unwrap().1.is_empty());
    }
}
