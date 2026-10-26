//! Original-table merchant stock and atomic inventory/currency transactions.

use crate::{combat, lobby::Lobby, movement::Position, starter, world_entry::Session};
use reko_assets::{CellValue, Table};
use rusqlite::{Connection, params};
use std::{net::SocketAddr, path::Path};
use tokio::net::TcpStream;

pub(crate) struct Merchant {
    pub id: u16,
    pub(crate) prototype: u16,
    appearance: u16,
    pub(crate) name: String,
    group: u32,
    nation: u8,
    level: u8,
    pub(crate) kind: u8,
    pub(crate) position: Position,
    size: u16,
    weapons: [u32; 2],
    direction: u8,
    object: bool,
    object_status: u32,
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS world_merchants (
        id INTEGER PRIMARY KEY CHECK(id BETWEEN 11000 AND 32767),
        prototype INTEGER NOT NULL, appearance INTEGER NOT NULL, name TEXT NOT NULL,
        selling_group INTEGER NOT NULL, nation INTEGER NOT NULL, level INTEGER NOT NULL,
        kind INTEGER NOT NULL, zone INTEGER NOT NULL, x INTEGER NOT NULL, z INTEGER NOT NULL,
        y INTEGER NOT NULL DEFAULT 0);",
    )
}

pub(crate) fn load(db: &Connection, game: &Path) -> anyhow::Result<Vec<Merchant>> {
    let looks = Table::from_encrypted_bytes(&std::fs::read(game.join("Data/NPC_Looks.tbl"))?)?;
    let mut query=db.prepare("SELECT id,prototype,appearance,name,selling_group,nation,level,kind,zone,x,z,y FROM world_merchants ORDER BY id")?;
    let merchants = query
        .query_map([], |r| {
            Ok(Merchant {
                id: r.get(0)?,
                prototype: r.get(1)?,
                appearance: r.get(2)?,
                name: r.get(3)?,
                group: r.get(4)?,
                nation: r.get(5)?,
                level: r.get(6)?,
                kind: r.get(7)?,
                size: 100,
                weapons: [0; 2],
                direction: 0,
                object: false,
                object_status: 0,
                position: Position {
                    zone: r.get(8)?,
                    x: r.get(9)?,
                    z: r.get(10)?,
                    y: r.get(11)?,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for merchant in &merchants {
        anyhow::ensure!(
            merchant.name.len() <= 255 && matches!(merchant.kind, 21 | 22),
            "invalid merchant definition"
        );
        anyhow::ensure!(
            looks
                .rows
                .iter()
                .any(|r| r.first() == Some(&CellValue::U32(u32::from(merchant.appearance)))),
            "missing merchant appearance"
        );
    }
    Ok(merchants)
}

pub(crate) fn load_population(db: &Connection, game: &Path) -> anyhow::Result<Vec<Merchant>> {
    Ok(crate::population::load(db, game)?
        .into_iter()
        .filter(|s| !s.monster)
        .map(|s| Merchant {
            id: s.id,
            prototype: s.prototype,
            appearance: s.appearance,
            name: s.name,
            group: if s.prototype == 27011 && s.position.zone == 21 {
                255000
            } else {
                s.group
            },
            nation: s.nation,
            level: s.level,
            kind: if s.prototype == 27011 && s.position.zone == 21 {
                22
            } else {
                s.kind
            },
            position: s.position,
            size: s.size,
            weapons: s.weapons,
            direction: s.direction,
            object: false,
            object_status: 0,
        })
        .collect())
}

pub(crate) fn load_objects(db: &Connection) -> anyhow::Result<Vec<Merchant>> {
    let mut q = db.prepare("SELECT o.data,n.data FROM world_reference_rules o JOIN world_reference_rules n ON n.table_name='OBJECT_NPC' AND n.rule_id=CAST(json_extract(o.data,'$.sIndex') AS INTEGER) WHERE o.table_name='K_OBJECTPOS' AND CAST(json_extract(o.data,'$.Type') AS INTEGER)=8 ORDER BY o.rule_id")?;
    let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut objects = Vec::new();
    for (index, row) in rows.enumerate() {
        let (o, n) = row?;
        let o: serde_json::Value = serde_json::from_str(&o)?;
        let n: serde_json::Value = serde_json::from_str(&n)?;
        let number = |v: &serde_json::Value, k: &str| -> anyhow::Result<f64> {
            v[k].as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v[k].as_f64())
                .ok_or_else(|| anyhow::anyhow!("missing object field {k}"))
        };
        objects.push(Merchant {
            id: 30000u16
                .checked_add(index.try_into()?)
                .ok_or_else(|| anyhow::anyhow!("too many map objects"))?,
            prototype: number(&n, "sSid")? as u16,
            appearance: number(&n, "sPid")? as u16,
            name: n["strName"].as_str().unwrap_or_default().to_string(),
            kind: number(&n, "byType")? as u8,
            group: number(&n, "iSellingGroup")? as u32,
            nation: 0,
            level: number(&n, "sLevel")? as u8,
            size: number(&n, "sSize")? as u16,
            weapons: [0; 2],
            direction: 0,
            object: true,
            object_status: number(&o, "Status")? as u32,
            position: Position {
                zone: number(&o, "ZoneID")? as u8,
                x: (number(&o, "PosX")? * 10.0).round() as u16,
                z: (number(&o, "PosZ")? * 10.0).round() as u16,
                y: (number(&o, "PosY")? * 10.0).round() as i16,
            },
        });
    }
    Ok(objects)
}

impl Merchant {
    pub fn spawn(&self) -> Vec<u8> {
        let mut p = vec![0x0a, 1];
        p.extend(self.id.to_le_bytes());
        p.extend(self.prototype.to_le_bytes());
        p.extend(self.appearance.to_le_bytes());
        p.push(self.kind);
        p.extend(self.group.to_le_bytes());
        p.extend(self.size.to_le_bytes());
        p.extend(self.weapons[0].to_le_bytes());
        p.extend(self.weapons[1].to_le_bytes());
        p.push(self.name.len() as u8);
        p.extend(self.name.as_bytes());
        p.extend([self.nation, self.level]);
        p.extend(self.position.x.to_le_bytes());
        p.extend(self.position.z.to_le_bytes());
        p.extend(self.position.y.to_le_bytes());
        p.extend(self.object_status.to_le_bytes());
        p.push(u8::from(self.object));
        p.extend([0; 4]);
        p.push(self.direction);
        p
    }
    pub fn zone(&self) -> u8 {
        self.position.zone
    }
    pub fn visible_from(&self, position: Position) -> bool {
        crate::population::same_region_view(self.position, position)
    }
    pub(crate) fn nearby(&self, session: &Session) -> bool {
        // TODO(re): Replace the training interaction envelope with model radii.
        self.zone() == session.position.zone
            && f32::from(self.position.x.abs_diff(session.position.x))
                .hypot(f32::from(self.position.z.abs_diff(session.position.z)))
                <= 100.0
    }
}

type TradeResult = ([starter::Item; 28], u32, Vec<(usize, starter::Item)>, u32);
type RepairResult = ([starter::Item; 14], [starter::Item; 28], u32, u32);

fn repair(
    db: &Connection,
    merchant: &Merchant,
    catalog: &combat::Catalog,
    account: &str,
    session: &Session,
    payload: &[u8],
) -> anyhow::Result<RepairResult> {
    anyhow::ensure!(
        payload.len() == 7
            && payload[0] == 0x3b
            && session.entered
            && session.skills.hp > 0
            && merchant.kind == 22
            && merchant.nearby(session),
        "repair unavailable"
    );
    let district = payload[1];
    let pos = usize::from(payload[2]);
    let id = u32::from_le_bytes(payload[3..7].try_into()?);
    let tx = db.unchecked_transaction()?;
    let mut gear = starter::equipment(&tx, account, session.character.slot)?;
    let mut bag = starter::inventory(&tx, account, session.character.slot)?;
    let item = match district {
        1 => gear.get_mut(pos),
        2 => bag.get_mut(pos),
        _ => None,
    }
    .ok_or_else(|| anyhow::anyhow!("invalid repair slot"))?;
    anyhow::ensure!(
        id != 0 && item.id == id && item.count == 1,
        "repair ownership mismatch"
    );
    let (row, extension) = catalog
        .item(id)
        .ok_or_else(|| anyhow::anyhow!("unknown repair item"))?;
    let extension = extension.ok_or_else(|| anyhow::anyhow!("unknown repair extension"))?;
    anyhow::ensure!(
        combat::integer(row, 10) != 255,
        "consumable charges cannot be repaired"
    );
    let maximum = u16::try_from(combat::integer(row, 19) + combat::integer(extension, 12))?;
    anyhow::ensure!(
        maximum > 1 && item.durability <= maximum,
        "invalid repair durability"
    );
    let price = combat::integer(row, 20) as f32 * combat::integer(extension, 13) as f32;
    anyhow::ensure!(price.is_finite() && price >= 0.0, "invalid repair price");
    let missing = maximum - item.durability;
    let cost = (((f64::from(price) - 10.0) * f64::from(0.0001f32)
        + f64::from(price).powf(f64::from(0.82f32)))
        * f64::from(missing)
        / f64::from(maximum)) as f32;
    let cost = cost.max(0.0) as u32;
    let coins = starter::coins(&tx, account, session.character.slot)?;
    anyhow::ensure!(cost <= coins, "insufficient repair coins");
    item.durability = maximum;
    let (table, column) = if district == 1 {
        ("character_equipment", "equipment_slot")
    } else {
        ("character_inventory", "inventory_slot")
    };
    let updated = tx.execute(
        &format!("UPDATE {table} SET durability=?4 WHERE account=?1 AND slot=?2 AND {column}=?3 AND item_id=?5"),
        params![account, session.character.slot, pos, maximum, id],
    )?;
    anyhow::ensure!(updated == 1, "missing repair item");
    let coins = coins - cost;
    let updated = tx.execute(
        "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
        params![account, session.character.slot, coins],
    )?;
    anyhow::ensure!(updated == 1, "missing repair wallet");
    tx.commit()?;
    Ok((gear, bag, coins, cost))
}

fn trade(
    db: &Connection,
    merchant: &Merchant,
    catalog: &combat::Catalog,
    account: &str,
    session: &Session,
    payload: &[u8],
) -> anyhow::Result<TradeResult> {
    anyhow::ensure!(
        session.entered
            && session.skills.hp > 0
            && matches!(merchant.kind, 21 | 22)
            && merchant.nearby(session),
        "merchant unavailable"
    );
    let kind = *payload
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("missing trade type"))?;
    let start = if kind == 3 { 2 } else { 8 };
    anyhow::ensure!(
        matches!((kind, payload.len()), (1, 15) | (2, 15) | (3, 8)),
        "malformed trade"
    );
    if kind != 3 {
        anyhow::ensure!(
            u32::from_le_bytes(payload[2..6].try_into()?) == merchant.group
                && u16::from_le_bytes(payload[6..8].try_into()?) == merchant.id,
            "merchant mismatch"
        );
    }
    let id = u32::from_le_bytes(payload[start..start + 4].try_into()?);
    let pos = usize::from(payload[start + 4]);
    let tx = db.unchecked_transaction()?;
    let mut bag = starter::inventory(&tx, account, session.character.slot)?;
    let mut coins = starter::coins(&tx, account, session.character.slot)?;
    let mut transferred = 0;
    anyhow::ensure!(pos < bag.len() && id != 0, "invalid inventory slot");
    let mut changed = vec![pos];
    if kind == 3 {
        let dest = usize::from(payload[start + 5]);
        anyhow::ensure!(dest < bag.len() && bag[pos].id == id, "invalid move");
        bag.swap(pos, dest);
        changed.push(dest);
    } else {
        let quantity = u16::from_le_bytes(payload[start + 5..start + 7].try_into()?);
        anyhow::ensure!((1..=9999).contains(&quantity), "invalid quantity");
        let (row, extension) = catalog
            .item(id)
            .ok_or_else(|| anyhow::anyhow!("unknown item"))?;
        let extension = extension.ok_or_else(|| anyhow::anyhow!("unknown item extension"))?;
        let stackable = combat::integer(row, 23) != 0;
        anyhow::ensure!(stackable || quantity == 1, "nonstackable item");
        let price = u64::try_from(combat::integer(row, 20))?
            .checked_mul(u64::try_from(combat::integer(extension, 13))?)
            .ok_or_else(|| anyhow::anyhow!("price overflow"))?
            .checked_mul(u64::from(quantity))
            .ok_or_else(|| anyhow::anyhow!("price overflow"))?;
        if kind == 1 {
            anyhow::ensure!(
                combat::integer(row, 35) as u32 == merchant.group / 1000
                    && id % 1000 == merchant.group % 1000,
                "item not sold here"
            );
            anyhow::ensure!(
                bag[pos].id == 0 || (stackable && bag[pos].id == id),
                "inventory slot occupied"
            );
            let count = bag[pos]
                .count
                .checked_add(quantity)
                .filter(|n| *n <= 9999)
                .ok_or_else(|| anyhow::anyhow!("stack full"))?;
            anyhow::ensure!(price <= u64::from(coins), "insufficient coins");
            coins -= price as u32;
            transferred = price as u32;
            bag[pos] = starter::Item {
                id,
                durability: u16::try_from(
                    combat::integer(row, 19) + combat::integer(extension, 12),
                )?,
                count,
            };
            let weight = u32::from(catalog.bag_weight(&bag))
                + u32::from(
                    catalog
                        .calculate(&session.character, &session.progress, &session.equipment)?
                        .weight,
                );
            anyhow::ensure!(weight <= u32::from(session.combat.max_weight), "too heavy");
        } else {
            anyhow::ensure!(
                bag[pos].id == id && bag[pos].count >= quantity,
                "item ownership mismatch"
            );
            // Full-price sale flag is 1; ordinary shop sales return a quarter.
            let value = if combat::integer(row, 21) == 1 {
                price
            } else {
                price / 4
            };
            let total = u64::from(coins) + value;
            anyhow::ensure!(total <= i32::MAX as u64, "currency limit");
            coins = total as u32;
            transferred = value as u32;
            bag[pos].count -= quantity;
            if bag[pos].count == 0 {
                bag[pos] = starter::Item::default();
            }
        }
    }
    for &pos in &changed {
        tx.execute(
            "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
            params![account, session.character.slot, pos],
        )?;
        let item = bag[pos];
        if item.id != 0 {
            tx.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?1,?2,?3,?4,?5,?6)",params![account,session.character.slot,pos,item.id,item.durability,item.count])?;
        }
    }
    let wallet_rows = tx.execute(
        "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
        params![account, session.character.slot, coins],
    )?;
    anyhow::ensure!(wallet_rows == 1, "missing character wallet");
    tx.commit()?;
    Ok((
        bag,
        coins,
        changed.into_iter().map(|pos| (pos, bag[pos])).collect(),
        transferred,
    ))
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    payload: &[u8],
) {
    if payload.first() == Some(&0x20) {
        session.merchant = None;
        session.npc_dialog = None;
        if !matches!(payload.len(), 3 | 7) || !session.entered || session.skills.hp == 0 {
            return;
        }
        let id = u16::from_le_bytes([payload[1], payload[2]]);
        if let Some(merchant) = lobby
            .merchants
            .iter()
            .find(|m| m.id == id && m.nearby(session))
        {
            if matches!(merchant.kind, 21 | 22) {
                session.merchant = Some(id);
                let mut p = vec![if merchant.kind == 22 { 0x3a } else { 0x25 }];
                p.extend(merchant.group.to_le_bytes());
                super::send_response(sock, peer, &None, &p, "MERCHANT_OPEN").await;
            } else {
                if merchant.kind == 25 {
                    session.merchant = Some(id);
                    super::send_response(sock, peer, &None, &[0x3c, 0x1b], "CAPE_SERVICE_OPEN")
                        .await;
                    return;
                }
                if merchant.kind == 24 {
                    let mut request = vec![0x33];
                    request.extend(merchant.prototype.to_le_bytes());
                    request.extend(merchant.id.to_le_bytes());
                    crate::upgrading::handle(sock, peer, lobby, account, session, &request).await;
                    return;
                }
                if merchant.kind == 162 && merchant.prototype == 603 {
                    session.merchant = Some(id);
                    session.anvil = None;
                    session.npc_dialog = None;
                    let mut reply = vec![0x5b, 4];
                    reply.extend(id.to_le_bytes());
                    super::send_response(sock, peer, &None, &reply, "CHAOTIC_GENERATOR_OPEN").await;
                    return;
                }
                if merchant.kind == 31 {
                    session.merchant = Some(id);
                    super::send_response(sock, peer, &None, &[0x45, 0x10], "WAREHOUSE_REQUEST")
                        .await;
                    return;
                }
                crate::npc_scripts::handle(sock, peer, lobby, account, session, payload).await;
            }
        }
        return;
    }
    if payload.first() == Some(&0x3b) {
        let result = if let (Some(merchant), Some(catalog)) = (
            lobby
                .merchants
                .iter()
                .find(|m| Some(m.id) == session.merchant),
            lobby.combat.as_ref(),
        ) {
            repair(
                &lobby.db.lock().unwrap(),
                merchant,
                catalog,
                account,
                session,
                payload,
            )
        } else {
            Err(anyhow::anyhow!("open a nearby repair merchant first"))
        };
        let success = match result {
            Ok((gear, bag, coins, cost)) => {
                session.equipment = gear;
                session.inventory = bag;
                session.coins = coins;
                tracing::info!(%peer,account,character=%session.character.name,coins_changed=cost,balance=coins,"item repair committed");
                true
            }
            Err(error) => {
                tracing::info!(%peer,%error,"item repair refused");
                false
            }
        };
        let mut p = vec![0x3b, u8::from(success)];
        p.extend(session.coins.to_le_bytes());
        super::send_response(sock, peer, &None, &p, "ITEM_REPAIR_RESULT").await;
        if success && let Some(catalog) = lobby.combat.as_ref() {
            crate::durability::refresh(session, catalog, lobby.skills.as_ref());
            crate::durability::attributes(sock, peer, session).await;
        }
        return;
    }
    let result = if let (Some(merchant), Some(catalog)) = (
        lobby
            .merchants
            .iter()
            .find(|m| Some(m.id) == session.merchant),
        lobby.combat.as_ref(),
    ) {
        trade(
            &lobby.db.lock().unwrap(),
            merchant,
            catalog,
            account,
            session,
            payload,
        )
    } else {
        Err(anyhow::anyhow!("open a nearby merchant first"))
    };
    match result {
        Ok((bag, coins, _changed, transferred)) => {
            session.inventory = bag;
            session.coins = coins;
            let mut p = vec![0x21, if payload[1] == 3 { 3 } else { 1 }];
            if payload[1] != 3 {
                p.extend(coins.to_le_bytes());
                p.extend(transferred.to_le_bytes());
            }
            super::send_response(sock, peer, &None, &p, "MERCHANT_RESULT").await;
            tracing::info!(%peer,account,character=%session.character.name,merchant=session.merchant,action=payload[1],coins_changed=transferred,balance=coins,"merchant transaction committed");
            if let Some(catalog) = lobby.combat.as_ref()
                && let Ok(mut base) =
                    catalog.calculate(&session.character, &session.progress, &session.equipment)
            {
                base.weight = base.weight.saturating_add(catalog.bag_weight(&bag));
                session.base_combat = base;
                crate::utility::refresh(session, lobby.skills.as_ref(), lobby.combat.as_ref());
                let mut p = vec![0x54];
                p.extend(base.weight.to_le_bytes());
                super::send_response(sock, peer, &None, &p, "ITEM_WEIGHT").await;
            }
        }
        Err(error) => {
            tracing::info!(%peer,%error,"merchant transaction refused");
            let p = if payload.get(1) == Some(&3) {
                vec![0x21, 4]
            } else {
                vec![0x21, 0, 2]
            };
            super::send_response(sock, peer, &None, &p, "MERCHANT_FAILED").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_potion_prices_weights_and_speed_cost_apply_to_real_trade_and_capacity() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let rules: serde_json::Value =
            serde_json::from_str(include_str!("../../../config/item-balance.json")).unwrap();
        let db = lobby.db.lock().unwrap();
        let catalog = lobby.combat.as_mut().unwrap();
        let mut expected_weight = 0u16;
        for rule in rules["items"].as_array().unwrap() {
            let id = rule["id"].as_u64().unwrap() as u32;
            let price = rule["price"].as_u64().unwrap() as u32;
            let weight = rule["weight"].as_u64().unwrap() as u16;
            let (base, _) = catalog.item(id).unwrap();
            let old_price = combat::integer(base, 20) as u32;
            let old_weight = combat::integer(base, 18) as u16;
            if id == 800015000 {
                assert_eq!(price, 1000);
                assert_eq!(weight, old_weight);
            } else {
                assert_eq!(weight * 2, old_weight);
                let percent = if matches!(id, 389014000 | 389019000 | 389020000) {
                    75
                } else {
                    20
                };
                assert_eq!(price, old_price * percent / 100);
            }
            db.execute(
                "INSERT INTO world_reference_rules VALUES('ITEM_ECONOMY',?1,?2)",
                rusqlite::params![
                    id,
                    serde_json::json!({"price":price,"weight":weight}).to_string()
                ],
            )
            .unwrap();
            expected_weight += weight * if id == 800015000 { 1 } else { 2 };
        }
        catalog.load_item_economy(&db).unwrap();
        catalog.load_item_economy(&db).unwrap();
        s.combat.max_weight = 7000;
        let mut merchant = Merchant {
            id: 11000,
            prototype: 506,
            appearance: 2040,
            name: "Test".into(),
            group: 253000,
            nation: 3,
            level: 50,
            kind: 21,
            position: s.position,
            size: 100,
            weapons: [0; 2],
            direction: 0,
            object: false,
            object_status: 0,
        };
        let mut total_spent = 0;
        for (slot, rule) in rules["items"].as_array().unwrap().iter().enumerate() {
            let id = rule["id"].as_u64().unwrap() as u32;
            let price = rule["price"].as_u64().unwrap() as u32;
            let count = if id == 800015000 { 1u16 } else { 2 };
            merchant.group = if id == 800015000 { 255000 } else { 253000 };
            let mut packet = vec![0x21, 1];
            packet.extend(merchant.group.to_le_bytes());
            packet.extend(merchant.id.to_le_bytes());
            packet.extend(id.to_le_bytes());
            packet.push(slot as u8);
            packet.extend(count.to_le_bytes());
            let (bag, coins, _, spent) =
                trade(&db, &merchant, catalog, "test", &s, &packet).unwrap();
            total_spent += price * u32::from(count);
            assert_eq!(spent, price * u32::from(count));
            assert_eq!(coins, 1000000 - total_spent);
            assert_eq!(bag[slot].count, count);
        }
        let bag = starter::inventory(&db, "test", 0).unwrap();
        assert_eq!(catalog.bag_weight(&bag), expected_weight);
        let before = starter::coins(&db, "test", 0).unwrap();
        s.combat.max_weight = expected_weight;
        let mut over = vec![0x21, 1];
        over.extend(255000u32.to_le_bytes());
        over.extend(11000u16.to_le_bytes());
        over.extend(800015000u32.to_le_bytes());
        over.push(11);
        over.extend(1u16.to_le_bytes());
        assert!(trade(&db, &merchant, catalog, "test", &s, &over).is_err());
        assert_eq!(starter::coins(&db, "test", 0).unwrap(), before);
        assert_eq!(starter::inventory(&db, "test", 0).unwrap(), bag);
        over[1] = 2;
        over[12] = 10;
        let (_, coins, _, received) = trade(&db, &merchant, catalog, "test", &s, &over).unwrap();
        assert_eq!((received, coins), (250, before + 250));
    }
    #[tokio::test]
    async fn habice_opens_native_cape_service_only_when_nearby() {
        use tokio::io::AsyncReadExt;
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            crate::population::tests::seed(&db);
            db.execute("UPDATE world_spawns SET monster=0,prototype=26031,appearance=22000,name='[Mantle]Habice',kind=25,zone=?1,x0=?2,x1=?2,z0=?3,z1=?3",params![s.position.zone,s.position.x,s.position.z]).unwrap();
            lobby.merchants = load_population(&db, &reko_common::game_dir()).unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut request = vec![0x20];
        request.extend(12000u16.to_le_bytes());
        handle(&mut socket, peer, &lobby, "test", &mut s, &request).await;
        let mut packet = [0; 8];
        client.read_exact(&mut packet).await.unwrap();
        assert_eq!(packet, crate::frame_packet(&[0x3c, 0x1b]).as_slice());
        assert_eq!(s.merchant, Some(12000));
        assert!(s.npc_dialog.is_none());
        s.position.x += 1000;
        handle(&mut socket, peer, &lobby, "test", &mut s, &request).await;
        assert!(s.merchant.is_none());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), client.read_u8())
                .await
                .is_err()
        );
        assert_eq!(
            starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            1000000
        );
    }
    #[test]
    fn anvil_object_registers_native_resource_instead_of_character_model() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("world_services.sql"))
            .unwrap();
        db.execute("INSERT INTO world_reference_rules VALUES ('K_OBJECTPOS',2105001,?1)",[r#"{"ZoneID":"21","sIndex":"5001","Type":"8","Status":"1","PosX":"337.887726","PosY":"13.5151672","PosZ":"389.9475"}"#]).unwrap();
        db.execute("INSERT INTO world_reference_rules VALUES ('OBJECT_NPC',5001,?1)",[r#"{"sSid":"5001","sPid":"5001","strName":"Magic anvil","sSize":"100","byType":"24","iSellingGroup":"0","sLevel":"60"}"#]).unwrap();
        let objects = load_objects(&db).unwrap();
        assert_eq!(objects.len(), 1);
        let p = objects[0].spawn();
        assert_eq!(&p[4..9], &[0x89, 0x13, 0x89, 0x13, 24]);
        assert_eq!(p[p.len() - 6], 1);
        assert_eq!(&p[p.len() - 10..p.len() - 6], &1u32.to_le_bytes());
        assert_eq!((objects[0].position.x, objects[0].position.z), (3379, 3899));
    }

    #[tokio::test]
    async fn shop_acknowledgements_do_not_duplicate_client_owned_inventory_icons() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        lobby.merchants.push(Merchant {
            id: 11000,
            prototype: 506,
            appearance: 2040,
            name: "Test".into(),
            group: 253000,
            nation: 3,
            level: 50,
            kind: 21,
            position: s.position,
            size: 100,
            weapons: [0; 2],
            direction: 0,
            object: false,
            object_status: 0,
        });
        s.merchant = Some(11000);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        use tokio::io::AsyncReadExt;
        for (kind, pos, dest) in [(1u8, 0u8, 0u8), (3, 0, 1), (2, 1, 0)] {
            let mut p = vec![0x21, kind];
            if kind != 3 {
                p.extend(253000u32.to_le_bytes());
                p.extend(11000u16.to_le_bytes());
            }
            p.extend(389014000u32.to_le_bytes());
            p.push(pos);
            if kind == 3 {
                p.push(dest);
            } else {
                p.extend(1u16.to_le_bytes());
            }
            handle(&mut socket, peer, &lobby, "test", &mut s, &p).await;
            let mut header = [0u8; 4];
            client.read_exact(&mut header).await.unwrap();
            let n = u16::from_le_bytes([header[2], header[3]]) as usize;
            let mut reply = vec![0; n + 2];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply[..2], &[0x21, if kind == 3 { 3 } else { 1 }]);
            let mut weight = [0u8; 9];
            client.read_exact(&mut weight).await.unwrap();
            assert_eq!(weight[4], 0x54);
            let mut extra = [0u8; 1];
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(30),
                    client.read_exact(&mut extra)
                )
                .await
                .is_err(),
                "shop inventory belongs to the pending trade UI; no parallel3D updates"
            );
            assert_eq!(
                starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
                s.inventory
            );
        }
        assert!(s.inventory.iter().all(|i| i.id == 0));
    }

    #[test]
    fn merchant_transactions_reject_forgery_and_preserve_items_and_currency() {
        let game = reko_common::game_dir();
        let rules = Path::new("../../logs/reference-coefficients.csv");
        if !game.join("Data/item_org_us.tbl").exists() || !rules.exists() {
            return;
        }
        let catalog = combat::Catalog::load(&game, rules).unwrap();
        let db = Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "test").unwrap();
        crate::characters::set_nation(&db, "test", 1).unwrap();
        let c = crate::characters::Character {
            slot: 0,
            name: "Buyer".into(),
            race: 1,
            class: 106,
            face: 0,
            hair: 0,
            stats: [65, 65, 60, 50, 50],
        };
        db.execute(
            "INSERT INTO lobby_characters VALUES ('test',0,'Buyer',1,106,0,0,?1)",
            [c.stats.as_slice()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO character_wallets VALUES ('test',0,1000000)",
            [],
        )
        .unwrap();
        let mut s = Session::new(c).unwrap();
        s.entered = true;
        s.progress.level = 80;
        s.combat.max_weight = 7000;
        let merchant = Merchant {
            id: 11000,
            prototype: 506,
            appearance: 2040,
            name: "Test".into(),
            group: 253000,
            nation: 3,
            level: 50,
            kind: 21,
            position: s.position,
            size: 100,
            weapons: [0; 2],
            direction: 0,
            object: false,
            object_status: 0,
        };
        let buy = |id: u32, count: u16| {
            let mut p = vec![0x21, 1];
            p.extend(253000u32.to_le_bytes());
            p.extend(11000u16.to_le_bytes());
            p.extend(id.to_le_bytes());
            p.push(0);
            p.extend(count.to_le_bytes());
            p
        };
        let (bag, coins, _, spent) =
            trade(&db, &merchant, &catalog, "test", &s, &buy(389014000, 10)).unwrap();
        assert_eq!((bag[0].count, coins), (10, 930000));
        assert_eq!(spent, 70000);
        for invalid in [
            buy(389014000, 9999),
            buy(391010000, 1),
            buy(389014001, 1),
            buy(389014000, 0),
        ] {
            assert!(trade(&db, &merchant, &catalog, "test", &s, &invalid).is_err());
        }
        assert_eq!(starter::coins(&db, "test", 0).unwrap(), 930000);
        assert_eq!(starter::inventory(&db, "test", 0).unwrap()[0].count, 10);
        let mut sell = buy(389014000, 2);
        sell[1] = 2;
        let mut forged = sell.clone();
        forged[6] ^= 1;
        assert!(trade(&db, &merchant, &catalog, "test", &s, &forged).is_err());
        let (bag, coins, _, received) = trade(&db, &merchant, &catalog, "test", &s, &sell).unwrap();
        assert_eq!((bag[0].count, coins), (8, 933500));
        assert_eq!(received, 3500);
        let tinker = Merchant {
            kind: 22,
            ..merchant
        };
        let id = 110110001u32;
        db.execute(
            "INSERT INTO character_equipment VALUES ('test',0,6,?1,0)",
            [id],
        )
        .unwrap();
        let mut request = vec![0x3b, 1, 6];
        request.extend(id.to_le_bytes());
        let mut forged = request.clone();
        forged[3] ^= 1;
        assert!(repair(&db, &tinker, &catalog, "test", &s, &forged).is_err());
        db.execute("UPDATE character_wallets SET coins=0", [])
            .unwrap();
        assert!(repair(&db, &tinker, &catalog, "test", &s, &request).is_err());
        assert_eq!(starter::equipment(&db, "test", 0).unwrap()[6].durability, 0);
        db.execute("UPDATE character_wallets SET coins=933500", [])
            .unwrap();
        let (gear, _, coins, cost) = repair(&db, &tinker, &catalog, "test", &s, &request).unwrap();
        assert!(gear[6].durability > 1 && cost > 0);
        assert_eq!(coins, 933500 - cost);
        assert_eq!(starter::equipment(&db, "test", 0).unwrap()[6], gear[6]);
        let (_, _, repeated_coins, repeated_cost) =
            repair(&db, &tinker, &catalog, "test", &s, &request).unwrap();
        assert_eq!((repeated_coins, repeated_cost), (coins, 0));
        db.execute(
            "INSERT INTO character_inventory VALUES ('test',0,1,?1,0,1)",
            [id],
        )
        .unwrap();
        request[1] = 2;
        request[2] = 1;
        let (_, repaired_bag, remaining, bag_cost) =
            repair(&db, &tinker, &catalog, "test", &s, &request).unwrap();
        assert_eq!(repaired_bag[1].durability, gear[6].durability);
        assert_eq!(bag_cost, cost);
        assert_eq!(remaining, coins - cost);
        assert_eq!(
            starter::inventory(&db, "test", 0).unwrap()[1],
            repaired_bag[1]
        );
        s.position.zone = 1;
        assert!(repair(&db, &tinker, &catalog, "test", &s, &request).is_err());
        assert!(trade(&db, &tinker, &catalog, "test", &s, &buy(389014000, 1)).is_err());
    }
}
