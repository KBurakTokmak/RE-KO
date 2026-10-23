//! Native fragment exchange with primary data weights and atomic rewards.
use crate::{combat, inventory, lobby::Lobby, starter, world_entry::Session};
use std::{
    net::SocketAddr,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::net::TcpStream;
static SEED: AtomicU64 = AtomicU64::new(1);

fn number(row: &serde_json::Value, key: &str) -> anyhow::Result<u32> {
    row[key]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| row[key].as_u64().and_then(|v| v.try_into().ok()))
        .ok_or_else(|| anyhow::anyhow!("Missing fragment exchange field {key}"))
}
fn pool(
    db: &rusqlite::Connection,
    catalog: &combat::Catalog,
    fragment: u32,
) -> anyhow::Result<Vec<(u32, u32)>> {
    anyhow::ensure!(
        (389160000..=389166000).contains(&fragment) && fragment.is_multiple_of(1000),
        "Unsupported fragment"
    );
    let mut query=db.prepare("SELECT data FROM world_reference_rules WHERE table_name='ITEM_EXCHANGE' AND CAST(json_extract(data,'$.nOriginItemNum1') AS INTEGER)=?1 AND CAST(json_extract(data,'$.bRandomFlag') AS INTEGER)=101 ORDER BY rule_id")?;
    let mut choices = Vec::new();
    for text in query.query_map([fragment], |r| r.get::<_, String>(0))? {
        let row: serde_json::Value = serde_json::from_str(&text?)?;
        anyhow::ensure!(
            number(&row, "nOriginItemCount1")? == 1,
            "Invalid fragment cost"
        );
        for i in 2..=11 {
            anyhow::ensure!(
                number(&row, &format!("nOriginItemNum{i}"))? == 0
                    && number(&row, &format!("nOriginItemCount{i}"))? == 0,
                "Unexpected secondary fragment cost"
            );
        }
        for i in 1..=5 {
            let id = number(&row, &format!("nExchangeItemNum{i}"))?;
            let weight = number(&row, &format!("nExchangeItemCount{i}"))?;
            if weight == 0 {
                continue;
            }
            anyhow::ensure!(
                catalog.item(id).is_some_and(|(_, ext)| ext.is_some()),
                "Fragment reward {id} is absent from the original1453 client"
            );
            choices.push((id, weight));
        }
    }
    anyhow::ensure!(
        choices.iter().map(|(_, w)| u64::from(*w)).sum::<u64>() == 10000,
        "Fragment reward weights must total10000"
    );
    Ok(choices)
}
fn give(
    bag: &mut [starter::Item; 28],
    catalog: &combat::Catalog,
    id: u32,
) -> anyhow::Result<usize> {
    let (row, ext) = catalog
        .item(id)
        .ok_or_else(|| anyhow::anyhow!("Unknown fragment reward"))?;
    let ext = ext.ok_or_else(|| anyhow::anyhow!("Unknown fragment reward extension"))?;
    let stackable = combat::integer(row, 23) != 0;
    let slot = bag
        .iter()
        .position(|i| stackable && i.id == id && i.count < 9999)
        .or_else(|| bag.iter().position(|i| i.id == 0))
        .ok_or_else(|| anyhow::anyhow!("Inventory is full"))?;
    let item = &mut bag[slot];
    if item.id == 0 {
        *item = starter::Item {
            id,
            durability: (combat::integer(row, 19) + combat::integer(ext, 12)).clamp(0, 65535)
                as u16,
            count: 0,
        };
    }
    item.count += 1;
    Ok(slot)
}
struct Exchange {
    before: [starter::Item; 28],
    bag: [starter::Item; 28],
    reply: Vec<u8>,
    effect: Vec<u8>,
}
fn attempt(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    p: &[u8],
    roll: u32,
) -> anyhow::Result<Exchange> {
    anyhow::ensure!(
        p.len() == 9
            && p[..2] == [0x5b, 5]
            && s.entered
            && !s.zone_loading
            && s.skills.hp > 0
            && roll < 10000,
        "Invalid fragment exchange request"
    );
    let npc = u16::from_le_bytes([p[2], p[3]]);
    let fragment = u32::from_le_bytes(p[4..8].try_into()?);
    anyhow::ensure!(
        s.merchant == Some(npc)
            && lobby
                .merchants
                .iter()
                .any(|m| m.id == npc && m.prototype == 603 && m.kind == 162 && m.nearby(s)),
        "Open a nearby Chaotic Generator first"
    );
    let catalog = lobby
        .combat
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Item catalog unavailable"))?;
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let choices = pool(&tx, catalog, fragment)?;
    let mut cumulative = 0;
    let reward = choices
        .iter()
        .find_map(|(id, weight)| {
            cumulative += weight;
            (roll < cumulative).then_some(*id)
        })
        .unwrap();
    let before = starter::inventory(&tx, account, s.character.slot)?;
    let source = usize::from(p[8]);
    let mut bag = before;
    let payment = bag
        .get_mut(source)
        .filter(|i| i.id == fragment && i.count > 0)
        .ok_or_else(|| anyhow::anyhow!("You do not have this fragment in the selected slot"))?;
    payment.count -= 1;
    if payment.count == 0 {
        tx.execute(
            "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
            rusqlite::params![account, s.character.slot, source],
        )?;
        *payment = starter::Item::default();
    } else {
        tx.execute("UPDATE character_inventory SET count=?4 WHERE account=?1 AND slot=?2 AND inventory_slot=?3",rusqlite::params![account,s.character.slot,source,payment.count])?;
    }
    let destination = give(&mut bag, catalog, reward)?;
    let gear_weight = catalog
        .calculate(&s.character, &s.progress, &s.equipment)?
        .weight;
    anyhow::ensure!(
        u32::from(gear_weight) + u32::from(catalog.bag_weight(&bag))
            <= u32::from(s.combat.max_weight),
        "Reward exceeds your weight limit"
    );
    let item = bag[destination];
    tx.execute("INSERT INTO character_inventory VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(account,slot,inventory_slot) DO UPDATE SET item_id=excluded.item_id,durability=excluded.durability,count=excluded.count",rusqlite::params![account,s.character.slot,destination,item.id,item.durability,item.count])?;
    let class: u8 = tx
        .query_row(
            "SELECT item_class FROM world_upgrade_item_types WHERE item_id=?1",
            [reward / 1000 * 1000],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let effect = if class == 4 {
        3
    } else if matches!(class, 5 | 11 | 12) {
        2
    } else {
        1
    };
    let mut reply = vec![0x5b, 5, 1];
    reply.extend(reward.to_le_bytes());
    reply.push(destination as u8);
    reply.extend(fragment.to_le_bytes());
    reply.extend([source as u8, effect]);
    let mut event = vec![0x33, 9, effect];
    event.extend(npc.to_le_bytes());
    tx.commit()?;
    Ok(Exchange {
        before,
        bag,
        reply,
        effect: event,
    })
}
pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) {
    let mut seed = crate::skills::now_ms() ^ SEED.fetch_add(0x9e3779b97f4a7c15, Ordering::Relaxed);
    let result = attempt(lobby, account, s, p, crate::loot::roll(&mut seed, 0, 9999));
    match result {
        Ok(result) => {
            s.inventory = result.bag;
            if let Some(update) = inventory::reward_packet(&result.before, &result.bag) {
                crate::send_response(sock, peer, &None, &update, "FRAGMENT_INVENTORY").await;
            }
            crate::send_response(sock, peer, &None, &result.reply, "FRAGMENT_RESULT").await;
            crate::send_response(sock, peer, &None, &result.effect, "FRAGMENT_EFFECT").await;
            if let Some(c) = lobby.combat.as_ref() {
                crate::durability::refresh(s, c, lobby.skills.as_ref());
                let mut weight = vec![0x54];
                weight.extend(s.combat.weight.to_le_bytes());
                crate::send_response(sock, peer, &None, &weight, "FRAGMENT_WEIGHT").await;
            }
        }
        Err(error) => {
            tracing::info!(%peer,%error,"fragment exchange refused atomically");
            let mut reply = vec![0x5b, 5, 0];
            reply.extend([0; 5]);
            reply.extend(p.get(4..8).unwrap_or(&[0; 4]));
            reply.extend([p.get(8).copied().filter(|slot| *slot < 28).unwrap_or(0), 0]);
            crate::send_response(sock, peer, &None, &reply, "FRAGMENT_REFUSED").await;
            let notice = crate::chat::packet(
                7,
                s.character.nation(),
                u16::MAX,
                "",
                error.to_string().as_bytes(),
            );
            crate::send_response(sock, peer, &None, &notice, "FRAGMENT_NOTICE").await;
        }
    }
}

#[cfg(test)]
#[path = "chaotic_tests.rs"]
mod tests;
