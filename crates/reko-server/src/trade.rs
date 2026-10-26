//! Native personal exchange; offers stay in their owners' saves until atomic commit.
use crate::{
    combat,
    lobby::Lobby,
    social::{Hub, Player},
    starter::{self, Item},
};
use rusqlite::{Connection, params};

const GOLD: u32 = 900000000;
const COIN_MAX: u32 = 2100000000;
#[derive(Clone)]
struct Line {
    slot: usize,
    original: Item,
    count: u16,
}
#[derive(Clone, Default)]
struct Offer {
    lines: Vec<Line>,
    coins: u32,
    ready: bool,
}
#[derive(Clone)]
pub(crate) struct Exchange {
    ids: [u16; 2],
    active: bool,
    until: u64,
    offers: [Offer; 2],
}
pub(crate) fn busy(h: &Hub, id: u16) -> bool {
    key(h, id).is_some()
}
fn key(h: &Hub, id: u16) -> Option<u16> {
    h.exchanges
        .iter()
        .find(|(_, t)| t.ids.contains(&id))
        .map(|(&k, _)| k)
}
fn notice(h: &mut Hub, id: u16, text: &str) {
    h.send(id, crate::chat::packet(7, 0, u16::MAX, "", text.as_bytes()));
}
pub(crate) fn cancel(h: &mut Hub, id: u16, reason: &str) {
    if let Some(t) = key(h, id).and_then(|k| h.exchanges.remove(&k)) {
        tracing::info!(
            first = t.ids[0],
            second = t.ids[1],
            reason,
            "personal trade cancelled"
        );
        for id in t.ids {
            h.send(id, vec![0x30, 8]);
            if !reason.is_empty() {
                notice(h, id, reason);
            }
        }
    }
}
fn available(h: &Hub, ids: [u16; 2]) -> bool {
    let [a, b] = ids.map(|id| h.players.get(&id));
    a.zip(b).is_some_and(|(a, b)| {
        [&a.s, &b.s]
            .iter()
            .all(|s| s.entered && !s.zone_loading && s.skills.hp > 0)
            && crate::social::near(a.s.position, b.s.position, 10)
            && (a.s.character.nation() == b.s.character.nation()
                || matches!(a.s.position.zone, 21 | 55))
            && h.can_see(ids[0], ids[1])
            && h.can_see(ids[1], ids[0])
    })
}
pub(crate) fn maintain(h: &mut Hub, id: u16) {
    if let Some(k) = key(h, id) {
        let t = &h.exchanges[&k];
        if crate::skills::now_ms() >= t.until || !available(h, t.ids) {
            cancel(
                h,
                id,
                "Trade cancelled: timeout, distance, death or zone change.",
            );
        }
    }
}
// Mutating another native window while exchange owns the inventory would leave
// that window's optimistic icons inconsistent. End exchange before dispatching it.
pub(crate) fn before_action(h: &mut Hub, id: u16, opcode: u8) {
    // 0x41 is the client's periodic timestamp check, not an inventory action.
    if matches!(
        opcode,
        0x08 | 0x45
            | 0x33
            | 0x5b
            | 0x27
            | 0x4b
            | 0x24
            | 0x26
            | 0x31
            | 0x20
            | 0x21
            | 0x3b
            | 0x28
            | 0x32
            | 0x48
            | 0x55
            | 0x64
            | 0x1f
            | 0x3f
            | 0x70
            | 0x2f
            | 0x3c
            | 0x0c
            | 0x12
    ) {
        cancel(
            h,
            id,
            &format!(
                "Trade cancelled before another inventory or gameplay action (0x{opcode:02X})."
            ),
        );
    }
}
fn tradable(c: &combat::Catalog, id: u32) -> anyhow::Result<bool> {
    let (r, e) = c
        .item(id)
        .ok_or_else(|| anyhow::anyhow!("Unknown trade item"))?;
    anyhow::ensure!(
        id < GOLD && e.is_some() && combat::integer(r, 13) != 20,
        "Item cannot be traded"
    );
    Ok(matches!(combat::integer(r, 23), 1 | 3))
}
fn stack_limit(c: &combat::Catalog, id: u32) -> u16 {
    if c.item(id).is_some_and(|(r, _)| combat::integer(r, 23) == 3) {
        500
    } else {
        9999
    }
}
fn offered(
    t: &Exchange,
    side: usize,
    bag: &mut [Item; 28],
    coins: u32,
    c: &combat::Catalog,
) -> anyhow::Result<u32> {
    let o = &t.offers[side];
    anyhow::ensure!(o.coins <= coins, "Not enough Noah");
    for l in &o.lines {
        tradable(c, l.original.id)?;
        let item = &mut bag[l.slot];
        anyhow::ensure!(
            *item == l.original && l.count > 0 && l.count <= item.count,
            "Offered item changed or is no longer owned"
        );
        item.count -= l.count;
        if item.count == 0 {
            *item = Item::default();
        }
    }
    Ok(coins - o.coins)
}
fn receive(
    bag: &mut [Item; 28],
    incoming: &Offer,
    c: &combat::Catalog,
) -> anyhow::Result<Vec<(u8, Item)>> {
    let mut moves = Vec::new();
    for line in &incoming.lines {
        let stack = tradable(c, line.original.id)?;
        let limit = stack_limit(c, line.original.id);
        let mut remaining = line.count;
        for (slot, item) in bag.iter_mut().enumerate() {
            if remaining == 0 {
                break;
            }
            if stack && item.id == line.original.id && item.durability == line.original.durability {
                let n = remaining.min(limit.saturating_sub(item.count));
                if n == 0 {
                    continue;
                }
                item.count += n;
                remaining -= n;
                moves.push((
                    slot as u8,
                    Item {
                        count: n,
                        ..line.original
                    },
                ));
            }
        }
        while remaining > 0 {
            let slot = bag
                .iter()
                .position(|i| i.id == 0)
                .ok_or_else(|| anyhow::anyhow!("Not enough inventory slots"))?;
            let n = remaining.min(limit);
            bag[slot] = Item {
                count: n,
                ..line.original
            };
            moves.push((slot as u8, bag[slot]));
            remaining -= n;
        }
    }
    Ok(moves)
}
fn done(coins: u32, moves: &[(u8, Item)]) -> Vec<u8> {
    let mut p = vec![0x30, 7, 1];
    p.extend(coins.to_le_bytes());
    p.extend((moves.len() as u16).to_le_bytes());
    for (slot, item) in moves {
        p.push(*slot);
        p.extend(item.id.to_le_bytes());
        p.extend(item.count.to_le_bytes());
        p.extend(item.durability.to_le_bytes());
        p.extend(0u32.to_le_bytes());
    }
    p
}
// One SQLite transaction owns both inventory and wallet snapshots and all writes.
fn commit(
    db: &mut Connection,
    t: &Exchange,
    players: &[Player; 2],
    c: &combat::Catalog,
) -> anyhow::Result<[Vec<Vec<u8>>; 2]> {
    let tx = db.transaction()?;
    let mut bags = [[Item::default(); 28]; 2];
    let mut coins = [0; 2];
    for side in 0..2 {
        let p = &players[side];
        bags[side] = starter::inventory(&tx, &p.account, p.s.character.slot)?;
        coins[side] = offered(
            t,
            side,
            &mut bags[side],
            starter::coins(&tx, &p.account, p.s.character.slot)?,
            c,
        )?;
    }
    let mut packets: [Vec<Vec<u8>>; 2] = Default::default();
    for side in 0..2 {
        let p = &players[side];
        let incoming = &t.offers[1 - side];
        coins[side] = coins[side]
            .checked_add(incoming.coins)
            .filter(|n| *n <= COIN_MAX)
            .ok_or_else(|| anyhow::anyhow!("Noah limit exceeded"))?;
        let moves = receive(&mut bags[side], incoming, c)?;
        // Sum without Catalog's display saturation: over-32767 bags must fail too.
        let mut weight = u64::from(
            c.calculate(&p.s.character, &p.s.progress, &p.s.equipment)?
                .weight,
        );
        for item in bags[side].iter().filter(|i| i.id != 0) {
            let (row, _) = c
                .item(item.id)
                .ok_or_else(|| anyhow::anyhow!("Unknown inventory item"))?;
            weight += combat::integer(row, 18).max(0) as u64 * u64::from(item.count);
        }
        anyhow::ensure!(
            weight <= u64::from(p.s.combat.max_weight),
            "Inventory weight limit exceeded"
        );
        packets[side].push(done(coins[side], &moves));
        let mut w = vec![0x54];
        w.extend((weight as u16).to_le_bytes());
        packets[side].push(w);
    }
    for side in 0..2 {
        let p = &players[side];
        let slot = p.s.character.slot;
        tx.execute(
            "DELETE FROM character_inventory WHERE account=?1 AND slot=?2",
            params![p.account, slot],
        )?;
        for (pos, item) in bags[side].iter().enumerate().filter(|(_, i)| i.id != 0) {
            tx.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?1,?2,?3,?4,?5,?6)",params![p.account,slot,pos,item.id,item.durability,item.count])?;
        }
        anyhow::ensure!(
            tx.execute(
                "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
                params![p.account, slot, coins[side]]
            )? == 1,
            "Missing character wallet"
        );
    }
    tx.commit()?;
    Ok(packets)
}
fn add(
    db: &Connection,
    h: &mut Hub,
    k: u16,
    side: usize,
    payload: &[u8],
    c: &combat::Catalog,
) -> anyhow::Result<()> {
    anyhow::ensure!(payload.len() == 11, "Invalid trade offer packet");
    let t = &h.exchanges[&k];
    anyhow::ensure!(
        t.active && !t.offers.iter().any(|o| o.ready),
        "Trade already confirmed"
    );
    let id = u32::from_le_bytes(payload[3..7].try_into()?);
    let mut count = u32::from_le_bytes(payload[7..11].try_into()?);
    anyhow::ensure!(count > 0, "Empty offer");
    let p = &h.players[&t.ids[side]];
    let mut offer = t.offers[side].clone();
    let duration;
    if id == GOLD {
        offer.coins = offer
            .coins
            .checked_add(count)
            .filter(|n| *n <= COIN_MAX)
            .ok_or_else(|| anyhow::anyhow!("Noah limit"))?;
        anyhow::ensure!(
            offer.coins <= starter::coins(db, &p.account, p.s.character.slot)?,
            "Not enough Noah"
        );
        duration = 0;
    } else {
        let stack = tradable(c, id)?;
        let slot = usize::from(payload[2]);
        let bag = starter::inventory(db, &p.account, p.s.character.slot)?;
        let item = *bag
            .get(slot)
            .ok_or_else(|| anyhow::anyhow!("Invalid inventory slot"))?;
        anyhow::ensure!(
            item.id == id && count <= u32::from(item.count),
            "Item ownership or quantity mismatch"
        );
        // Native UI moves the whole nonstackable icon and sends unit count1,
        // including packs whose saved count represents remaining charges.
        if !stack {
            anyhow::ensure!(
                count == 1 || count == u32::from(item.count),
                "Nonstackable item must move whole"
            );
            count = u32::from(item.count);
        }
        duration = item.durability;
        if let Some(line) = offer.lines.iter_mut().find(|l| l.slot == slot) {
            anyhow::ensure!(line.original == item, "Offered item changed");
            line.count = line
                .count
                .checked_add(count as u16)
                .filter(|n| *n <= item.count)
                .ok_or_else(|| anyhow::anyhow!("Quantity already offered"))?;
        } else {
            anyhow::ensure!(count <= 9999, "Stack limit");
            offer.lines.push(Line {
                slot,
                original: item,
                count: count as u16,
            });
        }
        let mut slots: Vec<(u32, u16)> = Vec::new();
        for l in &offer.lines {
            if tradable(c, l.original.id)?
                && let Some((_, total)) = slots.iter_mut().find(|(id, _)| *id == l.original.id)
            {
                *total = total
                    .checked_add(l.count)
                    .filter(|n| *n <= stack_limit(c, l.original.id))
                    .ok_or_else(|| anyhow::anyhow!("Trade stack limit"))?;
                continue;
            }
            slots.push((l.original.id, l.count));
        }
        anyhow::ensure!(slots.len() <= 12, "Trade has twelve item slots");
    }
    let ids = t.ids;
    h.exchanges.get_mut(&k).unwrap().offers[side] = offer;
    h.send(ids[side], vec![0x30, 3, 1]);
    let mut packet = vec![0x30, 4];
    packet.extend(id.to_le_bytes());
    packet.extend(count.to_le_bytes());
    packet.extend(duration.to_le_bytes());
    packet.extend(0u32.to_le_bytes());
    h.send(ids[1 - side], packet);
    Ok(())
}
pub(crate) fn process(lobby: &Lobby, id: u16, payload: &[u8]) {
    // Every path that needs both follows existing DB -> social lock order.
    let mut db = lobby.db.lock().unwrap();
    let mut h = lobby.social.lock().unwrap();
    maintain(&mut h, id);
    let Some(&op) = payload.get(1) else {
        cancel(&mut h, id, "Invalid trade request");
        return;
    };
    if op == 8 {
        cancel(&mut h, id, "");
        return;
    }
    if op == 1 && (payload.len() == 4 || payload.len() == 5 && payload[4] == 1) {
        let other = u16::from_le_bytes([payload[2], payload[3]]);
        if id == other || busy(&h, id) || busy(&h, other) || !available(&h, [id, other]) {
            if key(&h, id).is_none() {
                h.send(id, vec![0x30, 8]);
            }
            notice(
                &mut h,
                id,
                "Trade unavailable: player busy, too far away or not eligible.",
            );
            return;
        }
        // Both participation checks and insertion share this Hub lock, including
        // pending requests. Competing requests cannot reserve either player twice.
        h.exchanges.insert(
            id,
            Exchange {
                ids: [id, other],
                active: false,
                until: crate::skills::now_ms() + 60000,
                offers: Default::default(),
            },
        );
        let mut p = vec![0x30, 1];
        p.extend(id.to_le_bytes());
        h.send(other, p);
        return;
    }
    let Some(k) = key(&h, id) else {
        h.send(id, vec![0x30, 8]);
        return;
    };
    let side = usize::from(h.exchanges[&k].ids[1] == id);
    if op == 2 && payload.len() == 3 && side == 1 && !h.exchanges[&k].active {
        if payload[2] != 1 {
            cancel(&mut h, id, "");
            return;
        }
        let t = h.exchanges.get_mut(&k).unwrap();
        t.active = true;
        t.until = crate::skills::now_ms() + 600000;
        let other = t.ids[0];
        h.send(other, vec![0x30, 2, 1]);
        return;
    }
    let Some(c) = lobby.combat.as_ref() else {
        cancel(&mut h, id, "Item catalog unavailable");
        return;
    };
    if op == 3 {
        if let Err(error) = add(&db, &mut h, k, side, payload, c) {
            h.send(id, vec![0x30, 3, 0]);
            notice(&mut h, id, &error.to_string());
        }
    } else if op == 5 && payload.len() == 2 && h.exchanges[&k].active {
        let t = h.exchanges.get_mut(&k).unwrap();
        if t.offers[side].ready {
            return;
        }
        t.offers[side].ready = true;
        let t = t.clone();
        if !t.offers[1 - side].ready {
            h.send(t.ids[1 - side], vec![0x30, 6]);
            return;
        }
        h.exchanges.remove(&k);
        let players = t.ids.map(|id| h.players[&id].clone());
        match commit(&mut db, &t, &players, c) {
            Ok(packets) => {
                for (side, packets) in packets.into_iter().enumerate() {
                    let id = t.ids[side];
                    h.refresh.insert(id);
                    for p in packets {
                        h.send(id, p);
                    }
                }
                tracing::info!(first=%players[0].s.character.name,second=%players[1].s.character.name,"personal trade committed");
            }
            Err(error) => {
                tracing::warn!(%error,"personal trade rolled back");
                for id in t.ids {
                    h.send(id, vec![0x30, 7, 0]);
                    notice(&mut h, id, &format!("Trade failed: {error}"));
                }
            }
        }
    } else {
        cancel(&mut h, id, "Invalid or out-of-order trade request");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> Option<(Lobby, [Player; 2])> {
        let (lobby, a) = crate::world_service_tests::setup()?;
        let mut b = a.clone();
        b.player_id += 1;
        b.character.name = "TradePeer".into();
        {
            let mut db = lobby.db.lock().unwrap();
            crate::characters::ensure_account(&db, "peer").unwrap();
            crate::characters::set_nation(&db, "peer", 1).unwrap();
            crate::characters::create(&mut db, "peer", &b.character).unwrap();
            db.execute(
                "INSERT INTO character_wallets VALUES ('peer',0,1000000)",
                [],
            )
            .unwrap();
        }
        let players = [
            Player {
                account: "test".into(),
                s: a,
            },
            Player {
                account: "peer".into(),
                s: b,
            },
        ];
        {
            let mut h = lobby.social.lock().unwrap();
            for p in &players {
                h.synchronize(&p.account, &p.s);
            }
        }
        Some((lobby, players))
    }
    fn put(db: &Connection, account: &str, slot: usize, id: u32, count: u16, durability: u16) {
        db.execute(
            "INSERT OR REPLACE INTO character_inventory VALUES (?1,0,?2,?3,?4,?5)",
            params![account, slot, id, durability, count],
        )
        .unwrap();
    }
    fn request(lobby: &Lobby, p: &[Player; 2]) {
        let mut req = vec![0x30, 1];
        req.extend(p[1].s.player_id.to_le_bytes());
        req.push(1);
        process(lobby, p[0].s.player_id, &req);
        process(lobby, p[1].s.player_id, &[0x30, 2, 1]);
    }
    fn offer(lobby: &Lobby, id: u16, slot: u8, item: u32, count: u32) {
        let mut packet = vec![0x30, 3, slot];
        packet.extend(item.to_le_bytes());
        packet.extend(count.to_le_bytes());
        process(lobby, id, &packet);
    }
    fn decide(lobby: &Lobby, p: &[Player; 2]) {
        for p in p {
            process(lobby, p.s.player_id, &[0x30, 5]);
        }
    }
    fn saved(lobby: &Lobby) -> Vec<(String, usize, u32, u16, u16)> {
        lobby.db.lock().unwrap().prepare("SELECT account,inventory_slot,item_id,durability,count FROM character_inventory ORDER BY account,inventory_slot").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap().collect::<Result<_,_>>().unwrap()
    }
    #[test]
    fn competing_requests_reserve_each_player_once_pending_and_active() {
        let Some((lobby, players)) = setup() else {
            return;
        };
        let a = players[0].s.player_id;
        let b = players[1].s.player_id;
        let mut third = players[0].clone();
        third.s.player_id = b + 1;
        third.account = "third".into();
        third.s.character.name = "TradeThird".into();
        let c = third.s.player_id;
        lobby
            .social
            .lock()
            .unwrap()
            .synchronize(&third.account, &third.s);
        let before = saved(&lobby);
        let req = |from, to: u16| {
            let mut p = vec![0x30, 1];
            p.extend(to.to_le_bytes());
            p.push(1);
            process(&lobby, from, &p);
        };
        for _ in 0..20 {
            let barrier = std::sync::Barrier::new(3);
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    barrier.wait();
                    req(a, b);
                });
                scope.spawn(|| {
                    barrier.wait();
                    req(c, b);
                });
                barrier.wait();
            });
            let (winner, loser) = {
                let h = lobby.social.lock().unwrap();
                assert_eq!(h.exchanges.len(), 1);
                let t = h.exchanges.values().next().unwrap();
                assert_eq!(t.ids[1], b);
                (t.ids[0], if t.ids[0] == a { c } else { a })
            };
            for active in [false, true] {
                if active {
                    process(&lobby, b, &[0x30, 2, 1]);
                }
                for (from, to) in [
                    (winner, loser),
                    (b, loser),
                    (loser, winner),
                    (loser, b),
                    (b, winner),
                ] {
                    req(from, to);
                }
                process(&lobby, loser, &[0x30, 8]); // A stranger cannot cancel the reserved pair.
                let h = lobby.social.lock().unwrap();
                assert_eq!(h.exchanges.len(), 1);
                let t = &h.exchanges[&winner];
                assert_eq!(t.ids, [winner, b]);
                assert_eq!(t.active, active);
                let mut seen = std::collections::HashSet::new();
                for t in h.exchanges.values() {
                    for id in t.ids {
                        assert!(seen.insert(id));
                    }
                }
            }
            process(&lobby, winner, &[0x30, 8]);
            assert!(lobby.social.lock().unwrap().exchanges.is_empty());
            assert_eq!(saved(&lobby), before);
        }
    }
    #[tokio::test]
    async fn native_two_client_exchange_routes_consent_offers_and_completion_with_1453_item_tails()
    {
        use tokio::{
            io::AsyncReadExt,
            net::{TcpListener, TcpStream},
        };
        async fn pair() -> (TcpStream, TcpStream, std::net::SocketAddr) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (server, peer) = listener.accept().await.unwrap();
            (client, server, peer)
        }
        async fn receive(client: &mut TcpStream, prefix: &[u8]) -> Vec<u8> {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let mut head = [0; 4];
                    client.read_exact(&mut head).await.unwrap();
                    assert_eq!(&head[..2], &[0xaa, 0x55]);
                    let len = u16::from_le_bytes(head[2..].try_into().unwrap()) as usize;
                    let mut body = vec![0; len + 2];
                    client.read_exact(&mut body).await.unwrap();
                    assert_eq!(&body[len..], &[0x55, 0xaa]);
                    if body.starts_with(prefix) {
                        return body[..len].to_vec();
                    }
                }
            })
            .await
            .unwrap()
        }
        let Some((lobby, players)) = setup() else {
            return;
        };
        let ids = players.each_ref().map(|p| p.s.player_id);
        {
            let db = lobby.db.lock().unwrap();
            put(&db, "test", 0, 110110001, 1, 4000);
        }
        let (mut ca, mut sa, pa) = pair().await;
        let (mut cb, mut sb, pb) = pair().await;
        let mut aa = Some("test".into());
        let mut ab = Some("peer".into());
        let mut wa = Some(players[0].s.clone());
        let mut wb = Some(players[1].s.clone());
        let mut req = vec![0x30, 1];
        req.extend(ids[1].to_le_bytes());
        req.push(1);
        crate::handle_game_payload(&mut sa, pa, &req, &lobby, &mut aa, &mut wa).await;
        crate::social::flush(&mut sb, pb, &lobby, wb.as_ref().unwrap()).await;
        crate::handle_game_payload(&mut sa, pa, &[0x41, 0, 0, 0, 0], &lobby, &mut aa, &mut wa)
            .await;
        assert!(busy(&lobby.social.lock().unwrap(), ids[0]));
        let req = receive(&mut cb, &[0x30, 1]).await;
        assert_eq!(req.len(), 4);
        assert_eq!(&req[2..], &ids[0].to_le_bytes());
        crate::handle_game_payload(&mut sb, pb, &[0x30, 2, 1], &lobby, &mut ab, &mut wb).await;
        crate::social::flush(&mut sa, pa, &lobby, wa.as_ref().unwrap()).await;
        assert_eq!(receive(&mut ca, &[0x30, 2]).await, vec![0x30, 2, 1]);
        let mut add = vec![0x30, 3, 0];
        add.extend(110110001u32.to_le_bytes());
        add.extend(1u32.to_le_bytes());
        crate::handle_game_payload(&mut sa, pa, &add, &lobby, &mut aa, &mut wa).await;
        assert_eq!(receive(&mut ca, &[0x30, 3]).await, vec![0x30, 3, 1]);
        crate::social::flush(&mut sb, pb, &lobby, wb.as_ref().unwrap()).await;
        crate::handle_game_payload(&mut sb, pb, &[0x41, 1, 0, 0, 0], &lobby, &mut ab, &mut wb)
            .await;
        assert!(busy(&lobby.social.lock().unwrap(), ids[1]));
        let other = receive(&mut cb, &[0x30, 4]).await;
        assert_eq!(other.len(), 16);
        assert_eq!(&other[12..], &[0; 4]);
        let mut money = vec![0x30, 3, 255];
        money.extend(GOLD.to_le_bytes());
        money.extend(10000u32.to_le_bytes());
        crate::handle_game_payload(&mut sb, pb, &money, &lobby, &mut ab, &mut wb).await;
        assert_eq!(receive(&mut cb, &[0x30, 3]).await, vec![0x30, 3, 1]);
        crate::social::flush(&mut sa, pa, &lobby, wa.as_ref().unwrap()).await;
        let offered_money = receive(&mut ca, &[0x30, 4]).await;
        assert_eq!(&offered_money[2..6], &GOLD.to_le_bytes());
        assert_eq!(&offered_money[6..10], &10000u32.to_le_bytes());
        crate::handle_game_payload(&mut sb, pb, &[0x41, 3, 0, 0, 0], &lobby, &mut ab, &mut wb)
            .await;
        assert!(busy(&lobby.social.lock().unwrap(), ids[1]));
        crate::handle_game_payload(&mut sa, pa, &[0x30, 5], &lobby, &mut aa, &mut wa).await;
        crate::social::flush(&mut sb, pb, &lobby, wb.as_ref().unwrap()).await;
        assert_eq!(receive(&mut cb, &[0x30, 6]).await, vec![0x30, 6]);
        crate::handle_game_payload(&mut sa, pa, &[0x41, 2, 0, 0, 0], &lobby, &mut aa, &mut wa)
            .await;
        assert!(busy(&lobby.social.lock().unwrap(), ids[0]));
        crate::handle_game_payload(&mut sb, pb, &[0x30, 5], &lobby, &mut ab, &mut wb).await;
        let done = receive(&mut cb, &[0x30, 7, 1]).await;
        assert_eq!(done.len(), 22);
        assert_eq!(&done[7..9], &1u16.to_le_bytes());
        assert_eq!(&done[18..], &[0; 4]);
        assert_eq!(wb.as_ref().unwrap().inventory[0].id, 110110001);
        crate::social::synchronize(&lobby, "test", wa.as_mut().unwrap());
        crate::social::flush(&mut sa, pa, &lobby, wa.as_ref().unwrap()).await;
        assert_eq!(receive(&mut ca, &[0x30, 7, 1]).await.len(), 9);
        assert_eq!(wa.as_ref().unwrap().inventory[0].id, 0);
        assert_eq!(wa.as_ref().unwrap().coins, 1010000);
        assert_eq!(wb.as_ref().unwrap().coins, 990000);
    }
    #[test]
    fn native_consent_and_atomic_swap_use_slots_freed_by_both_offers() {
        let Some((lobby, players)) = setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            for slot in 0..28 {
                put(&db, "test", slot, 110110001, 1, 4000);
                put(&db, "peer", slot, 120150001, 1, 5000);
            }
        }
        request(&lobby, &players);
        let ids = players.each_ref().map(|p| p.s.player_id);
        offer(&lobby, ids[0], 0, 110110001, 1);
        offer(&lobby, ids[1], 0, 120150001, 1);
        offer(&lobby, ids[0], 255, GOLD, 12345);
        offer(&lobby, ids[1], 255, GOLD, 2345);
        let before = saved(&lobby);
        process(&lobby, ids[0], &[0x30, 5]);
        assert_eq!(saved(&lobby), before, "one confirmation cannot commit");
        offer(&lobby, ids[1], 1, 120150001, 1); // Cannot revise after either confirmation.
        assert_eq!(
            lobby.social.lock().unwrap().exchanges[&ids[0]].offers[1]
                .lines
                .len(),
            1
        );
        process(&lobby, ids[1], &[0x30, 5]);
        let db = lobby.db.lock().unwrap();
        let a = starter::inventory(&db, "test", 0).unwrap();
        let b = starter::inventory(&db, "peer", 0).unwrap();
        assert_eq!((a[0].id, b[0].id), (120150001, 110110001));
        assert_eq!((a[0].durability, b[0].durability), (5000, 4000));
        assert_eq!(
            (
                starter::coins(&db, "test", 0).unwrap(),
                starter::coins(&db, "peer", 0).unwrap()
            ),
            (990000, 1010000)
        );
        drop(db);
        assert!(lobby.social.lock().unwrap().exchanges.is_empty());
        let after = saved(&lobby);
        decide(&lobby, &players);
        assert_eq!(saved(&lobby), after, "replay cannot duplicate items");
        for id in ids {
            assert!(lobby.social.lock().unwrap().refresh.contains(&id));
        }
    }
    #[test]
    fn final_capacity_weight_wallet_and_ownership_failures_leave_both_saves_unchanged() {
        for failure in ["slots", "weight", "wallet", "ownership", "sql"] {
            let Some((lobby, players)) = setup() else {
                return;
            };
            let ids = players.each_ref().map(|p| p.s.player_id);
            {
                let db = lobby.db.lock().unwrap();
                put(&db, "test", 0, 110110001, 1, 4000);
                put(&db, "peer", 0, 120150001, 1, 5000);
                if failure == "slots" {
                    for slot in 1..28 {
                        put(&db, "peer", slot, 120150001, 1, 5000);
                    }
                }
            }
            request(&lobby, &players);
            offer(&lobby, ids[0], 0, 110110001, 1);
            offer(&lobby, ids[0], 255, GOLD, 100);
            // Only the giver offers an item; the receiver must find a free slot.
            match failure {
                "weight" => {
                    lobby
                        .social
                        .lock()
                        .unwrap()
                        .players
                        .get_mut(&ids[1])
                        .unwrap()
                        .s
                        .combat
                        .max_weight = 0;
                }
                "wallet" => {
                    lobby
                        .db
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE character_wallets SET coins=2100000000 WHERE account='peer'",
                            [],
                        )
                        .unwrap();
                }
                "ownership" => {
                    lobby
                        .db
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE character_inventory SET durability=1 WHERE account='test'",
                            [],
                        )
                        .unwrap();
                }
                "sql" => {
                    lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_peer BEFORE UPDATE ON character_wallets WHEN NEW.account='peer' BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
                }
                _ => {}
            }
            let before = saved(&lobby);
            let coins = {
                let db = lobby.db.lock().unwrap();
                [
                    starter::coins(&db, "test", 0).unwrap(),
                    starter::coins(&db, "peer", 0).unwrap(),
                ]
            };
            decide(&lobby, &players);
            assert_eq!(saved(&lobby), before, "{failure}");
            let db = lobby.db.lock().unwrap();
            assert_eq!(
                [
                    starter::coins(&db, "test", 0).unwrap(),
                    starter::coins(&db, "peer", 0).unwrap()
                ],
                coins,
                "{failure}"
            );
            drop(db);
            assert!(lobby.social.lock().unwrap().exchanges.is_empty());
        }
    }
    #[test]
    fn merged_stacks_split_without_loss_and_preserve_durability_and_charged_packs() {
        let Some((lobby, players)) = setup() else {
            return;
        };
        let ids = players.each_ref().map(|p| p.s.player_id);
        {
            let db = lobby.db.lock().unwrap();
            put(&db, "test", 0, 389020000, 10, 0);
            put(&db, "peer", 0, 389020000, 9995, 0);
        }
        for id in ids {
            lobby
                .social
                .lock()
                .unwrap()
                .players
                .get_mut(&id)
                .unwrap()
                .s
                .combat
                .max_weight = 32767;
        }
        // Soul is heavy: use a high max weight and a lighter original arrow stack.
        let arrow = 391010000;
        if lobby.combat.as_ref().unwrap().item(arrow).is_none() {
            panic!("missing native arrow");
        }
        {
            let db = lobby.db.lock().unwrap();
            db.execute("UPDATE character_inventory SET item_id=?1", params![arrow])
                .unwrap();
        }
        request(&lobby, &players);
        offer(&lobby, ids[0], 0, arrow, 10);
        decide(&lobby, &players);
        {
            let db = lobby.db.lock().unwrap();
            let a = starter::inventory(&db, "test", 0).unwrap();
            let b = starter::inventory(&db, "peer", 0).unwrap();
            assert_eq!(a[0].id, 0);
            assert_eq!((b[0].count, b[1].count), (9999, 6));
        }
        // Existing nonstackable skill scroll packs keep count and charges as one item.
        {
            let db = lobby.db.lock().unwrap();
            put(&db, "test", 0, 370001000, 20, 20);
        }
        request(&lobby, &players);
        offer(&lobby, ids[0], 0, 370001000, 2);
        assert!(
            lobby.social.lock().unwrap().exchanges[&ids[0]].offers[0]
                .lines
                .is_empty()
        );
        offer(&lobby, ids[0], 0, 370001000, 1);
        decide(&lobby, &players);
        let db = lobby.db.lock().unwrap();
        assert!(
            starter::inventory(&db, "peer", 0)
                .unwrap()
                .iter()
                .any(|i| i.id == 370001000 && i.count == 20 && i.durability == 20)
        );
    }
    #[test]
    fn consent_cancellation_disconnect_and_range_never_remove_offered_items() {
        let Some((lobby, players)) = setup() else {
            return;
        };
        let ids = players.each_ref().map(|p| p.s.player_id);
        {
            let db = lobby.db.lock().unwrap();
            put(&db, "test", 0, 110110001, 1, 4000);
        }
        let before = saved(&lobby);
        let mut req = vec![0x30, 1];
        req.extend(ids[1].to_le_bytes());
        process(&lobby, ids[0], &req);
        offer(&lobby, ids[0], 0, 110110001, 1); // no offers before consent
        assert!(
            lobby.social.lock().unwrap().exchanges[&ids[0]].offers[0]
                .lines
                .is_empty()
        );
        process(&lobby, ids[1], &[0x30, 2, 0]);
        assert!(lobby.social.lock().unwrap().exchanges.is_empty());
        request(&lobby, &players);
        offer(&lobby, ids[0], 0, 110110001, 1);
        {
            let mut h = lobby.social.lock().unwrap();
            h.players.get_mut(&ids[1]).unwrap().s.position.x += 1000;
            maintain(&mut h, ids[1]);
            assert!(h.exchanges.is_empty());
            h.players.get_mut(&ids[1]).unwrap().s.position = players[1].s.position;
        }
        for reason in ["external", "timeout", "death", "teleport", "service"] {
            request(&lobby, &players);
            offer(&lobby, ids[0], 0, 110110001, 1);
            let mut h = lobby.social.lock().unwrap();
            match reason {
                "external" => h.send(ids[1], vec![0x26, 1, 0]),
                "timeout" => {
                    h.exchanges.get_mut(&ids[0]).unwrap().until = 0;
                    maintain(&mut h, ids[0]);
                }
                "death" => {
                    h.players.get_mut(&ids[1]).unwrap().s.skills.hp = 0;
                    maintain(&mut h, ids[1]);
                }
                "teleport" => {
                    h.players.get_mut(&ids[1]).unwrap().s.position.zone = 2;
                    maintain(&mut h, ids[1]);
                }
                _ => before_action(&mut h, ids[0], 0x45),
            }
            assert!(h.exchanges.is_empty(), "{reason}");
            h.players.get_mut(&ids[1]).unwrap().s = players[1].s.clone();
        }
        request(&lobby, &players);
        offer(&lobby, ids[0], 0, 110110001, 1);
        lobby.social.lock().unwrap().disconnect(ids[1]);
        assert!(lobby.social.lock().unwrap().exchanges.is_empty());
        assert_eq!(saved(&lobby), before);
    }
}
