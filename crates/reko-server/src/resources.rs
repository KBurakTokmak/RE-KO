//! Atomic skill resources, inventory consumption and state effects.

use crate::{inventory, lobby::Lobby, skills, starter, world_entry::Session};
use std::net::SocketAddr;
use tokio::net::TcpStream;

pub(crate) struct Paid {
    pub state: skills::State,
    pub changed: Vec<(usize, starter::Item)>,
}

pub(crate) fn commit(
    lobby: &Lobby,
    account: &str,
    slot: u8,
    spell: &skills::Spell,
    quantity: u16,
    apply: impl FnOnce(&mut skills::State) -> Result<(), &'static str>,
) -> Result<Paid, &'static str> {
    let db = lobby.db.lock().unwrap();
    let tx = db
        .unchecked_transaction()
        .map_err(|_| "Unable to start skill transaction.")?;
    let paid = commit_in(
        &tx,
        account,
        slot,
        spell,
        quantity,
        lobby.combat.as_ref(),
        apply,
    )?;
    tx.commit()
        .map_err(|_| "Unable to commit skill resources.")?;
    Ok(paid)
}

/// The caller commits payment and all affected players/positions together.
pub(crate) fn commit_in(
    tx: &rusqlite::Transaction<'_>,
    account: &str,
    slot: u8,
    spell: &skills::Spell,
    quantity: u16,
    catalog: Option<&crate::combat::Catalog>,
    apply: impl FnOnce(&mut skills::State) -> Result<(), &'static str>,
) -> Result<Paid, &'static str> {
    let mut state =
        skills::load(tx, account, slot).map_err(|_| "Unable to load skill resources.")?;
    state.spend(spell, skills::now_ms())?;
    let consumed = spell.consumed_item();
    if consumed != spell.use_item {
        let bag = starter::inventory(tx, account, slot)
            .map_err(|_| "Unable to check required scroll.")?;
        if !bag
            .iter()
            .any(|item| item.id == spell.use_item && item.count > 0)
        {
            return Err("You do not have the required scroll.");
        }
    }
    let charged = catalog
        .and_then(|c| c.item(consumed))
        .is_some_and(|(base, _)| crate::combat::integer(base, 10) == 255);
    let changed = if charged {
        inventory::consume_charges(tx, account, slot, consumed, quantity)
    } else {
        inventory::consume(tx, account, slot, consumed, quantity)
    }
    .map_err(|_| "You do not have the required consumable items.")?;
    apply(&mut state)?;
    if spell.cast_ms > 0 {
        state.buffs.remove(&23);
    }
    skills::save_in(tx, account, slot, &state).map_err(|_| "Unable to save skill resources.")?;
    Ok(Paid { state, changed })
}

pub(crate) async fn sync(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    paid: Paid,
) {
    let instant_consumed =
        session.skills.buffs.contains_key(&23) && !paid.state.buffs.contains_key(&23);
    let hp_delta = i32::from(paid.state.hp) - i32::from(session.skills.hp);
    let hp_limit_changed = paid.state.max_hp != session.skills.max_hp;
    session.skills = paid.state;
    if instant_consumed {
        super::send_response(sock, peer, &None, &[0x31, 5, 23], "INSTANT_MAGIC_CONSUMED").await;
    }
    super::send_response(
        sock,
        peer,
        &None,
        &session.skills.mana_packet(),
        "SKILL_MANA",
    )
    .await;
    if hp_delta != 0 || hp_limit_changed || session.skills.hp == session.skills.max_hp {
        super::send_response(
            sock,
            peer,
            &None,
            &session
                .skills
                .hp_packet(hp_delta.clamp(-32767, 32767) as i16),
            "SKILL_HEALTH",
        )
        .await;
    }
    if !paid.changed.is_empty() {
        let bag = starter::inventory(&lobby.db.lock().unwrap(), account, session.character.slot);
        if let Ok(bag) = bag {
            session.inventory = bag;
            if let Some(catalog) = lobby.combat.as_ref()
                && let Ok(mut base) =
                    catalog.calculate(&session.character, &session.progress, &session.equipment)
            {
                base.weight = base.weight.saturating_add(catalog.bag_weight(&bag));
                session.base_combat = base;
            }
        }
        super::send_response(
            sock,
            peer,
            &None,
            &inventory::count_packet(&paid.changed),
            "SKILL_ITEMS",
        )
        .await;
        let mut weight = vec![0x54];
        weight.extend(session.base_combat.weight.to_le_bytes());
        super::send_response(sock, peer, &None, &weight, "ITEM_WEIGHT").await;
    }
    crate::utility::refresh(session, lobby.skills.as_ref(), lobby.combat.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mage_scroll_requirements_survive_repeated_casts_but_other_items_are_consumed() {
        let game = reko_common::game_dir();
        if !game.join("Data/Skill_Magic_Main_us.tbl").exists() {
            return;
        }
        let mut lobby = Lobby::new(
            rusqlite::Connection::open_in_memory().unwrap(),
            crate::characters::CreationRules::load(&game).unwrap(),
            crate::world_entry::Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        lobby.skills = Some(skills::Catalog::load(&game).unwrap());
        {
            let db = lobby.db.lock().unwrap();
            crate::characters::ensure_account(&db, "mage").unwrap();
            crate::characters::set_nation(&db, "mage", 1).unwrap();
            db.execute(
                "INSERT INTO lobby_characters VALUES ('mage',0,'ScrollTest',3,110,0,0,?1)",
                [[50u8, 60, 60, 70, 50].as_slice()],
            )
            .unwrap();
            let mut state = skills::State::default();
            state.set_limits(5000, 10000);
            skills::save(&db, "mage", 0, &state).unwrap();
        }
        for prefix in [110000, 210000] {
            for suffix in [535, 635, 735, 554, 754, 557, 657, 757] {
                let mut spell = lobby.skills.as_ref().unwrap().spells[&(prefix + suffix)].clone();
                assert!(spell.requires_scroll());
                spell.cooldown_ms = 0;
                let before = skills::load(&lobby.db.lock().unwrap(), "mage", 0)
                    .unwrap()
                    .mana;
                assert!(commit(&lobby, "mage", 0, &spell, 1, |_| Ok(())).is_err());
                assert_eq!(
                    skills::load(&lobby.db.lock().unwrap(), "mage", 0)
                        .unwrap()
                        .mana,
                    before
                );
                lobby
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "INSERT INTO character_inventory VALUES ('mage',0,0,?1,1,1)",
                        [spell.use_item],
                    )
                    .unwrap();
                for _ in 0..2 {
                    assert!(
                        commit(&lobby, "mage", 0, &spell, 1, |_| Ok(()))
                            .unwrap()
                            .changed
                            .is_empty()
                    );
                    assert_eq!(
                        starter::inventory(&lobby.db.lock().unwrap(), "mage", 0).unwrap()[0].count,
                        1
                    );
                }
                assert_eq!(
                    skills::load(&lobby.db.lock().unwrap(), "mage", 0)
                        .unwrap()
                        .mana,
                    before - 2 * spell.mana
                );
                lobby
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "DELETE FROM character_inventory WHERE account='mage' AND slot=0",
                        [],
                    )
                    .unwrap();
            }
        }
        let spell = &lobby.skills.as_ref().unwrap().spells[&110802];
        assert!(!spell.requires_scroll());
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO character_inventory VALUES ('mage',0,0,?1,1,1)",
                [spell.use_item],
            )
            .unwrap();
        assert!(commit(&lobby, "mage", 0, spell, 1, |_| Ok(())).is_err());
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO character_inventory VALUES ('mage',0,1,379061000,1,2)",
                [],
            )
            .unwrap();
        assert_eq!(
            commit(&lobby, "mage", 0, spell, 1, |_| Ok(()))
                .unwrap()
                .changed
                .len(),
            1
        );
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "mage", 0).unwrap()[0].id,
            379065000
        );
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "mage", 0).unwrap()[1].count,
            1
        );
    }
}
