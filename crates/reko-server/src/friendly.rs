//! Original moral-based friendly targets with one atomic caster payment.
use crate::{combat, lobby::Lobby, resources, skills, social, utility, world_entry::Session};
use std::net::SocketAddr;
use tokio::net::TcpStream;

type Applied = (resources::Paid, Vec<(u16, Vec<u8>)>);

fn supported(lobby: &Lobby, spell: &skills::Spell) -> bool {
    let Some(catalog) = &lobby.skills else {
        return false;
    };
    let Some(row) = catalog.effects.get(&(spell.types[0], spell.id)) else {
        return false;
    };
    match spell.types {
        [3, 0] => {
            matches!(combat::integer(row, 2), 1 | 2 | 14)
                && combat::integer(row, 3) >= 0
                && (combat::integer(row, 3) > 0 || combat::integer(row, 4) > 0)
        }
        [4, 0] => {
            matches!(
                combat::integer(row, 1),
                1 | 2 | 4 | 5 | 7 | 8 | 10 | 18 | 19 | 23 | 28 | 29 | 31
            ) || combat::integer(row, 1) == 6 && catalog.speeds.contains_key(&spell.id)
        }
        [5, 0] => matches!(combat::integer(row, 1), 1 | 2),
        _ => false,
    }
}

fn apply(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    spell: &skills::Spell,
    target: u16,
) -> Result<Applied, &'static str> {
    let catalog = lobby.skills.as_ref().ok_or("Skill data unavailable.")?;
    let row = &catalog.effects[&(spell.types[0], spell.id)];
    let group_cast = spell.moral == 6;
    if group_cast && ![s.player_id, u16::MAX].contains(&target) {
        return Err("Party-wide skills must target your party.");
    }
    let radius = if group_cast {
        combat::integer(row, if spell.types[0] == 4 { 2 } else { 1 }).clamp(0, 65535) as u16
    } else {
        spell.range
    };
    let db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let members = hub.group(s.player_id);
    let mut players: Vec<_> = hub
        .players
        .values()
        .filter(|p| {
            let id = p.s.player_id;
            let state = if id == s.player_id { s } else { &p.s };
            state.entered
                && !state.zone_loading
                && state.skills.hp > 0
                && (id == s.player_id
                    || group_cast
                    || spell.moral == 4 && members.contains(&id)
                    || hub.can_see(s.player_id, id))
                && (id == s.player_id
                    || radius > 0 && social::near(s.position, state.position, radius))
                && if group_cast {
                    members.contains(&id)
                } else {
                    id == target
                        && match spell.moral {
                            2 => friendly(s, state),
                            3 => id != s.player_id && friendly(s, state),
                            4 => members.contains(&id),
                            _ => false,
                        }
                }
        })
        .cloned()
        .collect();
    for p in &mut players {
        if p.s.player_id == s.player_id {
            p.s = s.clone();
            p.account = account.into();
        }
    }
    if !players.iter().any(|p| p.s.player_id == s.player_id)
        && (group_cast || target == s.player_id && spell.moral != 3)
    {
        players.push(social::Player {
            account: account.into(),
            s: s.clone(),
        });
    }
    players.sort_unstable_by_key(|p| p.s.player_id);
    if players.is_empty() {
        return Err("Select an eligible living friendly player within range.");
    }
    let tx = db
        .unchecked_transaction()
        .map_err(|_| "Unable to start friendly skill transaction.")?;
    let mut paid = resources::commit_in(
        &tx,
        account,
        s.character.slot,
        spell,
        1,
        lobby.combat.as_ref(),
        |_| Ok(()),
    )?;
    let mut changes = Vec::new();
    for player in players {
        let mut state = skills::load(&tx, &player.account, player.s.character.slot)
            .map_err(|_| "Unable to load friendly target.")?;
        if state.hp == 0 {
            if group_cast {
                continue;
            }
            return Err("The target is dead.");
        }
        let removed = match utility::apply_friendly(lobby, &player.s, spell, &mut state) {
            Ok(removed) => removed,
            Err(_) if group_cast => continue,
            Err(reason) => return Err(reason),
        };
        skills::save_in(&tx, &player.account, player.s.character.slot, &state)
            .map_err(|_| "Unable to save friendly target.")?;
        if player.s.player_id == s.player_id {
            paid.state = state.clone();
        }
        changes.push((player, state, removed));
    }
    if changes.is_empty() {
        return Err("No party member can receive this effect; resources were not consumed.");
    }
    tx.commit()
        .map_err(|_| "Unable to commit friendly skill.")?;
    let mut effects = Vec::new();
    for (mut player, state, removed) in changes {
        let id = player.s.player_id;
        player.s.skills = state.clone();
        utility::refresh(&mut player.s, Some(catalog), lobby.combat.as_ref());
        let mut effect = skills::effect(spell.id, s.player_id);
        effect[8..10].copy_from_slice(&id.to_le_bytes());
        effect[12..14].copy_from_slice(&1u16.to_le_bytes());
        if spell.types[0] == 4 {
            effect[16..18]
                .copy_from_slice(&(combat::integer(row, 3).clamp(0, 32767) as u16).to_le_bytes());
            effect[20..22]
                .copy_from_slice(&(combat::integer(row, 5).clamp(0, 200) as u16).to_le_bytes());
        }
        if id != s.player_id {
            hub.synchronize(&player.account, &player.s);
            hub.skill_refresh.insert(id);
            hub.send(id, state.hp_packet(0));
            hub.send(id, state.mana_packet());
            hub.send(id, crate::inventory::attributes(&player.s.combat, 2));
            for category in removed {
                hub.send(id, vec![0x31, 5, category]);
            }
            if spell.types[0] == 5 && combat::integer(row, 1) == 1 {
                hub.send(id, vec![0x31, 7, 200]);
            }
            hub.send(id, effect.clone());
        } else {
            for category in removed {
                effects.push((id, vec![0x31, 5, category]));
            }
            if spell.types[0] == 5 && combat::integer(row, 1) == 1 {
                effects.push((id, vec![0x31, 7, 200]));
            }
        }
        effects.push((id, effect));
    }
    Ok((paid, effects))
}

fn friendly(caster: &Session, target: &Session) -> bool {
    caster.player_id == target.player_id
        || caster.position.zone != 48
            && (!matches!(caster.position.zone, 71 | 72)
                || caster.character.nation() == target.character.nation())
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    spell: &skills::Spell,
    target: u16,
) -> Option<Result<(), &'static str>> {
    if spell.skill_group == 0 || !matches!(spell.moral, 2 | 3 | 4 | 6) || !supported(lobby, spell) {
        return None;
    }
    Some(match apply(lobby, account, session, spell, target) {
        Err(reason) => Err(reason),
        Ok((paid, effects)) => {
            resources::sync(sock, peer, lobby, account, session, paid).await;
            super::send_response(
                sock,
                peer,
                &None,
                &crate::inventory::attributes(&session.combat, 2),
                "FRIENDLY_ATTRIBUTES",
            )
            .await;
            for (_, packet) in effects {
                super::send_response(sock, peer, &None, &packet, "FRIENDLY_EFFECT").await;
            }
            Ok(())
        }
    })
}

#[cfg(test)]
#[path = "friendly_tests.rs"]
mod tests;
