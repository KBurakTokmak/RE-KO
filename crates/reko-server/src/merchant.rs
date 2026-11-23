//! Native player selling merchants. Listings reserve existing inventory slots.
use crate::{
    chat, combat, inventory,
    lobby::Lobby,
    movement::Position,
    social::{self, Hub, Player},
    starter::{self, Item},
    trade, wallet,
};
use rusqlite::{Connection, params};
use std::collections::hash_map::Entry;

const SLOTS: usize = 12;
const BAG_OFFSET: u8 = 14;

#[derive(Clone, Copy)]
struct Listing {
    source: usize,
    original: Item,
    count: u16,
    price: u32,
}

pub(crate) struct Stall {
    position: Position,
    published: bool,
    listings: [Option<Listing>; SLOTS],
}

fn answer(op: u8, result: i16) -> Vec<u8> {
    let mut packet = vec![0x68, op];
    packet.extend(result.to_le_bytes());
    packet
}

fn eligible(player: &Player) -> bool {
    player.s.entered && !player.s.zone_loading && player.s.skills.hp > 0
}

fn reachable(hub: &Hub, buyer: u16, seller: u16) -> bool {
    buyer != seller
        && hub.can_see(buyer, seller)
        && hub
            .players
            .get(&buyer)
            .zip(hub.players.get(&seller))
            .is_some_and(|(a, b)| {
                eligible(a) && eligible(b) && social::near(a.s.position, b.s.position, 10)
            })
        && hub.stalls.get(&seller).is_some_and(|stall| stall.published)
}

pub(crate) fn busy(hub: &Hub, id: u16) -> bool {
    hub.stalls.contains_key(&id) || hub.merchant_views.contains_key(&id)
}

pub(crate) fn announce(hub: &mut Hub, id: u16, text: &[u8]) {
    if !hub.stalls.get(&id).is_some_and(|stall| stall.published) {
        return;
    }
    let seller = &hub.players[&id].s;
    let packet = chat::packet(
        chat::MERCHANT_CHANNEL,
        seller.character.nation(),
        id,
        &seller.character.name,
        text,
    );
    hub.send(id, packet.clone());
    hub.broadcast_visual(id, None, None, &packet);
}

fn stop_viewing(hub: &mut Hub, id: u16) {
    if hub.merchant_views.remove(&id).is_some() {
        hub.send(id, answer(8, 1));
    }
}

pub(crate) fn close(hub: &mut Hub, id: u16) {
    stop_viewing(hub, id);
    if let Some(stall) = hub.stalls.remove(&id) {
        let mut packet = vec![0x68, 2];
        packet.extend(id.to_le_bytes());
        hub.send(id, packet.clone());
        if stall.published {
            hub.broadcast_visual(id, None, None, &packet);
        }
        let viewers: Vec<_> = hub
            .merchant_views
            .iter()
            .filter_map(|(&viewer, &seller)| (seller == id).then_some(viewer))
            .collect();
        for viewer in viewers {
            stop_viewing(hub, viewer);
        }
    }
}

pub(crate) fn maintain(hub: &mut Hub, id: u16) {
    if hub.stalls.get(&id).is_some_and(|stall| {
        let player = &hub.players[&id];
        !eligible(player)
            || stall.position.zone != player.s.position.zone
            || stall.position.x != player.s.position.x
            || stall.position.z != player.s.position.z
            || !hub.refresh.contains(&id)
                && stall
                    .listings
                    .iter()
                    .flatten()
                    .any(|listing| player.s.inventory[listing.source] != listing.original)
    }) {
        close(hub, id);
    }
    let stale: Vec<_> = hub
        .merchant_views
        .iter()
        .filter_map(|(&buyer, &seller)| {
            ((buyer == id || seller == id) && !reachable(hub, buyer, seller)).then_some(buyer)
        })
        .collect();
    for buyer in stale {
        stop_viewing(hub, buyer);
    }
}

pub(crate) fn before_action(hub: &mut Hub, id: u16, opcode: u8) {
    if matches!(
        opcode,
        0x08 | 0x0c
            | 0x12
            | 0x1f
            | 0x20
            | 0x21
            | 0x24
            | 0x26
            | 0x27
            | 0x28
            | 0x30
            | 0x31
            | 0x32
            | 0x33
            | 0x3b
            | 0x3c
            | 0x3f
            | 0x45
            | 0x48
            | 0x4b
            | 0x55
            | 0x5b
            | 0x64
            | 0x70
    ) {
        close(hub, id);
    }
}

fn append_icons(packet: &mut Vec<u8>, stall: &Stall) {
    for entry in &stall.listings[..4] {
        packet.extend(entry.map_or(0, |listing| listing.original.id).to_le_bytes());
    }
}

pub(crate) fn show_region(hub: &mut Hub, viewer: u16, sellers: impl Iterator<Item = u16>) {
    let mut packet = vec![0x69, 1, 0, 0];
    let mut count = 0u16;
    for seller in sellers {
        if let Some(stall) = hub.stalls.get(&seller).filter(|s| s.published) {
            count += 1;
            packet.extend(seller.to_le_bytes());
            packet.push(0); // Normal selling merchant.
            append_icons(&mut packet, stall);
        }
    }
    if count > 0 {
        packet[2..4].copy_from_slice(&count.to_le_bytes());
        hub.send(viewer, packet);
    }
}

fn open(hub: &mut Hub, id: u16) {
    let player = &hub.players[&id];
    let position = player.s.position;
    let result = if !eligible(player) {
        -2
    } else if trade::busy(hub, id) || hub.merchant_views.contains_key(&id) {
        -3
    } else {
        match hub.stalls.entry(id) {
            Entry::Occupied(_) => -4,
            Entry::Vacant(_) if !matches!(position.zone, 1 | 2 | 21) => -5,
            Entry::Vacant(_) if player.s.progress.level < 20 => 30,
            Entry::Vacant(entry) => {
                entry.insert(Stall {
                    position,
                    published: false,
                    listings: [None; SLOTS],
                });
                1
            }
        }
    };
    hub.send(id, answer(1, result));
}

fn add(
    hub: &mut Hub,
    id: u16,
    fields: &[u8],
    db: &Connection,
    catalog: &combat::Catalog,
) -> anyhow::Result<Vec<u8>> {
    let item = u32::from_le_bytes(fields[..4].try_into()?);
    let count = u16::from_le_bytes(fields[4..6].try_into()?);
    let price = u32::from_le_bytes(fields[6..10].try_into()?);
    let [source, index] = fields[10..12].try_into()?;
    let stall = hub
        .stalls
        .get_mut(&id)
        .filter(|stall| !stall.published)
        .ok_or_else(|| anyhow::anyhow!("Open a selling merchant first."))?;
    let player = &hub.players[&id];
    let bag = starter::inventory(db, &player.account, player.s.character.slot)?;
    let original = *bag
        .get(usize::from(source))
        .ok_or_else(|| anyhow::anyhow!("Invalid bag slot."))?;
    anyhow::ensure!(
        (usize::from(index) < SLOTS)
            && stall.listings[usize::from(index)].is_none()
            && !stall
                .listings
                .iter()
                .flatten()
                .any(|line| line.source == usize::from(source)),
        "Invalid or occupied merchant slot."
    );
    let stackable = trade::tradable(catalog, item)?;
    anyhow::ensure!(
        original.id == item
            && count > 0
            && count <= original.count
            && (stackable || count == 1)
            && price > 0
            && u64::from(price) * u64::from(count) <= u64::from(trade::COIN_MAX),
        "Invalid item, quantity or price."
    );
    stall.listings[usize::from(index)] = Some(Listing {
        source: source.into(),
        original,
        count,
        price,
    });
    let mut packet = answer(3, 1);
    packet.extend(item.to_le_bytes());
    packet.extend(count.to_le_bytes());
    packet.extend(original.durability.to_le_bytes());
    packet.extend(price.to_le_bytes());
    packet.extend([source + BAG_OFFSET, index]);
    Ok(packet)
}

fn publish(hub: &mut Hub, id: u16, advert: &[u8]) -> anyhow::Result<()> {
    let stall = hub
        .stalls
        .get_mut(&id)
        .filter(|stall| !stall.published)
        .ok_or_else(|| anyhow::anyhow!("Open a selling merchant first."))?;
    anyhow::ensure!(
        advert.len() <= 40
            && advert.iter().all(|&b| b >= 32 && b != 127)
            && stall.listings.iter().any(Option::is_some),
        "Add an item and keep the merchant message within 40 characters."
    );
    stall.published = true;
    let mut packet = answer(7, 1);
    packet.extend((advert.len() as u16).to_le_bytes());
    packet.extend(advert);
    packet.extend(id.to_le_bytes());
    packet.push(0);
    append_icons(&mut packet, stall);
    hub.send(id, packet.clone());
    hub.broadcast_visual(id, None, None, &packet);
    Ok(())
}

fn list(hub: &mut Hub, id: u16, seller: u16) -> anyhow::Result<()> {
    anyhow::ensure!(
        !hub.stalls.contains_key(&id) && !trade::busy(hub, id) && reachable(hub, id, seller),
        "This merchant is no longer available."
    );
    let mut packet = answer(5, 1);
    packet.extend(seller.to_le_bytes());
    for entry in &hub.stalls[&seller].listings {
        if let Some(line) = entry {
            packet.extend(line.original.id.to_le_bytes());
            packet.extend(line.count.to_le_bytes());
            packet.extend(line.original.durability.to_le_bytes());
            packet.extend(line.price.to_le_bytes());
            packet.extend([0; 4]);
        } else {
            packet.extend([0; 16]);
        }
    }
    hub.merchant_views.insert(id, seller);
    hub.send(id, packet);
    Ok(())
}

fn save_slot(db: &Connection, player: &Player, slot: usize, item: Item) -> rusqlite::Result<()> {
    let account = &player.account;
    let character = player.s.character.slot;
    if item.count == 0 {
        db.execute(
            "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
            params![account, character, slot],
        )?;
    } else {
        db.execute(
            "INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(account,slot,inventory_slot) DO UPDATE
             SET item_id=excluded.item_id,durability=excluded.durability,count=excluded.count",
            params![
                account,
                character,
                slot,
                item.id,
                item.durability,
                item.count
            ],
        )?;
    }
    Ok(())
}

fn buy(
    hub: &mut Hub,
    buyer: u16,
    fields: &[u8],
    db: &mut Connection,
    catalog: &combat::Catalog,
) -> anyhow::Result<()> {
    let seller = hub
        .merchant_views
        .get(&buyer)
        .copied()
        .filter(|&seller| reachable(hub, buyer, seller))
        .ok_or_else(|| anyhow::anyhow!("Open a nearby selling merchant first."))?;
    let item = u32::from_le_bytes(fields[..4].try_into()?);
    let count = u16::from_le_bytes(fields[4..6].try_into()?);
    let (index, dest) = (usize::from(fields[6]), usize::from(fields[7]));
    let mut listing = hub.stalls[&seller]
        .listings
        .get(index)
        .copied()
        .flatten()
        .filter(|line| line.original.id == item && count > 0 && count <= line.count)
        .ok_or_else(|| anyhow::anyhow!("This item or quantity is no longer listed."))?;
    let [a, b] = [&hub.players[&buyer], &hub.players[&seller]];
    let name = a.s.character.name.clone();
    let tx = db.transaction()?;
    let mut buyer_bag = starter::inventory(&tx, &a.account, a.s.character.slot)?;
    let buyer_before = buyer_bag;
    let mut seller_bag = starter::inventory(&tx, &b.account, b.s.character.slot)?;
    anyhow::ensure!(
        seller_bag[listing.source] == listing.original,
        "The listed inventory changed."
    );
    let previous = *buyer_bag
        .get(dest)
        .ok_or_else(|| anyhow::anyhow!("Invalid destination slot."))?;
    let stack = trade::tradable(catalog, item)?;
    anyhow::ensure!(
        previous.id == 0
            || stack && previous.id == item && previous.durability == listing.original.durability,
        "The destination slot is occupied."
    );
    let total = previous
        .count
        .checked_add(count)
        .filter(|&total| {
            total
                <= if stack {
                    trade::stack_limit(catalog, item)
                } else {
                    1
                }
        })
        .ok_or_else(|| anyhow::anyhow!("The item stack is full."))?;
    buyer_bag[dest] = Item {
        count: total,
        ..listing.original
    };
    let mut weight = u64::from(
        catalog
            .calculate(&a.s.character, &a.s.progress, &a.s.equipment)?
            .weight,
    );
    for item in buyer_bag.iter().filter(|item| item.id != 0) {
        let (row, _) = catalog
            .item(item.id)
            .ok_or_else(|| anyhow::anyhow!("Unknown inventory item."))?;
        weight += combat::integer(row, 18).max(0) as u64 * u64::from(item.count);
    }
    anyhow::ensure!(
        weight <= u64::from(a.s.combat.max_weight),
        "Inventory weight limit exceeded."
    );
    let cost = listing
        .price
        .checked_mul(u32::from(count))
        .ok_or_else(|| anyhow::anyhow!("Invalid price."))?;
    let wallet_before = [
        starter::coins(&tx, &a.account, a.s.character.slot)?,
        starter::coins(&tx, &b.account, b.s.character.slot)?,
    ];
    let wallet_after = [
        wallet_before[0]
            .checked_sub(cost)
            .ok_or_else(|| anyhow::anyhow!("Not enough Noah."))?,
        wallet_before[1]
            .checked_add(cost)
            .filter(|&n| n <= trade::COIN_MAX)
            .ok_or_else(|| anyhow::anyhow!("The seller's Noah limit would be exceeded."))?,
    ];
    listing.count -= count;
    listing.original.count -= count;
    seller_bag[listing.source] = if listing.original.count == 0 {
        Item::default()
    } else {
        listing.original
    };
    save_slot(&tx, a, dest, buyer_bag[dest])?;
    save_slot(&tx, b, listing.source, seller_bag[listing.source])?;
    for (player, balance) in [a, b].into_iter().zip(wallet_after) {
        anyhow::ensure!(
            tx.execute(
                "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
                params![player.account, player.s.character.slot, balance]
            )? == 1,
            "Missing character wallet."
        );
    }
    tx.commit()?;

    hub.stalls.get_mut(&seller).unwrap().listings[index] = (listing.count > 0).then_some(listing);
    let buyer_update = inventory::reward_packet(&buyer_before, &buyer_bag);
    let seller_update = inventory::count_packet(&[(listing.source, listing.original)]);
    for (side, (id, after, inventory_update)) in [
        (buyer, buyer_bag, buyer_update),
        (seller, seller_bag, Some(seller_update)),
    ]
    .into_iter()
    .enumerate()
    {
        hub.refresh.insert(id);
        hub.send(id, wallet::change(wallet_before[side], wallet_after[side]));
        if let Some(packet) = inventory_update {
            hub.send(id, packet);
        }
        let player = &hub.players[&id];
        let equipped_weight = catalog
            .calculate(&player.s.character, &player.s.progress, &player.s.equipment)
            .map_or(0, |stats| stats.weight);
        let mut packet = vec![0x54];
        packet.extend(
            equipped_weight
                .saturating_add(catalog.bag_weight(&after))
                .to_le_bytes(),
        );
        hub.send(id, packet);
    }
    let mut packet = answer(6, 1);
    packet.extend(item.to_le_bytes());
    packet.extend(listing.count.to_le_bytes());
    packet.extend([index as u8, dest as u8]);
    hub.send(buyer, packet);
    let mut packet = vec![0x68, 9];
    packet.extend(item.to_le_bytes());
    social::string(&mut packet, &name);
    hub.send(seller, packet);
    if listing.count == 0 && index < 4 {
        let mut packet = vec![0x69, 2];
        packet.extend(seller.to_le_bytes());
        packet.extend([0, index as u8]);
        hub.send(seller, packet.clone());
        hub.broadcast_visual(seller, None, None, &packet);
    }
    tracing::info!(
        buyer,
        seller,
        item,
        count,
        cost,
        "player merchant purchase committed"
    );
    if hub.stalls[&seller].listings.iter().all(Option::is_none) {
        close(hub, seller);
    }
    Ok(())
}

pub(crate) fn process(lobby: &Lobby, id: u16, payload: &[u8]) {
    let [0x68, op, fields @ ..] = payload else {
        return;
    };
    // Match personal trade's database -> Hub lock order.
    let mut db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    if !hub.players.contains_key(&id) {
        return;
    }
    maintain(&mut hub, id);
    let result = match (*op, fields, &lobby.combat) {
        (1, [], Some(_)) => {
            open(&mut hub, id);
            Ok(())
        }
        (2, [], _) => {
            close(&mut hub, id);
            Ok(())
        }
        (3, [..], Some(catalog)) if fields.len() == 12 => {
            add(&mut hub, id, fields, &db, catalog).map(|packet| hub.send(id, packet))
        }
        (4, [index], _) => {
            let removed = hub
                .stalls
                .get_mut(&id)
                .filter(|stall| !stall.published)
                .and_then(|stall| stall.listings.get_mut(usize::from(*index)))
                .and_then(Option::take);
            let mut packet = answer(4, if removed.is_some() { 1 } else { -3 });
            if removed.is_some() {
                packet.push(*index);
            }
            hub.send(id, packet);
            Ok(())
        }
        (5, [lo, hi], _) => list(&mut hub, id, u16::from_le_bytes([*lo, *hi])),
        (6, [..], Some(catalog)) if fields.len() == 8 => {
            buy(&mut hub, id, fields, &mut db, catalog)
        }
        (7, [lo, hi, advert @ ..], _)
            if usize::from(u16::from_le_bytes([*lo, *hi])) == advert.len() =>
        {
            publish(&mut hub, id, advert)
        }
        (8, [], _) => {
            stop_viewing(&mut hub, id);
            Ok(())
        }
        _ => Err(anyhow::anyhow!("Unsupported merchant request.")),
    };
    if let Err(error) = result {
        tracing::debug!(id, op, %error, "player merchant request rejected");
        hub.send(id, answer(*op, 0));
        hub.send(
            id,
            crate::chat::packet(
                7,
                0,
                u16::MAX,
                "",
                b"The merchant request could not be completed.",
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARROW: u32 = 391010000;

    fn setup() -> Option<(Lobby, [u16; 2])> {
        let (lobby, seller) = crate::world_service_tests::setup()?;
        let mut buyer = seller.clone();
        buyer.player_id += 1;
        buyer.character.name = "MerchantPeer".into();
        let ids = [seller.player_id, buyer.player_id];
        {
            let mut db = lobby.db.lock().unwrap();
            crate::characters::ensure_account(&db, "peer").unwrap();
            crate::characters::set_nation(&db, "peer", 1).unwrap();
            crate::characters::create(&mut db, "peer", &buyer.character).unwrap();
            db.execute(
                "INSERT INTO character_wallets VALUES ('peer',0,1000000)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count)
                 VALUES ('test',0,0,?1,0,10),('test',0,1,?1,0,4),('peer',0,2,?1,0,3)",
                [ARROW],
            ).unwrap();
            let mut hub = lobby.social.lock().unwrap();
            for (account, mut session) in [("test", seller), ("peer", buyer)] {
                session.inventory = starter::inventory(&db, account, 0).unwrap();
                hub.synchronize(account, &session);
            }
        }
        Some((lobby, ids))
    }

    fn listing(lobby: &Lobby, seller: u16, count: u16, source: u8, index: u8) {
        let mut packet = vec![0x68, 3];
        packet.extend(ARROW.to_le_bytes());
        packet.extend(count.to_le_bytes());
        packet.extend(100u32.to_le_bytes());
        packet.extend([source, index]);
        process(lobby, seller, &packet);
    }

    fn saved(lobby: &Lobby) -> (u32, u32, u16, u16) {
        let db = lobby.db.lock().unwrap();
        (
            starter::coins(&db, "test", 0).unwrap(),
            starter::coins(&db, "peer", 0).unwrap(),
            starter::inventory(&db, "test", 0).unwrap()[0].count,
            starter::inventory(&db, "peer", 0).unwrap()[2].count,
        )
    }

    #[test]
    fn sale_requires_current_view_and_commits_partial_and_final_quantities() {
        let Some((lobby, [seller, buyer])) = setup() else {
            return;
        };
        process(&lobby, seller, &[0x68, 1]);
        listing(&lobby, seller, 0, 0, 0);
        assert!(lobby.social.lock().unwrap().stalls[&seller].listings[0].is_none());
        listing(&lobby, seller, 5, 0, 0);
        listing(&lobby, seller, 4, 0, 1);
        assert!(lobby.social.lock().unwrap().stalls[&seller].listings[1].is_none());
        process(&lobby, seller, &[0x68, 7, 4, 0, b'S', b'h', b'o', b'p']);
        let mut buy_request = vec![0x68, 6];
        buy_request.extend(ARROW.to_le_bytes());
        buy_request.extend([2, 0, 0, 2]);
        process(&lobby, buyer, &buy_request);
        assert_eq!(saved(&lobby), (1000000, 1000000, 10, 3));
        let [lo, hi] = seller.to_le_bytes();
        process(&lobby, buyer, &[0x68, 5, lo, hi]);
        lobby
            .db
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_credit BEFORE UPDATE ON character_wallets
             WHEN NEW.account='test' BEGIN SELECT RAISE(ABORT,'test rollback'); END;",
            )
            .unwrap();
        process(&lobby, buyer, &buy_request);
        assert_eq!(saved(&lobby), (1000000, 1000000, 10, 3));
        lobby
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_credit")
            .unwrap();
        process(&lobby, buyer, &buy_request);
        assert_eq!(saved(&lobby), (1000200, 999800, 8, 5));
        buy_request[6] = 3;
        process(&lobby, buyer, &buy_request);
        assert_eq!(saved(&lobby), (1000500, 999500, 5, 8));
        assert!(!busy(&lobby.social.lock().unwrap(), seller));
        process(&lobby, buyer, &buy_request);
        assert_eq!(saved(&lobby), (1000500, 999500, 5, 8));
    }

    #[test]
    fn cancel_and_disconnect_preserve_unsold_items() {
        let Some((lobby, [seller, buyer])) = setup() else {
            return;
        };
        process(&lobby, seller, &[0x68, 1]);
        listing(&lobby, seller, 3, 0, 0);
        process(&lobby, seller, &[0x68, 4, 0]);
        assert!(lobby.social.lock().unwrap().stalls[&seller].listings[0].is_none());
        listing(&lobby, seller, 3, 0, 0);
        process(&lobby, seller, &[0x68, 7, 4, 0, b'S', b'h', b'o', b'p']);
        let [lo, hi] = seller.to_le_bytes();
        process(&lobby, buyer, &[0x68, 5, lo, hi]);
        let mut hub = lobby.social.lock().unwrap();
        assert!(busy(&hub, buyer));
        hub.disconnect(seller);
        assert!(!busy(&hub, seller) && !busy(&hub, buyer));
        drop(hub);
        assert_eq!(saved(&lobby), (1000000, 1000000, 10, 3));
    }

    #[test]
    fn blank_advert_publishes_both_rows_and_selling_one_leaves_the_other_open() {
        let Some((lobby, [seller, buyer])) = setup() else {
            return;
        };
        process(&lobby, seller, &[0x68, 1]);
        listing(&lobby, seller, 5, 0, 0);
        listing(&lobby, seller, 4, 1, 9);
        process(&lobby, seller, &[0x68, 7, 0, 0]);
        let [lo, hi] = seller.to_le_bytes();
        process(&lobby, buyer, &[0x68, 5, lo, hi]);
        let mut buy_request = vec![0x68, 6];
        buy_request.extend(ARROW.to_le_bytes());
        buy_request.extend([4, 0, 9, 2]);
        process(&lobby, buyer, &buy_request);
        let hub = lobby.social.lock().unwrap();
        let stall = &hub.stalls[&seller];
        assert!(stall.published && stall.listings[0].is_some() && stall.listings[9].is_none());
        drop(hub);
        assert_eq!(saved(&lobby), (1000400, 999600, 10, 7));
    }
}
