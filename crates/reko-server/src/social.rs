//! Shared online players and reliable per-player social notifications.
use crate::{clans, lobby::Lobby, movement::Position, party, world_entry::Session};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
};
use tokio::net::TcpStream;

#[derive(Clone)]
pub(crate) struct Player {
    pub account: String,
    pub s: Session,
}
#[derive(Default)]
pub(crate) struct Hub {
    pub players: HashMap<u16, Player>,
    pub parties: HashMap<u16, Vec<u16>>,
    pub party_invites: HashMap<u16, (u16, u64)>,
    pub clan_invites: HashMap<u16, (u16, u16, u64)>,
    queues: HashMap<u16, Vec<Vec<u8>>>,
    pub refresh: HashSet<u16>,
    pub transfers: HashMap<u16, Position>,
    pub loot_rotation: HashMap<u16, usize>,
    visible: HashMap<u16, HashSet<u16>>,
}
pub(crate) fn near(a: Position, b: Position, meters: u16) -> bool {
    let dx = i64::from(a.x) - i64::from(b.x);
    let dz = i64::from(a.z) - i64::from(b.z);
    a.zone == b.zone && dx * dx + dz * dz <= i64::from(meters * 10).pow(2)
}
pub(crate) fn string(p: &mut Vec<u8>, value: &str) {
    p.extend((value.len() as u16).to_le_bytes());
    p.extend(value.as_bytes());
}
pub(crate) fn name(p: &[u8]) -> Option<&str> {
    let size = u16::from_le_bytes(p.get(..2)?.try_into().ok()?) as usize;
    if size == 0 || size > 20 || p.len() != size + 2 {
        return None;
    }
    let text = std::str::from_utf8(&p[2..]).ok()?;
    text.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        .then_some(text)
}
impl Hub {
    pub fn send(&mut self, id: u16, p: Vec<u8>) {
        if self.players.contains_key(&id) {
            self.queues.entry(id).or_default().push(p);
        }
    }
    pub fn send_many(&mut self, ids: &[u16], p: &[u8]) {
        for &id in ids {
            self.send(id, p.to_vec());
        }
    }
    pub fn party(&self, id: u16) -> Option<u16> {
        self.parties
            .iter()
            .find(|(_, v)| v.contains(&id))
            .map(|(&k, _)| k)
    }
    pub fn named(&self, name: &str) -> Option<u16> {
        self.players
            .iter()
            .find(|(_, p)| p.s.character.name.eq_ignore_ascii_case(name))
            .map(|(&id, _)| id)
    }
    pub fn group(&self, id: u16) -> Vec<u16> {
        self.party(id)
            .map(|key| self.parties[&key].clone())
            .unwrap_or_else(|| vec![id])
    }
    pub fn disconnect(&mut self, id: u16) {
        self.transfers.remove(&id);
        party::leave(self, id, id);
        self.party_invites
            .retain(|target, (leader, _)| *target != id && *leader != id);
        self.clan_invites
            .retain(|target, (leader, _, _)| *target != id && *leader != id);
        self.players.remove(&id);
        self.queues.remove(&id);
        self.visible.remove(&id);
        let observers: Vec<_> = self
            .visible
            .iter_mut()
            .filter_map(|(&viewer, seen)| seen.remove(&id).then_some(viewer))
            .collect();
        for viewer in observers {
            let mut p = vec![7, 2];
            p.extend(id.to_le_bytes());
            self.send(viewer, p);
        }
    }
    pub fn synchronize(&mut self, account: &str, s: &Session) {
        let id = s.player_id;
        let prior = self.players.insert(
            id,
            Player {
                account: account.into(),
                s: s.clone(),
            },
        );
        let ids: Vec<_> = self.players.keys().copied().collect();
        for viewer in ids {
            let now: HashSet<_> = self
                .players
                .iter()
                .filter(|(other, p)| {
                    **other != viewer
                        && !self.players[&viewer].s.zone_loading
                        && !p.s.zone_loading
                        && crate::population::same_region_view(
                            self.players[&viewer].s.position,
                            p.s.position,
                        )
                })
                .map(|(&id, _)| id)
                .collect();
            let old = self.visible.insert(viewer, now.clone()).unwrap_or_default();
            for other in old.difference(&now) {
                let mut p = vec![7, 2];
                p.extend(other.to_le_bytes());
                self.send(viewer, p);
            }
            for other in now.difference(&old) {
                let p = self.user_in(*other);
                self.send(viewer, p);
            }
            if viewer != id
                && now.contains(&id)
                && old.contains(&id)
                && let Some(prior) = &prior
            {
                let appearance = prior.s.equipment != s.equipment
                    || prior.s.clan != s.clan
                    || prior.s.character.class != s.character.class
                    || prior.s.sitting != s.sitting
                    || (prior.s.skills.hp == 0) != (s.skills.hp == 0);
                if appearance {
                    let mut p = vec![7, 2];
                    p.extend(id.to_le_bytes());
                    self.send(viewer, p);
                    let p = self.user_in(id);
                    self.send(viewer, p);
                } else if prior.s.position != s.position {
                    let mut p = vec![6];
                    p.extend(id.to_le_bytes());
                    p.extend(s.position.x.to_le_bytes());
                    p.extend(s.position.z.to_le_bytes());
                    p.extend(s.position.y.to_le_bytes());
                    p.extend(45u16.to_le_bytes());
                    p.push(3);
                    self.send(viewer, p);
                }
            }
        }
        if let Some(prior) = prior {
            let members = self.group(id);
            if members.len() > 1 {
                if (
                    prior.s.skills.hp,
                    prior.s.skills.max_hp,
                    prior.s.skills.mana,
                    prior.s.skills.max_mana,
                ) != (
                    s.skills.hp,
                    s.skills.max_hp,
                    s.skills.mana,
                    s.skills.max_mana,
                ) {
                    let p = party::health(s);
                    self.send_many(&members, &p);
                }
                if prior.s.progress.level != s.progress.level {
                    let mut p = vec![0x2f, 7];
                    p.extend(id.to_le_bytes());
                    p.push(s.progress.level);
                    self.send_many(&members, &p);
                }
                if prior.s.character.class != s.character.class {
                    let mut p = vec![0x2f, 8];
                    p.extend(id.to_le_bytes());
                    p.extend(s.character.class.to_le_bytes());
                    self.send_many(&members, &p);
                }
            }
        }
    }
    pub fn user_in(&self, id: u16) -> Vec<u8> {
        let s = &self.players[&id].s;
        let mut p = vec![7, 1];
        p.extend(id.to_le_bytes());
        p.push(s.character.name.len() as u8);
        p.extend(s.character.name.as_bytes());
        p.push(s.character.nation());
        p.extend(s.clan.id.to_le_bytes());
        p.push(s.clan.fame);
        p.extend(0u16.to_le_bytes());
        p.push(s.clan.name.len() as u8);
        p.extend(s.clan.name.as_bytes());
        p.extend([s.clan.grade, 0]);
        p.extend(0u16.to_le_bytes());
        p.extend(s.clan.cape.to_le_bytes());
        p.extend([s.progress.level, s.character.race]);
        p.extend(s.character.class.to_le_bytes());
        p.extend(s.position.x.to_le_bytes());
        p.extend(s.position.z.to_le_bytes());
        p.extend(s.position.y.to_le_bytes());
        p.extend([
            s.character.face,
            s.character.hair,
            if s.skills.hp == 0 {
                3
            } else if s.sitting {
                2
            } else {
                1
            },
        ]);
        p.extend(1u32.to_le_bytes());
        p.extend([
            1,
            u8::from(!s.is_admin),
            u8::from(self.party(id) == Some(id)),
            0,
            0,
        ]);
        p.extend(0u16.to_le_bytes());
        p.extend([0; 4]);
        for slot in [4, 10, 1, 7, 12, 5, 6, 8] {
            let item = s.equipment[slot];
            p.extend(item.id.to_le_bytes());
            p.extend(item.durability.to_le_bytes());
            p.push(0);
        }
        p
    }
    pub fn chat(&mut self, id: u16, channel: u8, text: &[u8]) -> bool {
        let s = &self.players[&id].s;
        let members = if channel == 3 {
            let Some(key) = self.party(id) else {
                return false;
            };
            self.parties[&key].clone()
        } else {
            if s.clan.id == 0 {
                return false;
            }
            self.players
                .iter()
                .filter(|(_, p)| p.s.clan.id == s.clan.id)
                .map(|(&id, _)| id)
                .collect()
        };
        let p = crate::chat::packet(channel, s.character.nation(), id, &s.character.name, text);
        self.send_many(&members, &p);
        true
    }
}
pub(crate) fn synchronize(lobby: &Lobby, account: &str, s: &mut Session) {
    if s.entered {
        if lobby.social.lock().unwrap().refresh.remove(&s.player_id) {
            let db = lobby.db.lock().unwrap();
            match (
                crate::starter::coins(&db, account, s.character.slot),
                crate::starter::inventory(&db, account, s.character.slot),
            ) {
                (Ok(coins), Ok(bag)) => {
                    s.coins = coins;
                    s.inventory = bag;
                    if let Some(c) = &lobby.combat {
                        crate::durability::refresh(s, c, lobby.skills.as_ref());
                    }
                }
                _ => {
                    tracing::error!(character=%s.character.name,"party loot refresh failed");
                    lobby.social.lock().unwrap().refresh.insert(s.player_id);
                }
            }
        }
        match clans::load(&lobby.db.lock().unwrap(), account, s.character.slot) {
            Ok(info) => {
                if info.id != 0 && info.id == s.clan.id && info.grade != s.clan.grade {
                    let mut p = vec![0x3c, 0x12, 1, 0];
                    p.extend(info.id.to_le_bytes());
                    p.extend([info.grade, 0]);
                    lobby.social.lock().unwrap().send(s.player_id, p);
                }
                s.clan = info;
            }
            Err(error) => tracing::error!(%error,"clan refresh failed"),
        }
        lobby.social.lock().unwrap().synchronize(account, s);
    }
}
pub(crate) async fn flush(sock: &mut TcpStream, peer: SocketAddr, lobby: &Lobby, s: &Session) {
    if !s.entered || s.zone_loading {
        return;
    }
    let packets = lobby
        .social
        .lock()
        .unwrap()
        .queues
        .remove(&s.player_id)
        .unwrap_or_default();
    for p in packets {
        crate::send_response(sock, peer, &None, &p, "SOCIAL_UPDATE").await;
    }
}
pub(crate) struct Guard {
    pub lobby: std::sync::Arc<Lobby>,
    pub id: Option<u16>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.lobby.social.lock().unwrap().disconnect(id);
        }
    }
}

impl Hub {
    pub fn death(
        &self,
        prototype: u16,
        level: u8,
        position: Position,
        damage: &HashMap<String, u32>,
    ) -> (
        Vec<(String, crate::monsters::ExperienceCredit)>,
        Vec<String>,
    ) {
        let total_damage = damage.values().map(|&n| u64::from(n)).sum();
        let mut credits = Vec::new();
        let mut groups: HashMap<u16, u64> = HashMap::new();
        for (owner, &amount) in damage {
            if let Some(leader) = self
                .named(owner)
                .and_then(|id| self.party(id))
                .filter(|key| self.parties[key].len() > 1)
            {
                *groups.entry(leader).or_default() += u64::from(amount);
            } else if amount > 0 {
                credits.push((
                    owner.clone(),
                    crate::monsters::ExperienceCredit {
                        party: None,
                        prototype,
                        level,
                        position,
                        damage: amount,
                        total_damage,
                    },
                ));
            }
        }
        for (leader, amount) in groups {
            let party = &self.parties[&leader];
            let members = party.len() as u32;
            let total_level = party
                .iter()
                .map(|id| u32::from(self.players[id].s.progress.level))
                .sum();
            for id in party {
                let s = &self.players[id].s;
                if s.skills.hp > 0 && !s.zone_loading && near(s.position, position, 50) {
                    credits.push((
                        s.character.name.clone(),
                        crate::monsters::ExperienceCredit {
                            party: Some((members, total_level)),
                            prototype,
                            level,
                            position,
                            damage: amount.min(u64::from(u32::MAX)) as u32,
                            total_damage,
                        },
                    ));
                }
            }
        }
        let kills = damage
            .iter()
            .max_by_key(|(_, n)| *n)
            .map_or_else(Vec::new, |(owner, _)| {
                if let Some(id) = self.named(owner).filter(|id| self.party(*id).is_some()) {
                    self.group(id)
                        .into_iter()
                        .filter_map(|id| {
                            let s = &self.players[&id].s;
                            (s.skills.hp > 0 && !s.zone_loading && near(s.position, position, 50))
                                .then(|| s.character.name.clone())
                        })
                        .collect()
                } else {
                    vec![owner.clone()]
                }
            });
        (credits, kills)
    }
}

#[cfg(test)]
#[path = "social_tests.rs"]
pub(crate) mod tests;
