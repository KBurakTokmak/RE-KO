//! Authoritative six-slot corpse bundles and atomic pickup.
use crate::{
    combat, inventory, lobby::Lobby, movement::Position, skills, starter, world_entry::Session,
};
use rusqlite::{Connection, params};
use std::{collections::HashMap, net::SocketAddr, sync::Mutex};
use tokio::net::TcpStream;

const GOLD: u32 = 900000000;
pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(include_str!("world_services.sql"))
}
struct Bundle {
    owner: String,
    position: Position,
    items: Vec<(u32, u16)>,
}
struct Inner {
    bundles: HashMap<u32, Bundle>,
    next: u32,
    rng: u64,
    nations: HashMap<String, u8>,
}
pub(crate) struct Book {
    inner: Mutex<Inner>,
    monsters: HashMap<u16, (u32, u32)>,
    rules: HashMap<u32, Vec<(u8, u32, u16)>>,
    groups: HashMap<u32, Vec<u32>>,
    valid: std::collections::HashSet<u32>,
    generator: crate::loot_generation::Generator,
    levels: HashMap<u16, u8>,
}
pub(super) fn roll(seed: &mut u64, lo: u32, hi: u32) -> u32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    lo + (*seed >> 16) as u32 % (hi - lo + 1)
}
impl Book {
    pub fn load(db: &Connection, catalog: &combat::Catalog) -> anyhow::Result<Self> {
        let mut monsters = HashMap::new();
        for row in db
            .prepare("SELECT prototype,coins,drop_id FROM world_monster_loot")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        {
            let (k, v) = row?;
            monsters.insert(k, v);
        }
        let mut rules: HashMap<u32, Vec<(u8, u32, u16)>> = HashMap::new();
        for row in db
            .prepare("SELECT drop_id,slot,item,chance FROM world_drop_rules ORDER BY drop_id,slot")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        {
            let (k, s, i, p) = row?;
            rules.entry(k).or_default().push((s, i, p));
        }
        let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
        for row in db
            .prepare("SELECT group_id,item FROM world_drop_groups ORDER BY group_id,slot")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        {
            let (k, i) = row?;
            groups.entry(k).or_default().push(i);
        }
        let valid: std::collections::HashSet<u32> = catalog.item_ids().collect();
        let unsupported: std::collections::HashSet<_> = rules
            .values()
            .flatten()
            .filter(|(slot, id, _)| *slot >= 2 && *id != 0)
            .map(|(_, id, _)| *id)
            .chain(groups.values().flatten().copied().filter(|id| *id != 0))
            .filter(|id| !valid.contains(id))
            .collect();
        tracing::info!(
            monster_tables = monsters.len(),
            drop_tables = rules.len(),
            item_groups = groups.len(),
            unsupported_item_ids = unsupported.len(),
            "source loot rules loaded; unsupported client items retain no-drop probability"
        );
        let mut levels: HashMap<u16, u8> = db
            .prepare(
                "SELECT prototype,MAX(level) FROM world_spawns WHERE monster=1 GROUP BY prototype",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for row in db.prepare("SELECT rule_id,json_extract(data,'$.level') FROM world_reference_rules WHERE table_name='MONSTER_TEMPLATE'")?
            .query_map([], |r| Ok((r.get::<_, u16>(0)?, r.get::<_, u8>(1)?)))? {
            let (id, level) = row?;
            levels.entry(id).or_insert(level);
        }
        Ok(Self {
            inner: Mutex::new(Inner {
                bundles: HashMap::new(),
                next: 1,
                rng: skills::now_ms(),
                nations: HashMap::new(),
            }),
            monsters,
            rules,
            groups,
            valid,
            generator: crate::loot_generation::Generator::load(db)?,
            levels,
        })
    }
    pub fn register_owner(&self, name: &str, nation: u8) {
        self.inner
            .lock()
            .unwrap()
            .nations
            .insert(name.to_ascii_lowercase(), nation);
    }
    pub fn create(
        &self,
        prototype: u16,
        npc: u16,
        position: Position,
        owner: &str,
    ) -> Option<Vec<u8>> {
        let (money, drop) = *self.monsters.get(&prototype)?;
        let mut inner = self.inner.lock().unwrap();
        // TODO(re): Source cleanup lifetime is unverified; keep bundles until emptied/restart.
        let mut items = Vec::new();
        let money = money * roll(&mut inner.rng, 70, 100) / 100;
        let money = if money >= 32767 { 32000 } else { money } as u16;
        if money > 0 {
            items.push((GOLD, money));
        }
        if let Some(rules) = self.rules.get(&drop) {
            for &(slot, item, chance) in rules {
                if item == 0 || chance == 0 || roll(&mut inner.rng, 1, 10000) > u32::from(chance) {
                    continue;
                }
                let id = if slot < 2 {
                    if item < 100 {
                        let nation = inner
                            .nations
                            .get(&owner.to_ascii_lowercase())
                            .copied()
                            .unwrap_or(0);
                        self.generator.generate(
                            item,
                            self.levels.get(&prototype).copied().unwrap_or(0),
                            nation,
                            &mut inner.rng,
                        )
                    } else if let Some(group) = self.groups.get(&item).filter(|g| g.len() == 30) {
                        group[roll(&mut inner.rng, 0, 29) as usize]
                    } else {
                        tracing::warn!(
                            prototype,
                            group = item,
                            "unsupported procedural loot group"
                        );
                        0
                    }
                } else {
                    item
                };
                if id != 0 && self.valid.contains(&id) {
                    items.push((
                        id,
                        if slot >= 2 && (391010000..392010000).contains(&id) {
                            20
                        } else {
                            1
                        },
                    ));
                }
            }
        }
        if items.is_empty() {
            return None;
        }
        let id = inner.next;
        inner.next = inner.next.checked_add(1)?;
        inner.bundles.insert(
            id,
            Bundle {
                owner: owner.to_ascii_lowercase(),
                position,
                items,
            },
        );
        let mut p = vec![0x23];
        p.extend(npc.to_le_bytes());
        p.extend(id.to_le_bytes());
        Some(p)
    }
    pub fn visible(&self, id: u32, name: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .bundles
            .get(&id)
            .is_some_and(|b| b.owner.eq_ignore_ascii_case(name))
    }
}
fn allowed(bundle: &Bundle, session: &Session) -> bool {
    session.entered
        && session.skills.hp > 0
        && bundle.owner.eq_ignore_ascii_case(&session.character.name)
        && bundle.position.zone == session.position.zone
        && u64::from(bundle.position.x.abs_diff(session.position.x)).pow(2)
            + u64::from(bundle.position.z.abs_diff(session.position.z)).pow(2)
            <= 24200
}
type Pickup = (Vec<u8>, Vec<(usize, starter::Item)>);
fn pickup(
    book: &Book,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    peer: SocketAddr,
    id: u32,
    item: u32,
) -> anyhow::Result<Pickup> {
    let mut inner = book.inner.lock().unwrap();
    let b = inner
        .bundles
        .get_mut(&id)
        .ok_or_else(|| anyhow::anyhow!("unknown bundle"))?;
    anyhow::ensure!(allowed(b, session), "loot ownership/range/life mismatch");
    let index = b
        .items
        .iter()
        .position(|(i, c)| *i == item && *c > 0)
        .ok_or_else(|| anyhow::anyhow!("item no longer in bundle"))?;
    let count = b.items[index].1;
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let mut coins = starter::coins(&tx, account, session.character.slot)?;
    let mut changed = Vec::new();
    let mut bag = starter::inventory(&tx, account, session.character.slot)?;
    let mut position = 255u8;
    let mut total = count;
    if item == GOLD {
        coins = coins
            .checked_add(u32::from(count))
            .filter(|v| *v <= 2100000000)
            .ok_or_else(|| anyhow::anyhow!("coin limit"))?;
        anyhow::ensure!(
            tx.execute(
                "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
                params![account, session.character.slot, coins],
            )? == 1,
            "wallet missing"
        );
    } else {
        let catalog = lobby
            .combat
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("item catalog unavailable"))?;
        let (base, extension) = catalog
            .item(item)
            .ok_or_else(|| anyhow::anyhow!("unknown item"))?;
        let extension = extension.ok_or_else(|| anyhow::anyhow!("unknown item extension"))?;
        let stackable = combat::integer(base, 23) != 0;
        let pos = bag
            .iter()
            .position(|i| {
                stackable && i.id == item && u32::from(i.count) + u32::from(count) <= 9999
            })
            .or_else(|| bag.iter().position(|i| i.id == 0))
            .ok_or_else(|| anyhow::anyhow!("inventory full"))?;
        if bag[pos].id == 0 {
            bag[pos] = starter::Item {
                id: item,
                count: 0,
                durability: (combat::integer(base, 19) + combat::integer(extension, 12))
                    .clamp(0, 65535) as u16,
            };
        }
        bag[pos].count = bag[pos]
            .count
            .checked_add(count)
            .filter(|v| *v <= 9999)
            .ok_or_else(|| anyhow::anyhow!("stack limit"))?;
        let weight = u32::from(catalog.bag_weight(&bag))
            + u32::from(
                catalog
                    .calculate(&session.character, &session.progress, &session.equipment)?
                    .weight,
            );
        anyhow::ensure!(
            weight <= u32::from(session.combat.max_weight),
            "weight limit"
        );
        tx.execute("INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(account,slot,inventory_slot) DO UPDATE SET item_id=excluded.item_id,durability=excluded.durability,count=excluded.count",params![account,session.character.slot,pos,item,bag[pos].durability,bag[pos].count])?;
        position = pos as u8;
        total = bag[pos].count;
        changed.push((pos, bag[pos]));
    }
    tx.commit()?;
    b.items.remove(index);
    if b.items.is_empty() {
        inner.bundles.remove(&id);
    }
    session.inventory = bag;
    session.coins = coins;
    let mut response = vec![0x26, 1, position];
    response.extend(item.to_le_bytes());
    response.extend(total.to_le_bytes());
    response.extend(coins.to_le_bytes());
    tracing::info!(%peer,bundle=id,item,count,coins,"loot pickup committed");
    Ok((response, changed))
}
pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    p: &[u8],
) {
    let Some(book) = lobby.monsters.as_ref().and_then(|w| w.loot.as_ref()) else {
        return;
    };
    if p.len() < 5 {
        return;
    }
    let id = u32::from_le_bytes(p[1..5].try_into().unwrap());
    if p[0] == 0x24 {
        let mut response = vec![0x24];
        {
            let inner = book.inner.lock().unwrap();
            let Some(b) = inner.bundles.get(&id).filter(|b| allowed(b, session)) else {
                return;
            };
            for index in 0..6 {
                let (item, count) = b.items.get(index).copied().unwrap_or_default();
                response.extend(item.to_le_bytes());
                response.extend(count.to_le_bytes());
            }
        }
        super::send_response(sock, peer, &None, &response, "LOOT_OPEN").await;
        return;
    }
    if p.len() != 9 {
        return;
    }
    let item = u32::from_le_bytes(p[5..9].try_into().unwrap());
    let result = pickup(book, lobby, account, session, peer, id, item);
    match result {
        Ok((response, changed)) => {
            super::send_response(sock, peer, &None, &response, "LOOT_GET").await;
            if !changed.is_empty() {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &inventory::count_packet(&changed),
                    "LOOT_ITEMS",
                )
                .await;
            }
            if let Some(c) = lobby.combat.as_ref() {
                crate::durability::refresh(session, c, lobby.skills.as_ref());
                crate::durability::attributes(sock, peer, session).await;
            }
        }
        Err(e) => {
            tracing::info!(%peer,%e,"loot refused");
            super::send_response(sock, peer, &None, &[0x26, 0], "LOOT_FAILED").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_drop_rolls_atomic_pickup_ownership_and_replay() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute("INSERT INTO world_monster_loot VALUES (100,1000,1)", [])
                .unwrap();
            db.execute(
                "INSERT INTO world_drop_rules VALUES (1,2,389014000,10000),(1,3,389020000,0)",
                [],
            )
            .unwrap();
        }
        let book = Book::load(&lobby.db.lock().unwrap(), lobby.combat.as_ref().unwrap()).unwrap();
        let notification = book
            .create(100, 12000, s.position, &s.character.name)
            .unwrap();
        assert_eq!(notification.len(), 7);
        let id = u32::from_le_bytes(notification[3..7].try_into().unwrap());
        let money = book.inner.lock().unwrap().bundles[&id].items[0].1;
        assert!((700..=1000).contains(&money));
        assert_eq!(book.inner.lock().unwrap().bundles[&id].items.len(), 2);
        let peer = "127.0.0.1:1".parse().unwrap();
        s.character.name = "Thief".into();
        assert!(pickup(&book, &lobby, "test", &mut s, peer, id, GOLD).is_err());
        s.character.name = "ServiceTest".into();
        s.position.x += 200;
        assert!(pickup(&book, &lobby, "test", &mut s, peer, id, GOLD).is_err());
        s.position.x -= 200;
        let (reply, _) = pickup(&book, &lobby, "test", &mut s, peer, id, GOLD).unwrap();
        assert_eq!(reply.len(), 13);
        assert_eq!(s.coins, 1000000 + u32::from(money));
        assert!(pickup(&book, &lobby, "test", &mut s, peer, id, GOLD).is_err());
        {
            let db = lobby.db.lock().unwrap();
            for pos in 0..28 {
                db.execute(
                    "INSERT INTO character_inventory VALUES ('test',0,?1,110110001,4000,1)",
                    [pos],
                )
                .unwrap();
            }
        }
        assert!(pickup(&book, &lobby, "test", &mut s, peer, id, 389014000).is_err());
        assert_eq!(
            book.inner.lock().unwrap().bundles[&id].items,
            vec![(389014000, 1)]
        );
        lobby
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM character_inventory WHERE inventory_slot=0", [])
            .unwrap();
        let (reply, _) = pickup(&book, &lobby, "test", &mut s, peer, id, 389014000).unwrap();
        assert_eq!(reply.len(), 13);
        assert!(!book.visible(id, &s.character.name));
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[0].id,
            389014000
        );
    }
}
