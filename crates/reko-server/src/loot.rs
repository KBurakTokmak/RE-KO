//! Authoritative six-slot corpse bundles and atomic pickup.
use crate::{
    combat, inventory, lobby::Lobby, movement::Position, skills, starter, world_entry::Session,
};
use rusqlite::{Connection, params};
use std::{collections::HashMap, net::SocketAddr, sync::Mutex};
use tokio::net::TcpStream;

const GOLD: u32 = 900000000;
pub(crate) fn command(s: &mut Session, text: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(text)
        .ok()?
        .trim()
        .trim_start_matches('/');
    let mut words = text.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("autoloot") {
        return None;
    }
    let argument = words.next();
    let message = if words.next().is_some() {
        "Usage: /autoloot [on|off]"
    } else if argument.is_none() {
        s.autoloot = if s.autoloot.is_some() { None } else { Some(0) };
        if s.autoloot.is_some() {
            "Autoloot enabled."
        } else {
            "Autoloot disabled."
        }
    } else if argument.is_some_and(|a| a.eq_ignore_ascii_case("on")) {
        s.autoloot = Some(0);
        "Autoloot enabled."
    } else if argument.is_some_and(|a| a.eq_ignore_ascii_case("off")) {
        s.autoloot = None;
        "Autoloot disabled."
    } else {
        "Usage: /autoloot [on|off]"
    };
    Some(crate::chat::packet(
        7,
        s.character.nation(),
        u16::MAX,
        "",
        message.as_bytes(),
    ))
}

pub(crate) async fn auto_tick(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    now: u64,
) {
    let Some(next) = s.autoloot else {
        return;
    };
    if now < next || !s.entered || s.skills.hp == 0 {
        return;
    }
    s.autoloot = Some(now.saturating_add(1000));
    // Native service windows temporarily own the bag icons.
    if lobby.merchants.iter().any(|m| {
        (s.merchant == Some(m.id) || s.anvil.is_some_and(|(_, id, _)| id == m.id)) && m.nearby(s)
    }) {
        return;
    }
    let Some(book) = lobby.monsters.as_ref().and_then(|w| w.loot.as_ref()) else {
        return;
    };
    let candidates = {
        let inner = book.inner.lock().unwrap();
        let mut entries: Vec<_> = inner
            .bundles
            .iter()
            .filter(|(_, b)| allowed(b, s))
            .flat_map(|(&id, b)| {
                b.items
                    .iter()
                    .filter(|(_, count)| *count > 0)
                    .map(move |&(item, _)| (id, item))
            })
            .collect();
        entries.sort_unstable();
        entries.truncate(32);
        entries
    };
    let mut collected = false;
    for (id, item) in candidates {
        if let Ok((response, changed)) = pickup_routed(book, lobby, account, s, peer, id, item) {
            super::send_response(sock, peer, &None, &response, "AUTOLOOT_GET").await;
            if !changed.is_empty() {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &inventory::count_packet(&changed),
                    "AUTOLOOT_ITEMS",
                )
                .await;
            }
            collected = true;
        }
    }
    s.autoloot = Some(now.saturating_add(1000));
    if collected && let Some(catalog) = &lobby.combat {
        crate::durability::refresh(s, catalog, lobby.skills.as_ref());
        crate::durability::attributes(sock, peer, s).await;
    }
}
pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(include_str!("world_services.sql"))
}
struct Bundle {
    owners: Vec<String>,
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
    pub social: Option<std::sync::Arc<Mutex<crate::social::Hub>>>,
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
            social: None,
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
        let owners = self
            .social
            .as_ref()
            .and_then(|hub| {
                let h = hub.lock().unwrap();
                h.named(owner).map(|id| {
                    if prototype == 2690 && position.zone == 54 && h.players[&id].s.clan.id != 0 {
                        let clan = h.players[&id].s.clan.id;
                        return h
                            .players
                            .values()
                            .filter(|p| p.s.clan.id == clan && p.s.clan.fame == 1)
                            .map(|p| p.s.character.name.to_ascii_lowercase())
                            .collect();
                    }
                    h.group(id)
                        .iter()
                        .map(|id| h.players[id].s.character.name.to_ascii_lowercase())
                        .collect()
                })
            })
            .unwrap_or_else(|| vec![owner.to_ascii_lowercase()]);
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
                owners,
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
            .is_some_and(|b| {
                b.owners
                    .iter()
                    .any(|owner| owner.eq_ignore_ascii_case(name))
            })
    }
}
fn allowed(bundle: &Bundle, session: &Session) -> bool {
    session.entered
        && session.skills.hp > 0
        && bundle
            .owners
            .iter()
            .any(|owner| owner.eq_ignore_ascii_case(&session.character.name))
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
    if item == GOLD && b.owners.len() > 1 {
        let group = {
            let h = lobby.social.lock().unwrap();
            if h.party(session.player_id).is_some() {
                h.group(session.player_id)
                    .into_iter()
                    .map(|id| (id, h.players[&id].clone()))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        };
        if group.len() > 1 {
            let total_levels: u32 = group
                .iter()
                .map(|(_, p)| u32::from(p.s.progress.level))
                .sum();
            let db = lobby.db.lock().unwrap();
            let tx = db.unchecked_transaction()?;
            let mut updates = Vec::new();
            for (id, player) in group {
                if !allowed(b, &player.s) {
                    continue;
                }
                let amount = u32::from(count) * u32::from(player.s.progress.level) / total_levels;
                let amount = crate::costumes::gain(
                    amount,
                    crate::costumes::bonuses(&player.s.costumes).coins,
                );
                let before = starter::coins(&tx, &player.account, player.s.character.slot)?;
                let after = before
                    .checked_add(amount)
                    .filter(|v| *v <= 2100000000)
                    .ok_or_else(|| anyhow::anyhow!("party coin limit"))?;
                anyhow::ensure!(
                    tx.execute(
                        "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
                        params![player.account, player.s.character.slot, after]
                    )? == 1,
                    "party wallet missing"
                );
                let mut response = vec![0x26, 1, 255];
                response.extend(GOLD.to_le_bytes());
                response.extend((amount.min(u32::from(u16::MAX)) as u16).to_le_bytes());
                response.extend(after.to_le_bytes());
                updates.push((id, after, response));
            }
            anyhow::ensure!(!updates.is_empty(), "no eligible party loot members");
            tx.commit()?;
            drop(db);
            b.items.remove(index);
            if b.items.is_empty() {
                inner.bundles.remove(&id);
            }
            let mut own = None;
            let mut hub = lobby.social.lock().unwrap();
            for (target, coins, packet) in updates {
                if target == session.player_id {
                    session.coins = coins;
                    own = Some(packet);
                } else {
                    hub.refresh.insert(target);
                    hub.send(target, packet);
                }
            }
            return Ok((
                own.ok_or_else(|| anyhow::anyhow!("looter no longer eligible"))?,
                Vec::new(),
            ));
        }
    }
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let mut coins = starter::coins(&tx, account, session.character.slot)?;
    let mut changed = Vec::new();
    let mut bag = starter::inventory(&tx, account, session.character.slot)?;
    let mut position = 255u8;
    let total;
    if item == GOLD {
        let amount = crate::costumes::gain(
            u32::from(count),
            crate::costumes::bonuses(&session.costumes).coins,
        );
        total = amount.min(u32::from(u16::MAX)) as u16;
        coins = coins
            .checked_add(amount)
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
fn pickup_routed(
    book: &Book,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    peer: SocketAddr,
    id: u32,
    item: u32,
) -> anyhow::Result<Pickup> {
    if item == GOLD {
        return pickup(book, lobby, account, s, peer, id, item);
    }
    let owners = {
        let inner = book.inner.lock().unwrap();
        let bundle = inner
            .bundles
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown bundle"))?;
        anyhow::ensure!(allowed(bundle, s), "loot ownership/range/life mismatch");
        bundle.owners.clone()
    };
    let group = {
        let mut h = lobby.social.lock().unwrap();
        let Some(leader) = h
            .party(s.player_id)
            .filter(|leader| h.parties[leader].len() > 1)
        else {
            drop(h);
            return pickup(book, lobby, account, s, peer, id, item);
        };
        let mut ids = h.group(s.player_id);
        let offset = *h.loot_rotation.entry(leader).or_default() % ids.len();
        ids.rotate_left(offset);
        ids.into_iter()
            .filter(|id| {
                owners
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&h.players[id].s.character.name))
            })
            .map(|id| (leader, id, h.players[&id].clone()))
            .collect::<Vec<_>>()
    };
    for (leader, target, mut player) in group {
        match pickup(book, lobby, &player.account, &mut player.s, peer, id, item) {
            Ok((mut response, changed)) => {
                let mut h = lobby.social.lock().unwrap();
                if let Some(index) = h
                    .parties
                    .get(&leader)
                    .and_then(|ids| ids.iter().position(|id| *id == target))
                {
                    h.loot_rotation.insert(leader, index + 1);
                }
                let mut notice = vec![0x26, 3];
                notice.extend(id.to_le_bytes());
                notice.extend(item.to_le_bytes());
                crate::social::string(&mut notice, &player.s.character.name);
                let ids = h.group(s.player_id);
                h.send_many(&ids, &notice);
                if target == s.player_id {
                    *s = player.s;
                    return Ok((response, changed));
                }
                response[1] = 5;
                response.splice(2..2, id.to_le_bytes());
                h.send(target, response);
                if !changed.is_empty() {
                    h.send(target, inventory::count_packet(&changed));
                }
                if let Some(catalog) = &lobby.combat {
                    crate::durability::refresh(&mut player.s, catalog, lobby.skills.as_ref());
                    h.send(target, inventory::attributes(&player.s.combat, 2));
                }
                h.refresh.insert(target);
                return Ok((vec![0x26, 4], Vec::new()));
            }
            Err(error) => tracing::debug!(%error,target,"party loot recipient unavailable"),
        }
    }
    anyhow::bail!("no eligible party item recipient")
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
    let result = pickup_routed(book, lobby, account, session, peer, id, item);
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
    #[tokio::test]
    async fn autoloot_toggles_and_keeps_ownership_range_capacity_and_single_pickup() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let book = std::sync::Arc::new(
            Book::load(&lobby.db.lock().unwrap(), lobby.combat.as_ref().unwrap()).unwrap(),
        );
        let mut world = crate::monsters::World::load(&reko_common::game_dir()).unwrap();
        world.loot = Some(book.clone());
        lobby.monsters = Some(std::sync::Arc::new(world));
        {
            let mut inner = book.inner.lock().unwrap();
            for (id, owner, offset, item) in [
                (1, s.character.name.as_str(), 0, 389014000),
                (2, "SomeoneElse", 0, 389014000),
                (3, s.character.name.as_str(), 1000, 389014000),
            ] {
                let mut position = s.position;
                position.x += offset;
                inner.bundles.insert(
                    id,
                    Bundle {
                        owners: vec![owner.into()],
                        position,
                        items: vec![(item, 1)],
                    },
                );
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        auto_tick(&mut socket, peer, &lobby, "test", &mut s, 1000).await;
        assert!(s.inventory.iter().all(|i| i.id == 0));
        assert!(command(&mut s, b"/autoloot").is_some());
        auto_tick(&mut socket, peer, &lobby, "test", &mut s, 1000).await;
        assert_eq!(s.inventory[0].id, 389014000);
        auto_tick(&mut socket, peer, &lobby, "test", &mut s, 2000).await;
        assert_eq!(s.inventory[0].count, 1);
        assert_eq!(book.inner.lock().unwrap().bundles[&2].items[0].1, 1);
        assert_eq!(book.inner.lock().unwrap().bundles[&3].items[0].1, 1);
        command(&mut s, b"autoloot off");
        assert!(s.autoloot.is_none());
        command(&mut s, b"autoloot on extra");
        assert!(s.autoloot.is_none());
        {
            let db = lobby.db.lock().unwrap();
            db.execute("DELETE FROM character_inventory", []).unwrap();
            for pos in 0..28 {
                db.execute(
                    "INSERT INTO character_inventory VALUES('test',0,?1,110110001,4000,1)",
                    [pos],
                )
                .unwrap();
            }
            s.inventory = starter::inventory(&db, "test", 0).unwrap();
            book.inner.lock().unwrap().bundles.insert(
                4,
                Bundle {
                    owners: vec![s.character.name.clone()],
                    position: s.position,
                    items: vec![(389014000, 1)],
                },
            );
        }
        command(&mut s, b"/autoloot on");
        auto_tick(&mut socket, peer, &lobby, "test", &mut s, 3000).await;
        assert_eq!(book.inner.lock().unwrap().bundles[&4].items[0].1, 1);
        assert!(s.inventory.iter().all(|i| i.id == 110110001));
    }
    #[test]
    fn suppression_repair_preserves_other_loot_and_delivers_original_certificates_and_defense_armor()
     {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        if !root.join("target/debug/reko.exe").exists() {
            return;
        }
        let monsters = [
            (82, 7005, 910138000),
            (82, 7006, 910138000),
            (82, 7007, 910138000),
            (82, 7009, 910138000),
            (82, 7040, 910138000),
            (82, 7008, 910135000),
            (83, 7010, 910137000),
            (83, 7011, 910137000),
            (83, 7012, 910137000),
            (83, 7014, 910137000),
            (83, 7013, 910136000),
        ];
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "reko-suppression-{}-{stamp}.db",
            std::process::id()
        ));
        {
            let db = lobby.db.lock().unwrap();
            db.execute_batch(
                "INSERT INTO world_monster_loot VALUES (100,1000,100);
                INSERT INTO world_drop_rules VALUES (100,2,379021000,10000);",
            )
            .unwrap();
            for (exchange, first, second) in
                [(94, 910138000, 910135000), (98, 910137000, 910136000)]
            {
                let data = serde_json::json!({"nOriginItemNum1":first,"nOriginItemCount1":3,"nOriginItemNum2":second,"nOriginItemCount2":1});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES ('ITEM_EXCHANGE',?1,?2)",
                    params![exchange, data.to_string()],
                )
                .unwrap();
            }
            for (zone, prototype, _) in monsters {
                crate::population::tests::seed(&db);
                db.execute(
                    "UPDATE world_spawns SET id=?1,zone=?2,prototype=?3,source_row=?3 WHERE id=12000",
                    params![12000 + prototype, zone, prototype],
                )
                .unwrap();
                db.execute(
                    "INSERT INTO world_monster_loot VALUES (?1,1000,?1)",
                    [prototype],
                )
                .unwrap();
                db.execute(
                    "INSERT INTO world_drop_rules VALUES (?1,2,0,0)",
                    [prototype],
                )
                .unwrap();
            }
            db.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
                .unwrap();
        }
        let repair = || {
            let output = std::process::Command::new("python")
                .arg(root.join("scripts/Repair-1453SuppressionLoot.py"))
                .args(["--apply", "--db"])
                .arg(&path)
                .arg("--game-dir")
                .arg(reko_common::game_dir())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        repair();
        let db = Connection::open(&path).unwrap();
        let snapshot = || {
            let rules=db.query_row("SELECT group_concat(printf('%d:%d:%d:%d',drop_id,slot,item,chance),'|') FROM (SELECT * FROM world_drop_rules ORDER BY drop_id,slot)",[],|r|r.get::<_,String>(0)).unwrap();
            let groups=db.query_row("SELECT group_concat(printf('%d:%d:%d',group_id,slot,item),'|') FROM (SELECT * FROM world_drop_groups ORDER BY group_id,slot)",[],|r|r.get::<_,String>(0)).unwrap();
            (rules, groups)
        };
        let before = snapshot();
        repair();
        assert_eq!(before, snapshot());
        assert_eq!(
            db.query_row(
                "SELECT item FROM world_drop_rules WHERE drop_id=100 AND slot=2",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            379021000
        );
        assert_eq!(starter::coins(&db, "test", 0).unwrap(), 1000000);
        assert_eq!(db.query_row("SELECT count(*) FROM world_drop_groups WHERE group_id IN (82001,82002,83001,83002)",[],|r|r.get::<_,u32>(0)).unwrap(),120);
        let catalog = lobby.combat.as_ref().unwrap();
        for row in db
            .prepare("SELECT DISTINCT item FROM world_drop_groups")
            .unwrap()
            .query_map([], |r| r.get::<_, u32>(0))
            .unwrap()
        {
            let id = row.unwrap();
            assert_eq!(id % 1000, 541);
            let (base, ext) = catalog.item(id).unwrap();
            let part = combat::integer(base, 12);
            let (column, value) = match part {
                5 => (17, 5),
                6 => (18, 5),
                7 => (15, 3),
                8 => (16, 5),
                9 => (19, 3),
                _ => panic!("invalid armor part"),
            };
            assert_eq!(combat::integer(ext.unwrap(), column), value);
        }
        let mut book = Book::load(&db, catalog).unwrap();
        book.inner.get_mut().unwrap().rng = 42;
        let peer = "127.0.0.1:1".parse().unwrap();
        s.position.zone = 82;
        // Use the repaired book with the character's separate SQLite save, as normal pickup does.
        for (zone, prototype, certificate) in monsters {
            let mut position = s.position;
            position.zone = zone;
            let packet = book
                .create(prototype, 12000, position, &s.character.name)
                .unwrap();
            let id = u32::from_le_bytes(packet[3..7].try_into().unwrap());
            assert!(
                book.inner.lock().unwrap().bundles[&id]
                    .items
                    .contains(&(certificate, 1))
            );
            if matches!(prototype, 7005..=7008) {
                pickup_routed(&book, &lobby, "test", &mut s, peer, id, certificate).unwrap();
                assert!(
                    pickup_routed(&book, &lobby, "test", &mut s, peer, id, certificate).is_err()
                );
            }
        }
        assert_eq!(
            s.inventory
                .iter()
                .filter(|i| i.id == 910138000)
                .map(|i| i.count)
                .sum::<u16>(),
            3
        );
        assert!(
            s.inventory
                .iter()
                .any(|i| i.id == 910135000 && i.count == 1)
        );
        let mut helmet = None;
        for _ in 0..20000 {
            let packet = book
                .create(7005, 12000, s.position, &s.character.name)
                .unwrap();
            let id = u32::from_le_bytes(packet[3..7].try_into().unwrap());
            if book.inner.lock().unwrap().bundles[&id]
                .items
                .iter()
                .any(|(item, _)| *item == 245003541)
            {
                helmet = Some(id);
                break;
            }
        }
        let id =
            helmet.expect("fixed source RNG sequence must produce a rogue dagger-defense helmet");
        pickup_routed(&book, &lobby, "test", &mut s, peer, id, 245003541).unwrap();
        assert!(
            s.inventory
                .iter()
                .any(|i| i.id == 245003541 && i.count == 1)
        );
        assert!(pickup_routed(&book, &lobby, "test", &mut s, peer, id, 245003541).is_err());
        // A conflicting owner customization is rejected without partial certificate/group edits.
        db.execute(
            "UPDATE world_drop_rules SET item=379021000 WHERE drop_id=7005 AND slot=0",
            [],
        )
        .unwrap();
        db.execute(
            "DELETE FROM world_drop_rules WHERE drop_id=7008 AND slot=2",
            [],
        )
        .unwrap();
        let output = std::process::Command::new("python")
            .arg(root.join("scripts/Repair-1453SuppressionLoot.py"))
            .args(["--apply", "--db"])
            .arg(&path)
            .arg("--game-dir")
            .arg(reko_common::game_dir())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM world_drop_rules WHERE drop_id=7008 AND slot=2",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT item FROM world_drop_rules WHERE drop_id=7005 AND slot=0",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            379021000
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn caitharos_key_belongs_to_the_clan_chief_without_party_rotation() {
        let Some((lobby, mut chief)) = crate::world_service_tests::setup() else {
            return;
        };
        let mut member = crate::social::tests::player(&lobby, "TrialMate", 1, 80);
        {
            let db = lobby.db.lock().unwrap();
            db.execute_batch(
                "INSERT INTO clans VALUES (1,'KeyClan',1);
                INSERT INTO clan_members VALUES ('test',0,1,1),('TrialMate',0,1,4);
                INSERT INTO world_monster_loot VALUES (2690,0,2690);
                INSERT INTO world_drop_rules VALUES (2690,2,910045000,10000);",
            )
            .unwrap();
        }
        chief.position.zone = 54;
        member.position = chief.position;
        crate::social::synchronize(&lobby, "test", &mut chief);
        crate::social::synchronize(&lobby, "TrialMate", &mut member);
        lobby
            .social
            .lock()
            .unwrap()
            .parties
            .insert(member.player_id, vec![member.player_id, chief.player_id]);
        let mut book =
            Book::load(&lobby.db.lock().unwrap(), lobby.combat.as_ref().unwrap()).unwrap();
        book.social = Some(lobby.social.clone());
        let p = book
            .create(2690, 12000, chief.position, &member.character.name)
            .unwrap();
        let id = u32::from_le_bytes(p[3..7].try_into().unwrap());
        assert!(book.visible(id, &chief.character.name));
        assert!(!book.visible(id, &member.character.name));
        let peer = "127.0.0.1:1".parse().unwrap();
        assert!(
            pickup_routed(&book, &lobby, "TrialMate", &mut member, peer, id, 910045000).is_err()
        );
        pickup_routed(&book, &lobby, "test", &mut chief, peer, id, 910045000).unwrap();
        assert_eq!(chief.inventory[0].id, 910045000);
        assert!(pickup_routed(&book, &lobby, "test", &mut chief, peer, id, 910045000).is_err());
    }
    #[test]
    fn party_loot_is_owned_atomic_level_weighted_and_rotates_items_without_touching_solo_rules() {
        let Some((lobby, mut a)) = crate::world_service_tests::setup() else {
            return;
        };
        crate::social::synchronize(&lobby, "test", &mut a);
        let mut b = crate::social::tests::player(&lobby, "LootMate", 1, 80);
        let mut outsider = crate::social::tests::player(&lobby, "LootOutsider", 1, 80);
        lobby
            .social
            .lock()
            .unwrap()
            .parties
            .insert(a.player_id, vec![a.player_id, b.player_id]);
        {
            let db = lobby.db.lock().unwrap();
            db.execute("INSERT INTO world_monster_loot VALUES (100,1000,1)", [])
                .unwrap();
            db.execute(
                "INSERT INTO world_drop_rules VALUES(1,2,389014000,10000)",
                [],
            )
            .unwrap();
        }
        let mut book =
            Book::load(&lobby.db.lock().unwrap(), lobby.combat.as_ref().unwrap()).unwrap();
        book.social = Some(lobby.social.clone());
        let packet = book
            .create(100, 12000, a.position, &a.character.name)
            .unwrap();
        let id = u32::from_le_bytes(packet[3..7].try_into().unwrap());
        assert!(book.visible(id, &b.character.name));
        assert!(!book.visible(id, &outsider.character.name));
        let count = book.inner.lock().unwrap().bundles[&id].items[0].1;
        let peer = "127.0.0.1:1".parse().unwrap();
        assert!(
            pickup_routed(&book, &lobby, "LootOutsider", &mut outsider, peer, id, GOLD).is_err()
        );
        lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_party_coin BEFORE UPDATE ON character_wallets WHEN NEW.account='LootMate' BEGIN SELECT RAISE(ABORT,'test');END;").unwrap();
        assert!(pickup_routed(&book, &lobby, "test", &mut a, peer, id, GOLD).is_err());
        assert_eq!(
            starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            1000000
        );
        assert_eq!(book.inner.lock().unwrap().bundles[&id].items[0].1, count);
        lobby
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_party_coin;")
            .unwrap();
        pickup_routed(&book, &lobby, "test", &mut a, peer, id, GOLD).unwrap();
        crate::social::synchronize(&lobby, "LootMate", &mut b);
        assert_eq!(a.coins, 1000000 + u32::from(count) / 2);
        assert_eq!(b.coins, a.coins);
        pickup_routed(&book, &lobby, "test", &mut a, peer, id, 389014000).unwrap();
        assert_eq!(a.inventory[0].id, 389014000);
        let packet = book
            .create(100, 12000, a.position, &a.character.name)
            .unwrap();
        let next = u32::from_le_bytes(packet[3..7].try_into().unwrap());
        let (reply, _) =
            pickup_routed(&book, &lobby, "test", &mut a, peer, next, 389014000).unwrap();
        assert_eq!(reply, [0x26, 4]);
        crate::social::synchronize(&lobby, "LootMate", &mut b);
        assert_eq!(b.inventory[0].id, 389014000);
        assert_eq!(a.inventory[0].count, 1);
        assert!(pickup_routed(&book, &lobby, "LootMate", &mut b, peer, next, 389014000).is_err());
    }

    #[test]
    fn permanent_costume_noah_bonus_is_pickup_only_and_never_replayed() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        s.costumes = [
            starter::Item {
                id: 508013321,
                durability: 3000,
                count: 1,
            },
            starter::Item {
                id: 508011444,
                durability: 3000,
                count: 1,
            },
        ];
        lobby
            .db
            .lock()
            .unwrap()
            .execute("INSERT INTO world_monster_loot VALUES(100,1000,0)", [])
            .unwrap();
        let book = Book::load(&lobby.db.lock().unwrap(), lobby.combat.as_ref().unwrap()).unwrap();
        let notification = book
            .create(100, 12000, s.position, &s.character.name)
            .unwrap();
        let id = u32::from_le_bytes(notification[3..7].try_into().unwrap());
        let base = book.inner.lock().unwrap().bundles[&id].items[0].1;
        let expected = u32::from(base) * 109 / 100;
        let peer = "127.0.0.1:1".parse().unwrap();
        let (reply, _) = pickup(&book, &lobby, "test", &mut s, peer, id, GOLD).unwrap();
        assert_eq!(s.coins, 1000000 + expected);
        assert_eq!(
            u16::from_le_bytes(reply[7..9].try_into().unwrap()) as u32,
            expected
        );
        assert_eq!(
            starter::coins(&lobby.db.lock().unwrap(), "test", 0).unwrap(),
            s.coins
        );
        assert!(pickup(&book, &lobby, "test", &mut s, peer, id, GOLD).is_err());
        assert_eq!(s.coins, 1000000 + expected);
    }
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
