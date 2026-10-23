//! Original-table rogue self utility effects and effective attributes.

use crate::{combat, inventory, lobby::Lobby, resources, skills, world_entry::Session};
use std::net::SocketAddr;
use tokio::net::TcpStream;

pub(crate) fn refresh(
    session: &mut Session,
    catalog: Option<&skills::Catalog>,
    items: Option<&combat::Catalog>,
) {
    session.combat = effective(session, &session.skills, catalog, items);
}

pub(crate) fn effective(
    session: &Session,
    state: &skills::State,
    catalog: Option<&skills::Catalog>,
    items: Option<&combat::Catalog>,
) -> combat::Stats {
    let mut result = session.base_combat;
    let mut bonuses = [0; 5];
    if let Some(row) = state
        .buffs
        .get(&7)
        .filter(|b| skills::now_ms() < b.until)
        .and_then(|b| catalog.and_then(|c| c.effects.get(&(4, b.skill))))
    {
        for (value, col) in bonuses.iter_mut().zip(14..19) {
            *value = combat::integer(row, col).max(0) as u16;
        }
        if let Some(items) = items
            && let Ok(mut stats) = items.calculate_bonuses(
                &session.character,
                &session.progress,
                &session.equipment,
                bonuses,
            )
        {
            stats.weight = stats
                .weight
                .saturating_add(items.bag_weight(&session.inventory));
            result = stats;
        }
    }
    crate::costumes::apply(&mut result, &session.costumes);
    if session.character.class % 100 == 8
        && u32::from(state.hp) * 100 <= u32::from(state.max_hp) * 30
    {
        for value in &mut result.resistance {
            *value = value.saturating_add(50);
        }
    }
    if matches!(session.character.class % 100, 1 | 5 | 6) {
        let passives: Vec<_> = catalog
            .into_iter()
            .flat_map(|c| c.spells.values())
            .filter(|s| {
                s.skill_group == session.character.class * 10 + 6
                    && s.required_points <= u16::from(session.progress.points[6])
                    && s.types == [0, 0]
            })
            .collect();
        let mut ac = u32::from(
            passives
                .iter()
                .max_by_key(|s| {
                    if s.passive_defence > 0 {
                        s.required_points
                    } else {
                        0
                    }
                })
                .map_or(0, |s| s.passive_defence),
        );
        let mut resistance = passives
            .iter()
            .max_by_key(|s| {
                if s.passive_resistance > 0 {
                    s.required_points
                } else {
                    0
                }
            })
            .map_or(0, |s| s.passive_resistance);
        let shield = items
            .and_then(|c| c.item(session.equipment[8].id))
            .is_some_and(|(r, _)| combat::integer(r, 12) == 2);
        if !shield {
            ac /= 2;
            resistance /= 2;
        }
        result.defence = (u32::from(result.defence) * (100 + ac) / 100).min(32767) as u16;
        for value in &mut result.resistance {
            *value = value.saturating_add(resistance);
        }
    }
    if matches!(session.character.class % 100, 6 | 12)
        && u32::from(state.hp) * 100 < u32::from(state.max_hp) * 30
    {
        result.defence = (u32::from(result.defence) * 120 / 100).min(32767) as u16;
    }
    let base = result;
    if let Some(catalog) = catalog {
        for (category, buff) in state
            .buffs
            .iter()
            .filter(|(_, buff)| skills::now_ms() < buff.until)
        {
            let Some(row) = catalog.effects.get(&(4, buff.skill)) else {
                continue;
            };
            match category {
                1 => {
                    result.hp = if combat::integer(row, 10) != 0 {
                        (i32::from(base.hp) + combat::integer(row, 10)).clamp(1, 14000) as u16
                    } else {
                        (u32::from(base.hp) * combat::integer(row, 11).max(1) as u32 / 100)
                            .clamp(1, 14000) as u16
                    };
                    result.mp = if combat::integer(row, 12) != 0 {
                        (i32::from(base.mp) + combat::integer(row, 12)).clamp(1, 32767) as u16
                    } else {
                        (u32::from(base.mp) * combat::integer(row, 13).max(1) as u32 / 100)
                            .clamp(1, 32767) as u16
                    };
                }
                2 => {
                    result.defence = if combat::integer(row, 6) == 0 && combat::integer(row, 7) > 0
                    {
                        (u32::from(base.defence) * combat::integer(row, 7) as u32 / 100).min(32767)
                            as u16
                    } else {
                        (i32::from(base.defence) + combat::integer(row, 6)).clamp(0, 32767) as u16
                    }
                }
                4 => {
                    result.attack = (u32::from(base.attack) * combat::integer(row, 8).max(0) as u32
                        / 100)
                        .min(32767) as u16
                }
                8 => {
                    for (value, column) in result.resistance.iter_mut().zip(19..25) {
                        *value =
                            (*value).saturating_add(combat::integer(row, column).max(0) as u16);
                    }
                }
                5 => result.delay_ms = base.delay_ms * 100 / combat::integer(row, 4).max(1) as u64,
                18 => {
                    result.defence =
                        (i32::from(result.defence) + combat::integer(row, 6)).max(0) as u16;
                    result.delay_ms = base.delay_ms * 100 / combat::integer(row, 8).max(1) as u64;
                }
                28 => {
                    result.defence =
                        (u32::from(result.defence) * combat::integer(row, 7).max(0) as u32 / 100)
                            .min(32767) as u16
                }
                _ => {}
            }
        }
    }
    if let Some(row) = state
        .buffs
        .get(&24)
        .filter(|b| skills::now_ms() < b.until)
        .and_then(|b| catalog.and_then(|c| c.effects.get(&(4, b.skill))))
    {
        for (value, column) in result.resistance.iter_mut().zip(19..25) {
            *value = (u32::from(*value) * (100 - combat::integer(row, column).clamp(0, 100)) as u32
                / 100) as u16;
        }
    }
    result
}

pub(crate) fn damage_received(amount: u16, session: &Session) -> u16 {
    let mastered = matches!(session.character.class % 100, 6 | 8 | 10 | 12);
    let percent = if mastered && session.progress.points[8] >= 10 {
        85
    } else if mastered && session.progress.points[8] >= 5 {
        90
    } else {
        100
    };
    (u32::from(amount) * percent / 100) as u16
}
pub(crate) async fn npc_strikes(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    now: u64,
) {
    let Some(world) = lobby.monsters.as_ref() else {
        return;
    };
    s.is_admin = crate::admin::is_admin(&lobby.db.lock().unwrap(), account, s.character.slot)
        .unwrap_or(false);
    world.observe(s, now);
    for strike in world.incoming(s.player_id) {
        if s.is_admin {
            continue;
        }
        if strike.zone != s.position.zone || s.skills.hp == 0 {
            continue;
        }
        let result = (|| -> rusqlite::Result<(skills::State, u16, Option<u8>)> {
            let db = lobby.db.lock().unwrap();
            let tx = db.unchecked_transaction()?;
            let mut next = skills::load(&tx, account, s.character.slot)?;
            let amount = damage_received(
                absorb_damage(&mut next, strike.amount, lobby.skills.as_ref()),
                s,
            )
            .min(next.hp);
            next.hp -= amount;
            let reflected = if amount > 0 {
                next.buffs
                    .get(&25)
                    .filter(|b| now < b.until)
                    .and_then(|b| lobby.skills.as_ref().and_then(|c| c.spells.get(&b.skill)))
                    .and_then(|spell| {
                        (5..=7)
                            .contains(&(spell.skill_group % 10))
                            .then_some((spell.skill_group % 10 - 4) as u8)
                    })
            } else {
                None
            };
            if reflected.is_some() {
                next.buffs.remove(&25);
            }
            skills::save_in(&tx, account, s.character.slot, &next)?;
            tx.commit()?;
            Ok((next, amount, reflected))
        })();
        let Ok((next, amount, reflected)) = result else {
            tracing::error!(%peer,"NPC damage persistence failed");
            continue;
        };
        let old_mana = s.skills.mana;
        s.skills = next;
        if let Some(element) = reflected {
            let _ = world.reflect(strike.npc, s.position, amount, element, &s.character.name);
            super::send_response(sock, peer, &None, &[0x31, 5, 25], "MAGE_ARMOR_CONSUMED").await;
        }
        let mut attack = vec![
            0x08,
            1,
            if s.skills.hp == 0 {
                2
            } else if amount > 0 {
                1
            } else {
                0
            },
        ];
        attack.extend(strike.npc.to_le_bytes());
        attack.extend(s.player_id.to_le_bytes());
        if amount > 0 {
            super::send_response(
                sock,
                peer,
                &None,
                &s.skills.hp_packet_from(strike.npc),
                "NPC_DAMAGE",
            )
            .await;
            s.sitting = false;
        }
        if old_mana != s.skills.mana {
            super::send_response(
                sock,
                peer,
                &None,
                &s.skills.mana_packet(),
                "NPC_DAMAGE_MANA",
            )
            .await;
        }
        super::send_response(sock, peer, &None, &attack, "NPC_ATTACK").await;
        refresh(s, lobby.skills.as_ref(), lobby.combat.as_ref());
        if s.skills.hp == 0 {
            s.pending_cast = None;
            s.merchant = None;
            s.anvil = None;
            s.npc_dialog = None;
            let mut dead = vec![0x11];
            dead.extend(s.player_id.to_le_bytes());
            super::send_response(sock, peer, &None, &dead, "PLAYER_DEAD").await;
        }
        world.observe(s, now);
        tracing::info!(%peer,npc=strike.npc,amount,hp=s.skills.hp,"monster attack applied");
    }
}

fn absorb_damage(state: &mut skills::State, amount: u16, catalog: Option<&skills::Catalog>) -> u16 {
    let Some(row) = state
        .buffs
        .get(&31)
        .filter(|b| skills::now_ms() < b.until)
        .and_then(|b| catalog.and_then(|c| c.effects.get(&(4, b.skill))))
    else {
        return amount;
    };
    let percent = combat::integer(row, 25).clamp(0, 100) as u32;
    let cost = if percent == 15 { 4 } else { 1 };
    let absorbed = (u32::from(amount) * percent / 100).min(u32::from(state.mana) / cost);
    let mana = absorbed * cost;
    state.mana = state.mana.saturating_sub(mana.min(65535) as u16);
    amount.saturating_sub(absorbed as u16)
}

pub(crate) async fn training_command(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    text: &[u8],
) -> bool {
    if !crate::admin::is_admin(&lobby.db.lock().unwrap(), account, session.character.slot)
        .unwrap_or(false)
    {
        return false;
    }
    let Some(amount) = std::str::from_utf8(text)
        .ok()
        .map(str::trim)
        .and_then(|text| {
            text.strip_prefix("hurt ")
                .or_else(|| text.strip_prefix("/hurt "))
        })
        .map(str::trim)
        .and_then(|text| text.parse::<u16>().ok())
    else {
        return false;
    };
    let result = {
        let db = lobby.db.lock().unwrap();
        skills::load(&db, account, session.character.slot).and_then(|mut next| {
            let amount = damage_received(
                absorb_damage(&mut next, amount, lobby.skills.as_ref()),
                session,
            );
            let delta = amount.min(next.hp.saturating_sub(1));
            next.hp -= delta;
            skills::save(&db, account, session.character.slot, &next)?;
            Ok((next, delta))
        })
    };
    if let Ok((next, delta)) = result {
        session.skills = next;
        super::send_response(
            sock,
            peer,
            &None,
            &session.skills.mana_packet(),
            "TRAINING_DAMAGE_MANA",
        )
        .await;
        refresh(session, lobby.skills.as_ref(), lobby.combat.as_ref());
        super::send_response(
            sock,
            peer,
            &None,
            &session.skills.hp_packet(-(delta as i16)),
            "TRAINING_DAMAGE",
        )
        .await;
        super::send_response(
            sock,
            peer,
            &None,
            &inventory::attributes(&session.combat, 2),
            "TRAINING_ATTRIBUTES",
        )
        .await;
        tracing::info!(account, amount, "training damage applied");
        let notice = format!(
            "Training damage: {delta}. HP: {}/{}.",
            session.skills.hp, session.skills.max_hp
        );
        let packet = crate::chat::packet(
            7,
            session.character.nation(),
            u16::MAX,
            "",
            notice.as_bytes(),
        );
        super::send_response(sock, peer, &None, &packet, "TRAINING_NOTICE").await;
    }
    true
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
    let catalog = lobby.skills.as_ref()?;
    let row = catalog.effects.get(&(spell.types[0], spell.id));
    let heal = spell.types == [3, 0]
        && row.is_some_and(|row| {
            matches!(combat::integer(row, 2), 1 | 2 | 14)
                && combat::integer(row, 3) >= 0
                && (combat::integer(row, 3) > 0 || combat::integer(row, 4) > 0)
        });
    let category = if spell.types == [4, 0] {
        row.map(|row| combat::integer(row, 1) as u8)
    } else {
        None
    };
    let cure = if spell.types == [5, 0] {
        row.map(|row| combat::integer(row, 1))
            .filter(|kind| matches!(kind, 1 | 2))
    } else {
        None
    };
    let supported = heal
        || cure.is_some()
        || category.is_some_and(|kind| {
            matches!(kind, 1 | 2 | 4 | 5 | 7 | 8 | 10 | 18 | 19 | 23 | 28 | 31)
                || kind == 25
                    && matches!(session.character.class % 100, 3 | 9 | 10)
                    && (5..=7).contains(&(spell.skill_group % 10))
        });
    if !supported {
        return None;
    }
    if target != session.player_id && !(target == u16::MAX && spell.moral == 6) {
        return Some(Err(
            "Friendly effects on other players are not implemented yet.",
        ));
    }
    if !matches!(spell.moral, 1 | 2 | 4 | 6) {
        return Some(Err("This utility's target rules are unavailable."));
    }
    let row = row.unwrap();
    let now = skills::now_ms();
    let mut removed = Vec::new();
    let result = resources::commit(lobby, account, session.character.slot, spell, 1, |state| {
        if cure == Some(1) {
            state.periodic.retain(|_, effect| effect.amount >= 0);
        } else if cure == Some(2) {
            state.buffs.retain(|category, buff| {
                let debuff = catalog
                    .spells
                    .get(&buff.skill)
                    .is_some_and(|s| s.types[0] == 4 && matches!(s.moral, 7 | 10 | 12));
                if debuff {
                    removed.push(*category);
                }
                !debuff
            });
        } else if heal {
            recover(
                state,
                if combat::integer(row, 2) == 14 {
                    1
                } else {
                    combat::integer(row, 2)
                },
                combat::integer(row, 3) as u16,
            );
            let duration = combat::integer(row, 5).max(0) as u64;
            if duration >= 2 {
                let amount = if combat::integer(row, 2) == 14 {
                    let level = f64::from(session.progress.level);
                    (level * (1.0 + level / 30.0)) as i16 + 3
                } else {
                    (combat::integer(row, 4) / (duration as i32 / 2)).clamp(0, 32767) as i16
                };
                if state.periodic.contains_key(&spell.id) {
                    return Err("This recovery effect is already active.");
                }
                state.periodic.insert(
                    spell.id,
                    skills::Periodic {
                        amount,
                        next: now + 2000,
                        until: now + duration * 1000,
                    },
                );
            }
        } else {
            let kind = category.unwrap();
            if state.buffs.get(&kind).is_some_and(|buff| now < buff.until) {
                return Err("A buff of this category is already active.");
            }
            state.buffs.insert(
                kind,
                skills::Buff {
                    skill: spell.id,
                    until: now + combat::integer(row, 3).max(0) as u64 * 1000,
                },
            );
            if matches!(kind, 1 | 7) {
                let stats = effective(session, state, Some(catalog), lobby.combat.as_ref());
                let previous = (state.hp, state.max_hp);
                state.set_limits(stats.hp, stats.mp);
                if kind == 1 {
                    state.hp = previous
                        .0
                        .saturating_add(stats.hp.saturating_sub(previous.1))
                        .min(stats.hp);
                }
            }
        }
        Ok(())
    });
    match result {
        Ok(paid) => {
            resources::sync(sock, peer, lobby, account, session, paid).await;
            for category in removed {
                super::send_response(sock, peer, &None, &[0x31, 5, category], "CURSE_REMOVED")
                    .await;
            }
            if cure == Some(1) {
                super::send_response(sock, peer, &None, &[0x31, 7, 200], "DISEASE_REMOVED").await;
            }
            refresh(session, Some(catalog), lobby.combat.as_ref());
            super::send_response(
                sock,
                peer,
                &None,
                &inventory::attributes(&session.combat, 2),
                "UTILITY_ATTRIBUTES",
            )
            .await;
            super::send_response(
                sock,
                peer,
                &None,
                &skills::effect(spell.id, session.player_id),
                "UTILITY_EFFECT",
            )
            .await;
            tracing::info!(%peer,id=spell.id,heal,category,"rogue utility applied");
            Some(Ok(()))
        }
        Err(reason) => Some(Err(reason)),
    }
}

fn recover(state: &mut skills::State, kind: i32, amount: u16) {
    match kind {
        1 => state.hp = state.hp.saturating_add(amount).min(state.max_hp),
        2 => state.mana = state.mana.saturating_add(amount).min(state.max_mana),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mana_shield_never_absorbs_more_than_the_available_mp_can_pay() {
        let game = reko_common::game_dir();
        if !game.join("Data/Skill_Magic_4.tbl").exists() {
            return;
        }
        let catalog = skills::Catalog::load(&game).unwrap();
        for skill in [110815, 210815] {
            for (mana, damage, remaining_mana) in [
                (1000, 850, 400),
                (603, 850, 3),
                (12, 997, 0),
                (3, 1000, 3),
                (0, 1000, 0),
            ] {
                let mut state = skills::State {
                    mana,
                    ..Default::default()
                };
                state.buffs.insert(
                    31,
                    skills::Buff {
                        skill,
                        until: skills::now_ms() + 40000,
                    },
                );
                assert_eq!(absorb_damage(&mut state, 1000, Some(&catalog)), damage);
                assert_eq!(state.mana, remaining_mana);
                state.buffs.get_mut(&31).unwrap().until = 0;
                assert_eq!(absorb_damage(&mut state, 1000, Some(&catalog)), 1000);
            }
        }
    }
}
