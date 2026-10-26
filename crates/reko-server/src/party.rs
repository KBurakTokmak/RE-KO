//! Original eight-member parties, invitation consent and native roster packets.
use crate::{
    social::{Hub, name, string},
    world_entry::Session,
};

pub(crate) fn health(s: &Session) -> Vec<u8> {
    let mut p = vec![0x2f, 6];
    p.extend(s.player_id.to_le_bytes());
    for n in [
        s.skills.max_hp,
        s.skills.hp,
        s.skills.max_mana,
        s.skills.mana,
    ] {
        p.extend(n.to_le_bytes());
    }
    p
}
fn member(s: &Session) -> Vec<u8> {
    let mut p = vec![0x2f, 3];
    p.extend(s.player_id.to_le_bytes());
    p.push(1);
    string(&mut p, &s.character.name);
    p.extend(s.skills.max_hp.to_le_bytes());
    p.extend(s.skills.hp.to_le_bytes());
    p.push(s.progress.level);
    p.extend(s.character.class.to_le_bytes());
    p.extend(s.skills.max_mana.to_le_bytes());
    p.extend(s.skills.mana.to_le_bytes());
    p.push(s.character.nation());
    p
}
fn fail(h: &mut Hub, id: u16, code: i16) {
    let mut p = vec![0x2f, 3];
    p.extend(code.to_le_bytes());
    h.send(id, p);
}
fn leader_state(h: &mut Hub, id: u16, on: bool) {
    let mut p = vec![0x29];
    p.extend(id.to_le_bytes());
    p.push(6);
    p.extend(u32::from(on).to_le_bytes());
    let ids = h
        .players
        .iter()
        .filter(|(_, p)| {
            crate::population::same_region_view(h.players[&id].s.position, p.s.position)
        })
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    h.send_many(&ids, &p);
}
pub(crate) fn leave(h: &mut Hub, actor: u16, target: u16) {
    let Some(leader) = h.party(actor) else { return };
    if actor != target && actor != leader {
        return;
    }
    if !h.parties[&leader].contains(&target) {
        return;
    }
    let members = h.parties[&leader].clone();
    if target == leader || members.len() <= 2 {
        h.send_many(&members, &[0x2f, 5]);
        h.parties.remove(&leader);
        h.loot_rotation.remove(&leader);
        h.party_invites.retain(|_, (owner, _)| *owner != leader);
        leader_state(h, leader, false);
    } else {
        let cursor = *h.loot_rotation.get(&leader).unwrap_or(&0) % members.len();
        let next = (0..members.len())
            .map(|n| members[(cursor + n) % members.len()])
            .find(|id| *id != target);
        let mut p = vec![0x2f, 4];
        p.extend(target.to_le_bytes());
        h.send_many(&members, &p);
        h.parties
            .get_mut(&leader)
            .unwrap()
            .retain(|id| *id != target);
        if let Some(next) = next {
            h.loot_rotation.insert(
                leader,
                h.parties[&leader]
                    .iter()
                    .position(|id| *id == next)
                    .unwrap_or(0),
            );
        }
    }
}

pub(crate) fn zone_leave(h: &mut Hub, id: u16) {
    // Zone departure is not a socket disconnect. The source promotes the next
    // member before removing a travelling leader when a party can remain.
    if h.party(id) == Some(id)
        && h.parties[&id].len() > 2
        && let Some(next) = h.parties[&id]
            .iter()
            .copied()
            .find(|other| *other != id && h.players.contains_key(other))
    {
        let mut promote = vec![0x2f, 0x1c];
        promote.extend(next.to_le_bytes());
        process(h, id, &promote);
    }
    leave(h, id, id);
    h.party_invites
        .retain(|target, (owner, _)| *target != id && *owner != id);
    h.transfers.remove(&id);
}
pub(crate) fn process(h: &mut Hub, id: u16, p: &[u8]) {
    let now = crate::skills::now_ms();
    let expired = h
        .party_invites
        .iter()
        .filter(|(_, (_, until))| now >= *until)
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    for target in expired {
        if let Some((leader, _)) = h.party_invites.remove(&target) {
            fail(h, leader, -1);
            if h.parties.get(&leader).is_some_and(|m| m.len() == 1) {
                leave(h, leader, leader);
            }
        }
    }
    let Some((&sub, args)) = p.get(1..).and_then(|v| v.split_first()) else {
        return;
    };
    match sub {
        1 | 3 => {
            let Some(target) = name(args).and_then(|name| h.named(name)) else {
                fail(h, id, -1);
                return;
            };
            if target == id || h.party(target).is_some() || h.party_invites.contains_key(&target) {
                fail(h, id, -1);
                return;
            }
            let actor = &h.players[&id].s;
            let other = &h.players[&target].s;
            if actor.skills.hp == 0
                || other.skills.hp == 0
                || actor.zone_loading
                || other.zone_loading
                || actor.position.zone != other.position.zone
                || (actor.character.nation() != other.character.nation()
                    && !matches!(actor.position.zone, 21 | 55))
            {
                fail(h, id, -3);
                return;
            }
            let a = u32::from(actor.progress.level);
            let b = u32::from(other.progress.level);
            if !(actor.clan.id != 0 && actor.clan.id == other.clan.id
                || (b <= a * 3 / 2 && b >= a * 2 / 3)
                || a.abs_diff(b) <= 8)
            {
                fail(h, id, -2);
                return;
            }
            if (sub == 1 && h.party(id).is_some()) || (sub == 3 && h.party(id) != Some(id)) {
                fail(h, id, -1);
                return;
            }
            if h.parties.get(&id).is_some_and(|v| v.len() >= 8) {
                fail(h, id, -1);
                return;
            }
            let actor_name = actor.character.name.clone();
            if !h.parties.contains_key(&id) {
                h.loot_rotation.remove(&id);
            }
            h.parties.entry(id).or_insert_with(|| vec![id]);
            h.party_invites.insert(target, (id, now + 60000));
            let mut reply = vec![0x2f, 2];
            reply.extend(id.to_le_bytes());
            string(&mut reply, &actor_name);
            h.send(target, reply);
            leader_state(h, id, true);
        }
        2 if args.len() == 1 => {
            let Some((leader, _)) = h.party_invites.remove(&id) else {
                return;
            };
            if args[0] != 1
                || h.players[&id].s.skills.hp == 0
                || h.players[&id].s.zone_loading
                || h.players
                    .get(&leader)
                    .is_none_or(|p| p.s.skills.hp == 0 || p.s.zone_loading)
                || !h.parties.contains_key(&leader)
                || h.party(id).is_some()
                || h.parties[&leader].len() >= 8
                || h.players[&leader].s.position.zone != h.players[&id].s.position.zone
            {
                fail(h, leader, -1);
                if h.parties.get(&leader).is_some_and(|m| m.len() == 1) {
                    leave(h, leader, leader);
                }
                return;
            }
            h.parties.get_mut(&leader).unwrap().push(id);
            let members = h.parties[&leader].clone();
            for &existing in &members {
                if existing != id {
                    let packet = member(&h.players[&existing].s);
                    h.send(id, packet);
                }
            }
            let packet = member(&h.players[&id].s);
            h.send_many(&members, &packet);
        }
        4 if args.len() == 2 => leave(h, id, u16::from_le_bytes(args.try_into().unwrap())),
        5 if args.is_empty() => {
            if h.party(id) == Some(id) {
                leave(h, id, id);
            }
        }
        0x1c if args.len() == 2 => {
            let next = u16::from_le_bytes(args.try_into().unwrap());
            if h.party(id) != Some(id) || next == id || !h.parties[&id].contains(&next) {
                return;
            }
            let mut members = h.parties.remove(&id).unwrap();
            let cursor = h.loot_rotation.remove(&id).unwrap_or(0) % members.len();
            let recipient = members[cursor];
            let index = members.iter().position(|v| *v == next).unwrap();
            members.swap(0, index);
            h.loot_rotation.insert(
                next,
                members.iter().position(|id| *id == recipient).unwrap(),
            );
            h.send_many(&members, &[0x2f, 5]);
            h.parties.insert(next, members.clone());
            h.party_invites.retain(|_, (leader, _)| *leader != id);
            for &entry in &members {
                let packet = member(&h.players[&entry].s);
                h.send_many(&members, &packet);
            }
            leader_state(h, id, false);
            leader_state(h, next, true);
        }
        _ => fail(h, id, -1),
    }
}
