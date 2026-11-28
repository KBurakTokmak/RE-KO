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
                1 | 2 | 4 | 5 | 7 | 8 | 10 | 18 | 19 | 23 | 27 | 28 | 29 | 30 | 31
            ) || combat::integer(row, 1) == 6 && catalog.speeds.contains_key(&spell.id)
        }
        [5, 0] => matches!(combat::integer(row, 1), 1..=3),
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
    let area_friend = spell.moral == 11;
    let resurrection = spell.moral == 25 && spell.types == [5, 0] && combat::integer(row, 1) == 3;
    if group_cast && ![s.player_id, u16::MAX].contains(&target) {
        return Err("Party-wide skills must target your party.");
    }
    if spell.hp >= 10_000 && target == s.player_id {
        return Err("Sacrifice must target another party member.");
    }
    let radius = if area_friend {
        combat::integer(row, 2).clamp(0, 65535) as u16
    } else if group_cast {
        if spell.types[0] == 5 {
            spell.range
        } else {
            combat::integer(row, if spell.types[0] == 4 { 2 } else { 1 }).clamp(0, 65535) as u16
        }
    } else {
        spell.range
    };
    let db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    if resurrection {
        let Some(mut dead_player) = hub.players.get(&target).cloned() else {
            return Err("Select a dead friendly player within range.");
        };
        if target == s.player_id
            || !dead_player.s.entered
            || dead_player.s.zone_loading
            || dead_player.s.skills.hp != 0
            || !friendly(&hub, s, &dead_player.s)
            || !social::near(s.position, dead_player.s.position, radius)
        {
            return Err("Select a dead friendly player within range.");
        }
        let need_stones = combat::integer(row, 3).max(0) as u16;
        let tx = db
            .unchecked_transaction()
            .map_err(|_| "Unable to start resurrection transaction.")?;
        let paid = resources::commit_in(
            &tx,
            account,
            s.character.slot,
            spell,
            1,
            lobby.combat.as_ref(),
            |_| Ok(()),
        )?;
        if need_stones > 0 {
            crate::inventory::consume(
                &tx,
                &dead_player.account,
                dead_player.s.character.slot,
                379_006_000,
                need_stones,
            )
            .map_err(|_| "The dead player does not have enough Stones of Life.")?;
        }
        let mut tstate = skills::load(&tx, &dead_player.account, dead_player.s.character.slot)
            .map_err(|_| "Unable to load dead target.")?;
        if tstate.hp != 0 {
            return Err("The target is already alive.");
        }
        crate::travel::clear_zone_effects(&mut tstate, Some(catalog));
        let tstats = utility::effective(
            &dead_player.s,
            &tstate,
            Some(catalog),
            lobby.combat.as_ref(),
        );
        tstate.set_limits(tstats.hp, tstats.mp);
        tstate.hp = tstate.max_hp;
        skills::save_in(
            &tx,
            &dead_player.account,
            dead_player.s.character.slot,
            &tstate,
        )
        .map_err(|_| "Unable to save resurrected target.")?;
        tx.commit().map_err(|_| "Unable to commit resurrection.")?;
        dead_player.s.skills = tstate.clone();
        dead_player.s.combat = tstats;
        hub.synchronize(&dead_player.account, &dead_player.s);
        hub.skill_refresh.insert(target);
        if need_stones > 0 {
            hub.refresh.insert(target);
        }
        let mut regene = vec![0x12];
        regene.extend(dead_player.s.position.x.to_le_bytes());
        regene.extend(dead_player.s.position.z.to_le_bytes());
        regene.extend(dead_player.s.position.y.to_le_bytes());
        hub.send(target, regene);
        hub.send(target, tstate.hp_packet(0));
        hub.send(target, crate::inventory::attributes(&tstats, 2));
        hub.send(target, tstate.mana_packet());
        for pkt in utility::buff_restore_packets(&dead_player.s, skills::now_ms()) {
            hub.send(target, pkt);
        }
        let mut effect = skills::effect(spell.id, s.player_id);
        effect[8..10].copy_from_slice(&target.to_le_bytes());
        effect[12..14].copy_from_slice(&1u16.to_le_bytes());
        hub.send(target, effect.clone());
        return Ok((paid, vec![(target, effect)]));
    }
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
                    || area_friend
                    || spell.moral == 4 && members.contains(&id)
                    || hub.can_see(s.player_id, id))
                && (id == s.player_id
                    || radius > 0 && social::near(s.position, state.position, radius))
                && if area_friend {
                    friendly(&hub, s, state)
                } else if group_cast {
                    members.contains(&id)
                } else {
                    id == target
                        && match spell.moral {
                            2 => friendly(&hub, s, state),
                            3 => id != s.player_id && friendly(&hub, s, state),
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
        && (group_cast || area_friend || target == s.player_id && spell.moral != 3)
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
            if group_cast || area_friend {
                continue;
            }
            return Err("The target is dead.");
        }
        let removed = match utility::apply_friendly(lobby, &player.s, spell, &mut state) {
            Ok(removed) => removed,
            Err(_) if group_cast || area_friend => continue,
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
    let now = skills::now_ms();
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
        if removed.contains(&32) {
            for slot in [6usize, 8usize] {
                let item = player.s.equipment[slot];
                if item.id != 0 {
                    let pkt =
                        crate::pvp::user_look_change(id, slot as u8, item.id, item.durability);
                    hub.broadcast_visual(id, None, Some(id), &pkt);
                    if id != s.player_id {
                        hub.send(id, pkt);
                    } else {
                        effects.push((id, pkt));
                    }
                }
            }
        }
        if id != s.player_id {
            hub.synchronize(&player.account, &player.s);
            hub.skill_refresh.insert(id);
            hub.send(id, state.hp_packet(0));
            hub.send(id, state.mana_packet());
            hub.send(id, crate::inventory::attributes(&player.s.combat, 2));
            let poison_cleared = removed.contains(&20) && !state.periodic_statuses()[1];
            for category in removed {
                hub.send(id, vec![0x31, 5, category]);
                if let Some(scroll) = state.buffs.get(&category).copied()
                    && now < scroll.until
                {
                    hub.send(id, utility::buff_restore_packet(id, scroll, now));
                }
            }
            if spell.types[0] == 5 && combat::integer(row, 1) == 1 {
                hub.send(id, skills::DAMAGE_DURATION_EXPIRED.to_vec());
                hub.send(id, vec![0x5e, 2, 1, 0]);
                hub.send(id, vec![0x5e, 2, 2, 0]);
            }
            if spell.types[0] == 5 && combat::integer(row, 1) == 2 {
                hub.send(id, vec![0x5e, 2, 3, 0]);
                hub.send(id, vec![0x5e, 2, 4, 0]);
                if poison_cleared {
                    hub.send(id, vec![0x5e, 2, 2, 0]);
                }
            }
            hub.send(id, effect.clone());
        } else {
            let poison_cleared = removed.contains(&20) && !state.periodic_statuses()[1];
            for category in removed {
                effects.push((id, vec![0x31, 5, category]));
                if let Some(scroll) = state.buffs.get(&category).copied()
                    && now < scroll.until
                {
                    effects.push((id, utility::buff_restore_packet(id, scroll, now)));
                }
            }
            if spell.types[0] == 5 && combat::integer(row, 1) == 1 {
                effects.push((id, skills::DAMAGE_DURATION_EXPIRED.to_vec()));
                effects.push((id, vec![0x5e, 2, 1, 0]));
                effects.push((id, vec![0x5e, 2, 2, 0]));
            }
            if spell.types[0] == 5 && combat::integer(row, 1) == 2 {
                effects.push((id, vec![0x5e, 2, 3, 0]));
                effects.push((id, vec![0x5e, 2, 4, 0]));
                if poison_cleared {
                    effects.push((id, vec![0x5e, 2, 2, 0]));
                }
            }
        }
        effects.push((id, effect));
    }
    Ok((paid, effects))
}

/// Friendly skills never help a player the caster may fight here.
fn friendly(hub: &social::Hub, caster: &Session, target: &Session) -> bool {
    if caster.player_id == target.player_id {
        return true;
    }
    let same_nation = caster.character.nation() == target.character.nation();
    let gates = &hub.events.gates;
    match caster.position.zone {
        48 => false,
        61..=69 | 71 | 72 => same_nation,
        zone @ (1 | 2) if gates.invasion == Some(zone) => same_nation,
        crate::events::DELOS if gates.siege_battle() => {
            caster.clan.id == 0 || target.clan.id == 0 || caster.clan.id == target.clan.id
        }
        _ => true,
    }
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
    if spell.skill_group == 0
        || !matches!(spell.moral, 2 | 3 | 4 | 6 | 11 | 25)
        || !supported(lobby, spell)
    {
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
            if session.skills.hp == 0 {
                let mut dead_pkt = vec![0x11];
                dead_pkt.extend(session.player_id.to_le_bytes());
                super::send_response(sock, peer, &None, &dead_pkt, "SACRIFICE_DEAD").await;
            }
            Ok(())
        }
    })
}

#[cfg(test)]
#[path = "friendly_tests.rs"]
mod tests;
