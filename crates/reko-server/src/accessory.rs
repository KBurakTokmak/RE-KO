//! Accessory compounding at the anvil (native `5B 03`).
//!
//! The native 1453 client sends a 30-byte request: mode, anvil NPC, three
//! identical accessories and two material entries (one Accessory Compound
//! Scroll). Every request must be answered with subtype 3 or the client's
//! compound window stays locked. The 1453 data has no accessory recipes, so the
//! item chains, costs and scroll come from supplementary 2086 rows (migration
//! 010). Success rates are an owner rule: steps to +1..+3 always succeed, +3→+4
//! succeeds 50% and +4→+5 25%; higher steps are refused. A failure destroys all
//! three accessories and the scroll, and coins are charged either way.
use crate::{combat, lobby::Lobby, starter, world_entry::Session};
use rusqlite::{Connection, params};
use std::collections::HashSet;

pub(crate) const COMPOUND_SCROLL: u32 = 379_159_000;

/// Native reply codes for refusals (client texts 6702-6704).
pub(crate) const CANNOT_UPGRADE: u8 = 2;
pub(crate) const NEED_COINS: u8 = 3;
pub(crate) const NO_MATCH: u8 = 4;

#[derive(Clone, Debug)]
struct Recipe {
    origin_type: i64,
    origin: u32,
    scroll: u32,
    coins: u32,
    /// Signed: a few supplementary rows step backwards.
    give: i64,
    npc: u16,
}

fn recipes(db: &Connection) -> rusqlite::Result<Vec<Recipe>> {
    let mut query = db.prepare(
        "SELECT CAST(json_extract(data,'$.nOriginType') AS INTEGER),CAST(json_extract(data,'$.nOriginItem') AS INTEGER),CAST(json_extract(data,'$.nReqItem1') AS INTEGER),CAST(json_extract(data,'$.nReqNoah') AS INTEGER),CAST(json_extract(data,'$.nGiveItem') AS INTEGER),CAST(json_extract(data,'$.nNPCNum') AS INTEGER) FROM world_reference_rules WHERE table_name='ACCESSORY_UPGRADE' ORDER BY rule_id",
    )?;
    query
        .query_map([], |r| {
            Ok(Recipe {
                origin_type: r.get(0)?,
                origin: r.get(1)?,
                scroll: r.get(2)?,
                coins: r.get(3)?,
                give: r.get(4)?,
                npc: r.get::<_, i64>(5)?.clamp(0, 65535) as u16,
            })
        })?
        .collect()
}

/// The + level a recipe starts from: the number of earlier recipes in its chain.
fn level(recipes: &[Recipe], recipe: &Recipe) -> u8 {
    let mut origin = recipe.origin;
    let mut level = 0;
    while level < 20
        && let Some(previous) = recipes.iter().find(|r| {
            r.origin_type == recipe.origin_type
                && (i64::from(r.origin) + r.give) == i64::from(origin)
        })
    {
        origin = previous.origin;
        level += 1;
    }
    level
}

/// Owner rule: +0..+2 starts always succeed, +3→+4 50%, +4→+5 25%.
pub(crate) fn rate(level: u8) -> Option<u32> {
    match level {
        0..=2 => Some(10_000),
        3 => Some(5_000),
        4 => Some(2_500),
        _ => None,
    }
}

pub(crate) type Outcome = ([starter::Item; 28], u32, Vec<u8>, u8);

fn refuse(code: u8, reason: &str) -> (u8, String) {
    (code, reason.to_owned())
}

pub(crate) fn attempt(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    p: &[u8],
    roll: u32,
) -> Result<Outcome, (u8, String)> {
    if p.len() != 30 || p[1] != 3 || !matches!(p[2], 1 | 2) {
        return Err(refuse(NO_MATCH, "malformed accessory request"));
    }
    let mode = p[2];
    if !s.entered || s.skills.hp == 0 || s.merchant.is_some() {
        return Err(refuse(CANNOT_UPGRADE, "cannot compound now"));
    }
    let &(object_id, npc, position) = s
        .anvil
        .as_ref()
        .ok_or_else(|| refuse(CANNOT_UPGRADE, "open an anvil first"))?;
    if !crate::upgrading::anvil_in_range(s.position, position)
        || u16::from_le_bytes([p[3], p[4]]) != npc
    {
        return Err(refuse(CANNOT_UPGRADE, "anvil out of range or mismatched"));
    }
    let entry = |i: usize| {
        let start = 5 + i * 5;
        (
            u32::from_le_bytes(p[start..start + 4].try_into().unwrap()),
            p[start + 4],
        )
    };
    let accessories = [entry(0), entry(1), entry(2)];
    let materials = [entry(3), entry(4)];
    let catalog = lobby
        .combat
        .as_ref()
        .ok_or_else(|| refuse(CANNOT_UPGRADE, "item catalog unavailable"))?;
    let db = lobby.db.lock().unwrap();
    let tx = db
        .unchecked_transaction()
        .map_err(|e| refuse(CANNOT_UPGRADE, &e.to_string()))?;
    let mut bag = starter::inventory(&tx, account, s.character.slot)
        .map_err(|e| refuse(CANNOT_UPGRADE, &e.to_string()))?;
    let mut slots = HashSet::new();
    let origin = accessories[0].0;
    for (id, pos) in accessories {
        let item = bag.get(usize::from(pos)).copied().unwrap_or_default();
        if id != origin || id == 0 || item.id != id || item.count != 1 || !slots.insert(pos) {
            return Err(refuse(
                NO_MATCH,
                "three identical owned accessories required",
            ));
        }
    }
    let (base, ext) = catalog
        .item(origin)
        .ok_or_else(|| refuse(NO_MATCH, "unknown accessory"))?;
    let ext = ext.ok_or_else(|| refuse(NO_MATCH, "unknown accessory extension"))?;
    if !(91..=94).contains(&combat::integer(base, 10)) || !matches!(combat::integer(ext, 7), 4 | 5)
    {
        return Err(refuse(NO_MATCH, "not a compoundable accessory"));
    }
    let [(scroll, scroll_pos), second] = materials;
    let held = bag
        .get(usize::from(scroll_pos))
        .copied()
        .unwrap_or_default();
    if second != (0, 255)
        || scroll != COMPOUND_SCROLL
        || held.id != scroll
        || held.count == 0
        || !slots.insert(scroll_pos)
    {
        return Err(refuse(NO_MATCH, "one Accessory Compound Scroll required"));
    }
    let all = recipes(&tx).map_err(|e| refuse(CANNOT_UPGRADE, &e.to_string()))?;
    let origin_type = combat::integer(base, 1);
    let recipe = all
        .iter()
        .find(|r| {
            r.origin == origin % 100_000
                && r.origin_type == i64::from(origin_type)
                && r.scroll == scroll
                && (r.npc == 0 || r.npc == object_id)
        })
        .ok_or_else(|| refuse(NO_MATCH, "no accessory recipe"))?;
    let rate =
        rate(level(&all, recipe)).ok_or_else(|| refuse(NO_MATCH, "maximum compound level"))?;
    let upgraded = u32::try_from(i64::from(origin) + recipe.give)
        .map_err(|_| refuse(NO_MATCH, "invalid result"))?;
    let (new_base, new_ext) = catalog
        .item(upgraded)
        .ok_or_else(|| refuse(NO_MATCH, "result absent from client"))?;
    let new_ext = new_ext.ok_or_else(|| refuse(NO_MATCH, "result extension absent from client"))?;
    let coins = starter::coins(&tx, account, s.character.slot)
        .map_err(|e| refuse(CANNOT_UPGRADE, &e.to_string()))?;
    if coins < recipe.coins {
        return Err(refuse(NEED_COINS, "insufficient coins"));
    }
    if mode == 2 {
        let mut reply = vec![0x5b, 3, 2, 1];
        reply.extend(upgraded.to_le_bytes());
        return Ok((bag, coins, reply, 1));
    }
    let success = roll < rate;
    let mut reply = vec![0x5b, 3, 1, u8::from(success)];
    let entries = [
        (if success { upgraded } else { origin }, accessories[0].1),
        accessories[1],
        accessories[2],
        (scroll, scroll_pos),
        (0, 255),
    ];
    for (id, pos) in entries {
        reply.extend(id.to_le_bytes());
        reply.push(pos);
    }
    bag[usize::from(accessories[0].1)] = if success {
        starter::Item {
            id: upgraded,
            count: 1,
            durability: (combat::integer(new_base, 19) + combat::integer(new_ext, 12))
                .clamp(0, 65535) as u16,
        }
    } else {
        starter::Item::default()
    };
    for (_, pos) in &accessories[1..] {
        bag[usize::from(*pos)] = starter::Item::default();
    }
    let item = &mut bag[usize::from(scroll_pos)];
    item.count -= 1;
    if item.count == 0 {
        *item = starter::Item::default();
    }
    let persist = || -> rusqlite::Result<u32> {
        for &pos in &slots {
            let i = bag[usize::from(pos)];
            tx.execute(
                "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
                params![account, s.character.slot, pos],
            )?;
            if i.id != 0 {
                tx.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?1,?2,?3,?4,?5,?6)",params![account,s.character.slot,pos,i.id,i.durability,i.count])?;
            }
        }
        let left = coins - recipe.coins;
        if tx.execute(
            "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
            params![account, s.character.slot, left],
        )? != 1
        {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(left)
    };
    let left = persist().map_err(|e| refuse(CANNOT_UPGRADE, &e.to_string()))?;
    tx.commit()
        .map_err(|e| refuse(CANNOT_UPGRADE, &e.to_string()))?;
    tracing::info!(character=%s.character.name,origin,upgraded,success,rate,cost=recipe.coins,"accessory compound committed");
    Ok((bag, left, reply, u8::from(success)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(mode: u8, npc: u16, accessories: [(u32, u8); 3], scroll: (u32, u8)) -> Vec<u8> {
        let mut p = vec![0x5b, 3, mode];
        p.extend(npc.to_le_bytes());
        for (id, pos) in accessories.into_iter().chain([scroll, (0, 255)]) {
            p.extend(id.to_le_bytes());
            p.push(pos);
        }
        p
    }

    #[test]
    fn bronze_earring_chain_follows_owner_rates_and_consumes_atomically() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        s.anvil = Some((5001, 7, s.position));
        {
            let db = lobby.db.lock().unwrap();
            for (index, origin, give) in [
                (1, 10101, 100),
                (2, 10201, 1),
                (3, 10202, 1),
                (4, 10203, 1),
                (5, 10204, 1),
                (6, 10205, 1),
            ] {
                db.execute(
                    "INSERT INTO world_reference_rules VALUES('ACCESSORY_UPGRADE',?1,?2)",
                    params![
                        index,
                        serde_json::json!({"nOriginType":18,"nOriginItem":origin,"nReqItem1":COMPOUND_SCROLL,"nReqNoah":240000,"nGiveItem":give,"nNPCNum":5001,"strNote":""}).to_string()
                    ],
                )
                .unwrap();
            }
        }
        let stock = |items: &[(u8, u32, u16)]| {
            let db = lobby.db.lock().unwrap();
            db.execute("DELETE FROM character_inventory", []).unwrap();
            for &(pos, id, count) in items {
                db.execute(
                    "INSERT INTO character_inventory VALUES('test',0,?1,?2,1000,?3)",
                    params![pos, id, count],
                )
                .unwrap();
            }
        };
        let three = |id| [(id, 0u8), (id, 1), (id, 2)];
        assert_eq!(rate(0), Some(10_000));
        assert_eq!(rate(3), Some(5_000));
        assert_eq!(rate(4), Some(2_500));
        assert_eq!(rate(5), None);

        // +0 → +1 is guaranteed and the result replaces the first slot.
        stock(&[
            (0, 310110101, 1),
            (1, 310110101, 1),
            (2, 310110101, 1),
            (3, COMPOUND_SCROLL, 2),
        ]);
        let preview = attempt(
            &lobby,
            "test",
            &s,
            &request(2, 7, three(310110101), (COMPOUND_SCROLL, 3)),
            9999,
        )
        .unwrap();
        assert_eq!(
            preview.2,
            [&[0x5b, 3, 2, 1][..], &310110201u32.to_le_bytes()].concat()
        );
        assert_eq!(preview.0[3].count, 2, "preview consumes nothing");
        let (bag, coins, reply, effect) = attempt(
            &lobby,
            "test",
            &s,
            &request(1, 7, three(310110101), (COMPOUND_SCROLL, 3)),
            9999,
        )
        .unwrap();
        assert_eq!((effect, &reply[..4]), (1, &[0x5b, 3, 1, 1][..]));
        assert_eq!(
            u32::from_le_bytes(reply[4..8].try_into().unwrap()),
            310110201
        );
        assert_eq!(
            (bag[0].id, bag[1].id, bag[2].id, bag[3].count),
            (310110201, 0, 0, 1)
        );
        assert_eq!(coins, 1_000_000 - 240_000);
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            bag
        );
        // Replaying the same request no longer matches owned items.
        let replay = attempt(
            &lobby,
            "test",
            &s,
            &request(1, 7, three(310110101), (COMPOUND_SCROLL, 3)),
            0,
        );
        assert_eq!(replay.map(|_| ()).unwrap_err().0, NO_MATCH);

        // +3 → +4 succeeds below 50% and destroys everything above it.
        stock(&[
            (0, 310110203, 1),
            (1, 310110203, 1),
            (2, 310110203, 1),
            (5, COMPOUND_SCROLL, 1),
        ]);
        let (bag, _, reply, effect) = attempt(
            &lobby,
            "test",
            &s,
            &request(1, 7, three(310110203), (COMPOUND_SCROLL, 5)),
            5000,
        )
        .unwrap();
        assert_eq!((effect, reply[3]), (0, 0));
        assert!(
            bag.iter().all(|i| i.id == 0),
            "failure destroys accessories and scroll"
        );
        assert_eq!(
            u32::from_le_bytes(reply[4..8].try_into().unwrap()),
            310110203
        );
        stock(&[
            (0, 310110203, 1),
            (1, 310110203, 1),
            (2, 310110203, 1),
            (5, COMPOUND_SCROLL, 1),
        ]);
        let (bag, ..) = attempt(
            &lobby,
            "test",
            &s,
            &request(1, 7, three(310110203), (COMPOUND_SCROLL, 5)),
            4999,
        )
        .unwrap();
        assert_eq!(bag[0].id, 310110204);

        // +5 is the maximum; mismatched, short and wrong-scroll requests change nothing.
        stock(&[
            (0, 310110205, 1),
            (1, 310110205, 1),
            (2, 310110205, 1),
            (3, COMPOUND_SCROLL, 1),
        ]);
        let max = attempt(
            &lobby,
            "test",
            &s,
            &request(1, 7, three(310110205), (COMPOUND_SCROLL, 3)),
            0,
        );
        assert_eq!(max.map(|_| ()).unwrap_err().0, NO_MATCH);
        stock(&[
            (0, 310110203, 1),
            (1, 310110203, 1),
            (2, 310110204, 1),
            (3, COMPOUND_SCROLL, 1),
        ]);
        let mixed = [(310110203, 0), (310110203, 1), (310110204, 2)];
        assert_eq!(
            attempt(
                &lobby,
                "test",
                &s,
                &request(1, 7, mixed, (COMPOUND_SCROLL, 3)),
                0
            )
            .map(|_| ())
            .unwrap_err()
            .0,
            NO_MATCH
        );
        let duplicate = [(310110203, 0), (310110203, 1), (310110203, 1)];
        assert_eq!(
            attempt(
                &lobby,
                "test",
                &s,
                &request(1, 7, duplicate, (COMPOUND_SCROLL, 3)),
                0
            )
            .map(|_| ())
            .unwrap_err()
            .0,
            NO_MATCH
        );
        assert_eq!(
            attempt(
                &lobby,
                "test",
                &s,
                &request(1, 9, three(310110203), (COMPOUND_SCROLL, 3)),
                0
            )
            .map(|_| ())
            .unwrap_err()
            .0,
            CANNOT_UPGRADE
        );
        assert_eq!(
            attempt(
                &lobby,
                "test",
                &s,
                &request(1, 7, three(310110203), (379160000, 3)),
                0
            )
            .map(|_| ())
            .unwrap_err()
            .0,
            NO_MATCH
        );
        lobby
            .db
            .lock()
            .unwrap()
            .execute("UPDATE character_wallets SET coins=1", [])
            .unwrap();
        stock(&[
            (0, 310110201, 1),
            (1, 310110201, 1),
            (2, 310110201, 1),
            (3, COMPOUND_SCROLL, 1),
        ]);
        assert_eq!(
            attempt(
                &lobby,
                "test",
                &s,
                &request(1, 7, three(310110201), (COMPOUND_SCROLL, 3)),
                0
            )
            .map(|_| ())
            .unwrap_err()
            .0,
            NEED_COINS
        );
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[3].count,
            1
        );
    }
}
