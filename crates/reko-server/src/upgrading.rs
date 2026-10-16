//! Map anvil objects and transactional reference-table upgrades.
use crate::{combat, inventory, lobby::Lobby, movement::Position, starter, world_entry::Session};
use rusqlite::{OptionalExtension, params};
use std::{collections::HashSet, net::SocketAddr};
use tokio::net::TcpStream;

fn in_range(a: Position, b: Position) -> bool {
    a.zone == b.zone
        && u32::from(a.x.abs_diff(b.x)).pow(2) + u32::from(a.z.abs_diff(b.z)).pow(2) <= 10000
}
fn object(lobby: &Lobby, s: &Session, index: u16) -> anyhow::Result<(u8, Position)> {
    let db = lobby.db.lock().unwrap();
    Ok(db.query_row("SELECT CAST(json_extract(data,'$.Type') AS INTEGER),CAST(round(json_extract(data,'$.PosX')*10) AS INTEGER),CAST(round(json_extract(data,'$.PosZ')*10) AS INTEGER) FROM world_reference_rules WHERE table_name='K_OBJECTPOS' AND rule_id=?1",[u32::from(s.position.zone)*100000+u32::from(index)],|r|Ok((r.get(0)?,Position{zone:s.position.zone,x:r.get(1)?,z:r.get(2)?,y:0})))?)
}
struct Recipe {
    index: u32,
    origin: u32,
    kind: i32,
    required: Vec<u32>,
    coins: u32,
    tier: u8,
    rate: u32,
    delta: i32,
    npc: u16,
}
fn recipes(db: &rusqlite::Connection) -> anyhow::Result<Vec<Recipe>> {
    let fields = [
        "nIndex",
        "nOriginItem",
        "nOriginType",
        "nReqNoah",
        "bRateType",
        "nGenRate",
        "nGiveItem",
        "nNPCNum",
        "nReqItem1",
        "nReqItem2",
        "nReqItem3",
        "nReqItem4",
        "nReqItem5",
        "nReqItem6",
        "nReqItem7",
        "nReqItem8",
    ];
    let sql = format!(
        "SELECT {} FROM world_reference_rules WHERE table_name='ITEM_UPGRADE' ORDER BY rule_id",
        fields
            .map(|f| format!("CAST(json_extract(data,'$.{f}') AS INTEGER)"))
            .join(",")
    );
    Ok(db
        .prepare(&sql)?
        .query_map([], |r| {
            Ok(Recipe {
                index: r.get(0)?,
                origin: r.get(1)?,
                kind: r.get(2)?,
                coins: r.get(3)?,
                tier: r.get(4)?,
                rate: r.get(5)?,
                delta: r.get(6)?,
                npc: r.get(7)?,
                required: (8..16).map(|i| r.get(i)).collect::<rusqlite::Result<_>>()?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}
fn kind_matches(kind: i32, weapon: i32, id: u32) -> bool {
    match kind {
        -1 => true,
        0 => weapon == 110,
        1 => weapon == 21,
        2 => weapon == 22,
        3 => weapon == 31,
        4 => weapon == 32,
        5 => weapon == 41,
        6 => weapon == 42,
        7 => weapon == 51,
        8 => weapon == 52,
        9 => matches!(weapon, 70 | 71),
        10 => weapon == 110,
        11 => id / 10000000 == 19,
        12 => weapon == 60,
        13 => matches!(weapon, 210 | 220 | 230 | 240),
        14 => weapon == 11,
        _ => false,
    }
}
type Outcome = ([starter::Item; 28], u32, Vec<u8>, u8);
fn attempt(lobby: &Lobby, account: &str, s: &Session, p: &[u8]) -> anyhow::Result<Outcome> {
    anyhow::ensure!(
        s.entered && s.skills.hp > 0 && p.get(1) == Some(&2),
        "upgrade unavailable"
    );
    let &(object_id, npc, position) = s
        .anvil
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("open an anvil first"))?;
    anyhow::ensure!(in_range(s.position, position), "anvil out of range");
    let modern = p.len() == 55;
    anyhow::ensure!(modern || p.len() == 54, "malformed upgrade request");
    let mode = if modern { p[2] } else { 1 };
    anyhow::ensure!(matches!(mode, 1 | 2), "invalid upgrade mode");
    let offset = if modern { 3 } else { 2 };
    anyhow::ensure!(
        u16::from_le_bytes([p[offset], p[offset + 1]]) == npc,
        "anvil mismatch"
    );
    let mut entries = [(0u32, 255u8); 10];
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let mut bag = starter::inventory(&tx, account, s.character.slot)?;
    let mut slots = HashSet::new();
    for (i, e) in entries.iter_mut().enumerate() {
        let start = offset + 2 + i * 5;
        *e = (
            u32::from_le_bytes(p[start..start + 4].try_into()?),
            p[start + 4],
        );
        if e.1 == 255 {
            anyhow::ensure!(e.0 == 0, "item without a slot");
            continue;
        }
        let item = bag
            .get(usize::from(e.1))
            .ok_or_else(|| anyhow::anyhow!("invalid slot"))?;
        anyhow::ensure!(
            slots.insert(e.1) && item.id == e.0 && item.id != 0 && item.count > 0,
            "item ownership or duplicate slot"
        );
    }
    let (origin, origin_pos) = entries[0];
    anyhow::ensure!(
        origin_pos < 28 && bag[usize::from(origin_pos)].count == 1,
        "invalid origin"
    );
    let catalog = lobby
        .combat
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("catalog unavailable"))?;
    let (base, ext) = catalog
        .item(origin)
        .ok_or_else(|| anyhow::anyhow!("unknown item"))?;
    anyhow::ensure!(ext.is_some(), "unknown extension");
    let (class, _ext): (u8, u8) = tx
        .query_row(
            "SELECT item_class,item_ext FROM world_upgrade_item_types WHERE item_id=?1",
            [origin / 1000 * 1000],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("missing upgrade metadata"))?;
    // Accessory/rebirth exchanges use separate source handlers.
    anyhow::ensure!(matches!(class, 1..=4), "unsupported upgrade class");
    let supplied: Vec<u32> = entries[1..]
        .iter()
        .filter(|(_, pos)| *pos != 255)
        .map(|(id, _)| *id)
        .collect();
    let bonus = supplied
        .iter()
        .copied()
        .find(|id| matches!(id, 700002000 | 379258000));
    let mut ingredients = supplied.clone();
    if let Some(id) = bonus {
        ingredients.remove(ingredients.iter().position(|i| *i == id).unwrap());
    }
    ingredients.sort_unstable();
    let recipe = recipes(&tx)?
        .into_iter()
        .find(|r| {
            let mut required: Vec<_> = r.required.iter().copied().filter(|i| *i != 0).collect();
            required.sort_unstable();
            let class_ok = match r.tier {
                1 => class <= 1,
                2 => class <= 2,
                3 => class <= 3,
                4 => class == 4,
                _ => false,
            };
            r.origin == origin % 100000
                && (r.index >= 300000 || r.index / 100000 == origin / 100000000)
                && (r.npc == 0 || r.npc == object_id)
                && required == ingredients
                && class_ok
                && (r.index >= 200000 || kind_matches(r.kind, combat::integer(base, 10), origin))
                && bonus.is_none_or(|id| {
                    if r.tier == 4 {
                        id == 379258000
                    } else {
                        id == 700002000
                    }
                })
        })
        .ok_or_else(|| anyhow::anyhow!("no matching original upgrade recipe"))?;
    let coins = starter::coins(&tx, account, s.character.slot)?;
    anyhow::ensure!(coins >= recipe.coins, "insufficient coins");
    let upgraded = origin
        .checked_add_signed(recipe.delta)
        .ok_or_else(|| anyhow::anyhow!("invalid upgrade delta"))?;
    let (newbase, newext) = catalog
        .item(upgraded)
        .ok_or_else(|| anyhow::anyhow!("result absent from client"))?;
    let newext = newext.ok_or_else(|| anyhow::anyhow!("result extension absent from client"))?;
    let mut rng = crate::skills::now_ms();
    let upper = crate::loot::roll(&mut rng, 9000, 10000);
    let success = mode == 2
        || crate::loot::roll(&mut rng, 0, upper)
            <= recipe
                .rate
                .saturating_add(if bonus.is_some() { 1000 } else { 0 })
                .min(10000);
    let mut response = vec![0x5b, 2, mode, u8::from(success)];
    if success {
        entries[0].0 = upgraded;
    }
    for (id, pos) in entries {
        response.extend(id.to_le_bytes());
        response.push(pos);
    }
    if mode == 2 {
        return Ok((bag, coins, response, 1));
    }
    let pos = usize::from(origin_pos);
    bag[pos] = if success {
        starter::Item {
            id: upgraded,
            count: 1,
            durability: (combat::integer(newbase, 19) + combat::integer(newext, 12)).clamp(0, 65535)
                as u16,
        }
    } else {
        starter::Item::default()
    };
    for &(_, pos) in &entries[1..] {
        if pos == 255 {
            continue;
        }
        let i = &mut bag[usize::from(pos)];
        i.count -= 1;
        if i.count == 0 {
            *i = starter::Item::default();
        }
    }
    for pos in slots {
        let i = bag[usize::from(pos)];
        tx.execute(
            "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
            params![account, s.character.slot, pos],
        )?;
        if i.id != 0 {
            tx.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?1,?2,?3,?4,?5,?6)",params![account,s.character.slot,pos,i.id,i.durability,i.count])?;
        }
    }
    let coins = coins - recipe.coins;
    anyhow::ensure!(
        tx.execute(
            "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
            params![account, s.character.slot, coins]
        )? == 1,
        "wallet missing"
    );
    tx.commit()?;
    tracing::info!(character=%s.character.name,origin,upgraded,success,cost=recipe.coins,"anvil upgrade committed");
    Ok((bag, coins, response, u8::from(success)))
}
pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) {
    if p.first() == Some(&0x33) {
        if p.len() != 5 || !s.entered || s.skills.hp == 0 {
            return;
        }
        let index = u16::from_le_bytes([p[1], p[2]]);
        let npc = u16::from_le_bytes([p[3], p[4]]);
        match object(lobby, s, index) {
            Ok((8, position)) if in_range(s.position, position) => {
                let Some(instance) = lobby.merchants.iter().find(|m| {
                    m.kind == 24 && m.prototype == index && m.nearby(s) && (npc == 0 || m.id == npc)
                }) else {
                    tracing::info!(%peer,index,npc,"anvil instance unavailable or mismatched");
                    return;
                };
                let npc = instance.id;
                s.anvil = Some((index, npc, position));
                s.merchant = None;
                let mut reply = vec![0x5b, 1];
                reply.extend(npc.to_le_bytes());
                super::send_response(sock, peer, &None, &reply, "ANVIL_OPEN").await;
            }
            Ok((kind, _)) => {
                super::send_response(sock, peer, &None, &[0x33, kind, 0], "OBJECT_UNAVAILABLE")
                    .await
            }
            Err(e) => {
                tracing::info!(%peer,%e,index,"unknown map object");
            }
        }
        return;
    }
    match attempt(lobby, account, s, p) {
        Ok((bag, coins, response, effect)) => {
            let changed: Vec<_> = bag
                .iter()
                .enumerate()
                .filter(|(p, i)| s.inventory[*p] != **i)
                .map(|(p, i)| (p, *i))
                .collect();
            let spent = s.coins.saturating_sub(coins);
            s.inventory = bag;
            s.coins = coins;
            super::send_response(sock, peer, &None, &response, "ANVIL_RESULT").await;
            if spent > 0 {
                let gold = crate::wallet::change(coins + spent, coins);
                super::send_response(sock, peer, &None, &gold, "ANVIL_COINS").await;
            }
            if !changed.is_empty() {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &inventory::count_packet(&changed),
                    "ANVIL_ITEMS",
                )
                .await;
            }
            if p.len() != 55 || p[2] == 1 {
                let mut event = vec![0x33, 8, effect];
                event.extend(s.anvil.unwrap().1.to_le_bytes());
                super::send_response(sock, peer, &None, &event, "ANVIL_EFFECT").await;
            }
            if let Some(c) = lobby.combat.as_ref() {
                crate::durability::refresh(s, c, lobby.skills.as_ref());
                crate::durability::attributes(sock, peer, s).await;
            }
        }
        Err(e) => {
            tracing::info!(%peer,%e,"anvil request refused");
            super::send_response(sock, peer, &None, &[0x5b, 2, 1, 4], "ANVIL_REFUSED").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn visible_anvil_opens_from_map_or_npc_and_resolves_real_instance_id() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute("INSERT INTO world_reference_rules VALUES ('K_OBJECTPOS',2105001,?1)",[r#"{"ZoneID":"21","sIndex":"5001","Type":"8","Status":"1","PosX":"337.887726","PosY":"13.5151672","PosZ":"389.9475"}"#]).unwrap();
            db.execute("INSERT INTO world_reference_rules VALUES ('OBJECT_NPC',5001,?1)",[r#"{"sSid":"5001","sPid":"5001","strName":"Magic anvil","sSize":"100","byType":"24","iSellingGroup":"0","sLevel":"60"}"#]).unwrap();
        }
        lobby.merchants = crate::shops::load_objects(&lobby.db.lock().unwrap()).unwrap();
        s.position = lobby.merchants[0].position;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        use tokio::io::AsyncReadExt;
        for p in [
            vec![0x33, 0x89, 0x13, 0, 0],
            vec![0x33, 0x89, 0x13, 0x30, 0x75],
            vec![0x20, 0x30, 0x75],
        ] {
            if p[0] == 0x33 {
                handle(&mut socket, peer, &lobby, "test", &mut s, &p).await;
            } else {
                crate::shops::handle(&mut socket, peer, &lobby, "test", &mut s, &p).await;
            }
            let mut reply = [0u8; 10];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply[4..8], &[0x5b, 1, 0x30, 0x75]);
            assert_eq!(s.anvil.unwrap().1, 30000);
        }
        s.anvil = None;
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x33, 0x89, 0x13, 1, 0],
        )
        .await;
        assert!(s.anvil.is_none());
        s.position.x += 101;
        handle(
            &mut socket,
            peer,
            &lobby,
            "test",
            &mut s,
            &[0x33, 0x89, 0x13, 0x30, 0x75],
        )
        .await;
        assert!(s.anvil.is_none());
    }

    #[test]
    fn preview_preserves_items_and_upgrade_commits_once_without_slot_forgery() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        s.anvil = Some((5001, 0, s.position));
        {
            let db = lobby.db.lock().unwrap();
            db.execute("INSERT INTO character_inventory VALUES ('test',0,0,110110001,100,1),('test',0,1,379021000,1,2)",[]).unwrap();
            db.execute(
                "INSERT INTO world_upgrade_item_types VALUES (110110000,1,0)",
                [],
            )
            .unwrap();
            db.execute("INSERT INTO world_reference_rules VALUES ('ITEM_UPGRADE',100000,?1)",[r#"{"nIndex":100000,"nOriginItem":10001,"nOriginType":-1,"nReqNoah":100,"bRateType":3,"nGenRate":10000,"nGiveItem":1,"nNPCNum":5001,"nReqItem1":379021000,"nReqItem2":0,"nReqItem3":0,"nReqItem4":0,"nReqItem5":0,"nReqItem6":0,"nReqItem7":0,"nReqItem8":0}"#]).unwrap();
        }
        let mut p = vec![0x5b, 2, 2, 0, 0];
        for (id, pos) in [(110110001u32, 0), (379021000, 1)]
            .into_iter()
            .chain(std::iter::repeat_n((0, 255), 8))
        {
            p.extend(id.to_le_bytes());
            p.push(pos);
        }
        let (_, coins, reply, _) = attempt(&lobby, "test", &s, &p).unwrap();
        assert_eq!(coins, 1000000);
        assert_eq!(reply.len(), 54);
        assert_eq!(
            u32::from_le_bytes(reply[4..8].try_into().unwrap()),
            110110002
        );
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[0].id,
            110110001
        );
        p[2] = 1;
        let mut duplicate = p.clone();
        duplicate[14] = 0;
        assert!(attempt(&lobby, "test", &s, &duplicate).is_err());
        let (bag, coins, reply, _) = attempt(&lobby, "test", &s, &p).unwrap();
        assert_eq!(
            (bag[0].id, bag[1].count, coins, reply[3]),
            (110110002, 1, 999900, 1)
        );
        assert!(bag[0].durability > 100);
        assert!(attempt(&lobby, "test", &s, &p).is_err());
        assert_eq!(
            starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            999900
        );
        s.position.x += 101;
        assert!(attempt(&lobby, "test", &s, &p).is_err());
    }
}
