//! Account-shared inn inventory and coin transfers.
use crate::{combat, inventory, lobby::Lobby, starter, world_entry::Session};
use rusqlite::{Connection, OptionalExtension, params};
use std::net::SocketAddr;
use tokio::net::TcpStream;

fn contents(db: &Connection, account: &str) -> rusqlite::Result<([starter::Item; 192], u32)> {
    let mut items = [starter::Item::default(); 192];
    let mut q = db.prepare(
        "SELECT position,item_id,durability,count FROM account_storage WHERE account=?1",
    )?;
    for r in q.query_map([account], |r| {
        Ok((
            r.get::<_, usize>(0)?,
            starter::Item {
                id: r.get(1)?,
                durability: r.get(2)?,
                count: r.get(3)?,
            },
        ))
    })? {
        let (p, i) = r?;
        if p < 192 {
            items[p] = i;
        }
    }
    let bank = db
        .query_row(
            "SELECT coins FROM account_bank WHERE account=?1",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok((items, bank))
}

fn nearby(lobby: &Lobby, s: &Session) -> bool {
    s.entered
        && s.skills.hp > 0
        && lobby
            .merchants
            .iter()
            .any(|n| Some(n.id) == s.merchant && n.kind == 31 && n.nearby(s))
}
fn transfer(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    p: &[u8],
) -> anyhow::Result<([starter::Item; 28], u32)> {
    anyhow::ensure!(nearby(lobby, s), "open a nearby inn first");
    let action = p[1];
    let explicit = matches!(p.len(), 11 | 15);
    let start = if explicit {
        anyhow::ensure!(
            Some(u16::from_le_bytes([p[2], p[3]])) == s.merchant,
            "inn mismatch"
        );
        4
    } else {
        2
    };
    anyhow::ensure!(
        matches!((action, p.len()), (2 | 3, 13 | 15) | (4 | 5, 9 | 11)),
        "malformed warehouse request"
    );
    let id = u32::from_le_bytes(p[start..start + 4].try_into()?);
    let page = usize::from(p[start + 4]);
    let src = usize::from(p[start + 5]);
    let dst = usize::from(p[start + 6]);
    anyhow::ensure!(page < 8, "invalid warehouse page");
    let count = if action <= 3 {
        u32::from_le_bytes(p[start + 7..start + 11].try_into()?)
    } else {
        0
    };
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let (mut store, mut bank) = contents(&tx, account)?;
    let mut bag = starter::inventory(&tx, account, s.character.slot)?;
    let mut coins = starter::coins(&tx, account, s.character.slot)?;
    if id == 900000000 && matches!(action, 2 | 3) {
        anyhow::ensure!(count > 0, "empty coin transfer");
        let (from, to) = if action == 2 {
            (&mut coins, &mut bank)
        } else {
            (&mut bank, &mut coins)
        };
        *from = from
            .checked_sub(count)
            .ok_or_else(|| anyhow::anyhow!("insufficient coins"))?;
        *to = to
            .checked_add(count)
            .filter(|v| *v <= 2100000000)
            .ok_or_else(|| anyhow::anyhow!("coin cap"))?;
    } else {
        let catalog = lobby
            .combat
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("catalog unavailable"))?;
        let (base, extension) = catalog
            .item(id)
            .ok_or_else(|| anyhow::anyhow!("unknown item"))?;
        anyhow::ensure!(
            extension.is_some() && id < 900000000,
            "item cannot be stored"
        );
        let stack = combat::integer(base, 23) != 0;
        let (from, to) = match action {
            2 => {
                anyhow::ensure!(src < 28 && dst < 24, "slot outside inventory/page");
                (&mut bag[src], &mut store[page * 24 + dst])
            }
            3 => {
                anyhow::ensure!(src < 24 && dst < 28, "slot outside inventory/page");
                (&mut store[page * 24 + src], &mut bag[dst])
            }
            4 => {
                anyhow::ensure!(src < 24 && dst < 24 && src != dst, "invalid warehouse move");
                let source = page * 24 + src;
                let target = page * 24 + dst;
                anyhow::ensure!(
                    store[source].id == id && store[target].id == 0,
                    "move ownership mismatch"
                );
                store.swap(source, target);
                return save(tx, account, s, &store, bank, bag, coins);
            }
            5 => {
                anyhow::ensure!(
                    src < 28 && dst < 28 && src != dst && bag[src].id == id && bag[dst].id == 0,
                    "invalid inventory move"
                );
                bag.swap(src, dst);
                return save(tx, account, s, &store, bank, bag, coins);
            }
            _ => anyhow::bail!("invalid warehouse action"),
        };
        anyhow::ensure!(
            count > 0 && count <= 9999 && from.id == id && u32::from(from.count) >= count,
            "source ownership/count mismatch"
        );
        anyhow::ensure!(
            to.id == 0 || (stack && to.id == id && to.durability == from.durability),
            "destination occupied"
        );
        anyhow::ensure!(stack || count == 1, "nonstackable count");
        let total = u32::from(to.count) + count;
        anyhow::ensure!(total <= 9999, "stack cap");
        if to.id == 0 {
            *to = *from;
        }
        to.count = total as u16;
        from.count -= count as u16;
        if from.count == 0 {
            *from = starter::Item::default();
        }
        let weight = u32::from(catalog.bag_weight(&bag))
            + u32::from(
                catalog
                    .calculate(&s.character, &s.progress, &s.equipment)?
                    .weight,
            );
        anyhow::ensure!(
            action != 3 || weight <= u32::from(s.combat.max_weight),
            "weight limit"
        );
    }
    save(tx, account, s, &store, bank, bag, coins)
}
fn save(
    tx: rusqlite::Transaction<'_>,
    account: &str,
    s: &Session,
    store: &[starter::Item; 192],
    bank: u32,
    bag: [starter::Item; 28],
    coins: u32,
) -> anyhow::Result<([starter::Item; 28], u32)> {
    tx.execute("DELETE FROM account_storage WHERE account=?1", [account])?;
    for (p, i) in store.iter().enumerate().filter(|(_, i)| i.id != 0) {
        tx.execute(
            "INSERT INTO account_storage VALUES (?1,?2,?3,?4,?5)",
            params![account, p, i.id, i.durability, i.count],
        )?;
    }
    tx.execute("INSERT INTO account_bank VALUES (?1,?2) ON CONFLICT(account) DO UPDATE SET coins=excluded.coins",params![account,bank])?;
    tx.execute(
        "DELETE FROM character_inventory WHERE account=?1 AND slot=?2",
        params![account, s.character.slot],
    )?;
    for (p, i) in bag.iter().enumerate().filter(|(_, i)| i.id != 0) {
        tx.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?1,?2,?3,?4,?5,?6)",params![account,s.character.slot,p,i.id,i.durability,i.count])?;
    }
    anyhow::ensure!(
        tx.execute(
            "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
            params![account, s.character.slot, coins]
        )? == 1,
        "wallet missing"
    );
    tx.commit()?;
    Ok((bag, coins))
}
pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) {
    if p.len() < 2 || !nearby(lobby, s) {
        return;
    }
    if p[1] == 1 {
        let result = contents(&lobby.db.lock().unwrap(), account);
        if let Ok((items, coins)) = result {
            let mut reply = vec![0x45, 1, 1];
            reply.extend(coins.to_le_bytes());
            for i in items {
                reply.extend(i.id.to_le_bytes());
                reply.extend(i.durability.to_le_bytes());
                reply.extend(i.count.to_le_bytes());
                reply.push(0);
                reply.extend([0; 8]);
            }
            super::send_response(sock, peer, &None, &reply, "WAREHOUSE_OPEN").await;
        }
        return;
    }
    match transfer(lobby, account, s, p) {
        Ok((bag, coins)) => {
            let changed: Vec<_> = bag
                .iter()
                .enumerate()
                .filter(|(p, i)| s.inventory[*p] != **i)
                .map(|(p, i)| (p, *i))
                .collect();
            s.inventory = bag;
            s.coins = coins;
            super::send_response(sock, peer, &None, &[0x45, p[1], 1], "WAREHOUSE_TRANSFER").await;
            if !changed.is_empty() {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &inventory::count_packet(&changed),
                    "WAREHOUSE_ITEMS",
                )
                .await;
            }
            if let Some(c) = lobby.combat.as_ref() {
                crate::durability::refresh(s, c, lobby.skills.as_ref());
                crate::durability::attributes(sock, peer, s).await;
            }
        }
        Err(e) => {
            tracing::info!(%peer,%e,"warehouse transfer refused");
            super::send_response(sock, peer, &None, &[0x45, p[1], 0], "WAREHOUSE_FAILED").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deposits_withdrawals_are_owned_atomic_and_shared_by_account() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            crate::population::tests::seed(&db);
            db.execute(
                "UPDATE world_spawns SET monster=0,kind=31,x0=3200,x1=3200,z0=4000,z1=4000",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO character_inventory VALUES ('test',0,0,389014000,1,10)",
                [],
            )
            .unwrap();
        }
        lobby.merchants =
            crate::shops::load_population(&lobby.db.lock().unwrap(), &reko_common::game_dir())
                .unwrap();
        s.position.x = 3200;
        s.position.z = 4000;
        s.merchant = Some(12000);
        let request = |action: u8, id: u32, src: u8, dst: u8, count: u32| {
            let mut p = vec![0x45, action];
            p.extend(id.to_le_bytes());
            p.extend([0, src, dst]);
            p.extend(count.to_le_bytes());
            p
        };
        transfer(&lobby, "test", &s, &request(2, 389014000, 0, 3, 4)).unwrap();
        let (store, bank) = contents(&lobby.db.lock().unwrap(), "TEST").unwrap();
        assert_eq!((store[3].count, bank), (4, 0));
        let bag = starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap();
        assert_eq!(bag[0].count, 6);
        assert!(transfer(&lobby, "test", &s, &request(2, 389014000, 0, 3, 7)).is_err());
        assert_eq!(
            contents(&lobby.db.lock().unwrap(), "test").unwrap().0[3].count,
            4
        );
        transfer(&lobby, "test", &s, &request(3, 389014000, 3, 1, 4)).unwrap();
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[1].count,
            4
        );
        assert!(transfer(&lobby, "test", &s, &request(3, 389014000, 3, 1, 4)).is_err());
        transfer(&lobby, "test", &s, &request(2, 900000000, 0, 0, 400000)).unwrap();
        assert_eq!(
            contents(&lobby.db.lock().unwrap(), "test").unwrap().1,
            400000
        );
        assert_eq!(
            starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            600000
        );
        assert_eq!(contents(&lobby.db.lock().unwrap(), "other").unwrap().1, 0);
        s.position.x = 100;
        assert!(transfer(&lobby, "test", &s, &request(3, 900000000, 0, 0, 1)).is_err());
    }
}
