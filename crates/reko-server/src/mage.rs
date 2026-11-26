//! Original-table elemental spells, staff strikes and area impacts.

use crate::{
    combat, lobby::Lobby, magic, monsters, movement::Position, resources, skills,
    world_entry::Session,
};
use std::net::SocketAddr;
use tokio::net::TcpStream;

#[derive(Clone)]
pub(crate) struct Flight {
    pub target: u16,
    pub expires: u64,
    pub origin: Position,
    pub center: Option<Position>,
}

fn is_mage(class: u16) -> bool {
    matches!(class % 100, 3 | 9 | 10)
}

fn damage(
    session: &Session,
    lobby: &Lobby,
    row: &[reko_assets::CellValue],
) -> monsters::MagicDamage {
    let charisma =
        u16::from(session.character.stats[4]).saturating_add(session.combat.bonus_stats[4]);
    let mut bonus = 0;
    if let Some(buff) = session.skills.buffs.get(&10)
        && skills::now_ms() < buff.until
        && let Some(effect) = lobby
            .skills
            .as_ref()
            .and_then(|c| c.effects.get(&(4, buff.skill)))
    {
        bonus += (combat::integer(effect, 9) - 100).max(0) as u32 * u32::from(charisma) / 100;
    }
    if session.equipment[8].id == 0
        && let Some((base, extension)) = lobby
            .combat
            .as_ref()
            .and_then(|c| c.item(session.equipment[6].id))
        && combat::integer(base, 10) / 10 == 11
    {
        let staff = combat::integer(base, 15) + extension.map_or(0, |e| combat::integer(e, 8));
        bonus += (staff.max(0) as f32 * 0.8
            + (staff.max(0) * i32::from(session.progress.level) / 60) as f32)
            as u32;
    }
    monsters::MagicDamage {
        element: combat::integer(row, 6).clamp(0, 6) as u8,
        first: combat::integer(row, 3).min(0).unsigned_abs().min(32767) as u16,
        over_time: combat::integer(row, 4).min(0).unsigned_abs().min(32767) as u16,
        duration: combat::integer(row, 5).clamp(0, 255) as u8,
        charisma,
        bonus: bonus.min(32767) as u16,
        raw_over_time: combat::integer(row, 6) == 4,
        raw_first: combat::integer(row, 2) == 2,
    }
}

fn pay(
    lobby: &Lobby,
    account: &str,
    session: &Session,
    spell: &skills::Spell,
) -> Result<resources::Paid, &'static str> {
    resources::commit(lobby, account, session.character.slot, spell, 1, |state| {
        // 1453 User.h:23 / MagicInstance.cpp:421-455: Type1+3 Mage staff skills
        // (e.g. 42 Fire Blade, 72 Fire Staff) have 0 table recast time and use
        // PLAYER_SKILL_REQUEST_INTERVAL (700 ms) so they cannot be packet-spammed.
        if spell.types == [1, 3] && spell.cooldown_ms < 700 {
            state.cooldowns.insert(spell.id, skills::now_ms() + 700);
        }
        Ok(())
    })
}

pub(crate) fn center(
    origin: Position,
    spell: &skills::Spell,
    p: &[u8],
) -> Result<Position, &'static str> {
    let x = i16::from_le_bytes([p[10], p[11]]);
    let z = i16::from_le_bytes([p[14], p[15]]);
    let (Ok(x), Ok(z)) = (
        u16::try_from(i32::from(x) * 10),
        u16::try_from(i32::from(z) * 10),
    ) else {
        return Err("Invalid ground target coordinates.");
    };
    let distance = f32::from(x.abs_diff(origin.x)).hypot(f32::from(z.abs_diff(origin.z))) / 10.0;
    if distance > f32::from(spell.range) + 2.0 {
        return Err("The ground target is too far away.");
    }
    Ok(Position { x, z, ..origin })
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    spell: &skills::Spell,
    payload: &[u8],
) -> Option<Result<(), &'static str>> {
    let warrior_spell = matches!(session.character.class % 100, 1 | 5 | 6 | 4 | 11 | 12)
        && spell.types == [3, 0]
        && matches!(spell.moral, 7 | 10);
    if (!is_mage(session.character.class) && !warrior_spell)
        || spell.skill_group == 0
        || !matches!(spell.types, [3, 0 | 4] | [1, 3])
    {
        return None;
    }
    if lobby
        .skills
        .as_ref()
        .and_then(|c| c.effects.get(&(3, spell.id)))
        .is_some_and(|row| combat::integer(row, 2) == 16)
    {
        return None;
    }
    Some(execute(sock, peer, lobby, account, session, spell, payload).await)
}

async fn execute(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    spell: &skills::Spell,
    payload: &[u8],
) -> Result<(), &'static str> {
    let catalog = lobby.skills.as_ref().ok_or("Skill data unavailable.")?;
    let row = catalog
        .effects
        .get(&(3, spell.id))
        .ok_or("Elemental skill data unavailable.")?;
    let direct_type = combat::integer(row, 2);
    if !matches!(direct_type, 1 | 2 | 8) {
        return Err("This spell affects player resources; player combat is not available yet.");
    }
    let mana_burn = direct_type == 2 && combat::integer(row, 5) == 0;
    if !matches!(spell.moral, 7 | 10) {
        return Err("Unsupported elemental target rule.");
    }
    let physical = spell.types[0] == 1;
    if physical && session.combat.weapon_kind / 10 != 11 {
        return Err("This skill requires an equipped staff.");
    }
    let target = u16::from_le_bytes([payload[8], payload[9]]);
    if payload[1] == 4 {
        if session
            .pending_cast
            .as_ref()
            .is_some_and(|c| c.skill == spell.id)
        {
            session.pending_cast = None;
        }
        return Ok(());
    }
    if (spell.moral == 10) != (target == u16::MAX) {
        return Err("Invalid spell target type.");
    }
    let now = skills::now_ms();
    session
        .magic_projectiles
        .retain(|_, flight| now <= flight.expires);
    if payload[1] == 1 {
        session.pending_cast = Some(magic::Cast {
            skill: spell.id,
            target,
            expires_at: now + spell.cast_ms + 3000,
        });
        super::send_response(sock, peer, &None, payload, "MAGE_CASTING").await;
        return Ok(());
    }
    let flight = if payload[1] == 3 {
        session.magic_projectiles.remove(&spell.id)
    } else {
        None
    };
    let launched = flight.is_some();
    if let Some(flight) = &flight {
        if flight.target != target || now > flight.expires {
            return Err("No matching projectile is in flight.");
        }
    } else if spell.cast_ms > 0
        && !session
            .skills
            .buffs
            .get(&23)
            .is_some_and(|buff| now < buff.until)
        && !session
            .pending_cast
            .as_ref()
            .is_some_and(|c| c.matches(spell.id, target, now))
    {
        return Err("No matching active mage cast was found.");
    }
    if !matches!(payload[1], 2 | 3) {
        return Err("Unsupported mage spell phase.");
    }
    if payload[1] == 2 && (physical || session.magic_projectiles.contains_key(&spell.id)) {
        return Err("This projectile cannot be launched again.");
    }
    let world = lobby
        .monsters
        .as_ref()
        .ok_or("Combat targets unavailable.")?;
    let origin = flight.as_ref().map_or(session.position, |f| f.origin);
    let area = if spell.moral == 10 {
        Some(
            if let Some(center) = flight.as_ref().and_then(|f| f.center) {
                center
            } else if matches!(spell.id, 105760 | 106760 | 205760 | 206760)
                && payload[10..16].iter().all(|&value| value == 0)
            {
                origin
            } else {
                center(origin, spell, payload)?
            },
        )
    } else {
        None
    };
    let radius = combat::integer(row, 1).max(0) as f32;
    if area.is_some() && radius == 0.0 {
        return Err("Invalid area spell radius.");
    }
    let elemental = damage(session, lobby, row);
    let primary = catalog.effects.get(&(1, spell.id));
    let make_attack = |position, range| monsters::Attack {
        position,
        range,
        attack: if physical { session.combat.attack } else { 0 },
        percent: if physical {
            primary.map_or(0, |r| combat::integer(r, 3).clamp(0, 32767) as u16)
        } else {
            0
        },
        extra: if physical {
            catalog
                .type1_add_damage
                .get(&spell.id)
                .copied()
                .unwrap_or(0)
        } else {
            0
        },
        skill: true,
        drain_percent: 0,
        arrow: false,
        penetrate: false,
        magic: Some(elemental),
    };
    if payload[1] == 2 {
        if area.is_none() {
            if crate::pvp::is_player_id(target) {
                crate::pvp::validate_target(
                    lobby,
                    session,
                    target,
                    origin,
                    f32::from(spell.range),
                )?;
            } else {
                world.validate(target, origin, f32::from(spell.range))?;
            }
        }
        let paid = pay(lobby, account, session, spell)?;
        if session
            .pending_cast
            .as_ref()
            .is_some_and(|c| c.matches(spell.id, target, now))
        {
            session.pending_cast = None;
        }
        session.magic_projectiles.insert(
            spell.id,
            Flight {
                target,
                expires: now + 5000,
                origin,
                center: area,
            },
        );
        resources::sync(sock, peer, lobby, account, session, paid).await;
        super::send_response(sock, peer, &None, payload, "MAGE_FLYING").await;
        return Ok(());
    }
    let slow_debuff = if spell.types[1] == 4
        && let Some(slow) = catalog.effects.get(&(4, spell.id))
        && combat::integer(slow, 1) == 6
    {
        Some((6u8, spell.id, combat::integer(slow, 3).max(0) as u64 * 1000))
    } else if !mana_burn && elemental.element == 3 && elemental.first > 0 {
        Some((47u8, spell.id, 2000))
    } else {
        None
    };
    let mut paid = None;
    let mut hits: Vec<(u16, u32)> = Vec::new();
    if let Some(center) = area {
        if !launched {
            paid = Some(pay(lobby, account, session, spell)?);
        }
        for id in world.area_targets(center, radius) {
            if let Ok(hit) = world.hit_owned(
                id,
                make_attack(center, (radius - 2.0).max(0.0)),
                Some(&session.character.name),
                || Ok(()),
            ) {
                hits.push((id, hit.damage));
            }
        }
        for id in crate::pvp::area_targets(lobby, session, center, radius) {
            if mana_burn {
                if let Ok(drained) = crate::pvp::drain_player_mana(
                    lobby,
                    session,
                    id,
                    center,
                    (radius - 2.0).max(0.0),
                    elemental.first,
                    || Ok(()),
                ) {
                    hits.push((id, drained));
                }
            } else if let Ok(hit) = crate::pvp::hit_player(
                lobby,
                account,
                session,
                id,
                make_attack(center, (radius - 2.0).max(0.0)),
                false,
                slow_debuff,
                || Ok(()),
            ) {
                hits.push((id, hit.damage));
            }
        }
    } else if crate::pvp::is_player_id(target) {
        if mana_burn {
            let drained = crate::pvp::drain_player_mana(
                lobby,
                session,
                target,
                origin,
                f32::from(spell.range),
                elemental.first,
                || {
                    if !launched {
                        paid = Some(pay(lobby, account, session, spell)?);
                    }
                    Ok(())
                },
            )?;
            hits.push((target, drained));
        } else {
            let hit = crate::pvp::hit_player(
                lobby,
                account,
                session,
                target,
                make_attack(origin, f32::from(spell.range)),
                false,
                slow_debuff,
                || {
                    if !launched {
                        paid = Some(pay(lobby, account, session, spell)?);
                    }
                    Ok(())
                },
            )?;
            hits.push((target, hit.damage));
        }
    } else {
        let hit = world.hit_owned(
            target,
            make_attack(origin, f32::from(spell.range)),
            Some(&session.character.name),
            || {
                if !launched {
                    paid = Some(pay(lobby, account, session, spell)?);
                }
                Ok(())
            },
        )?;
        hits.push((target, hit.damage));
    }
    // A paid projectile belongs to an earlier completed cast. Its impact must
    // not erase the next spell being cast while that projectile travels.
    if !launched
        && session
            .pending_cast
            .as_ref()
            .is_some_and(|c| c.matches(spell.id, target, now))
    {
        session.pending_cast = None;
    }
    if let Some(paid) = paid {
        resources::sync(sock, peer, lobby, account, session, paid).await;
    }
    if combat::integer(row, 2) == 8 {
        let absorbed: u32 = hits
            .iter()
            .filter(|(id, _)| crate::pvp::is_player_id(*id))
            .map(|(_, d)| *d)
            .sum();
        if absorbed > 0 {
            let healed = (|| -> rusqlite::Result<Option<(skills::State, u16)>> {
                let db = lobby.db.lock().unwrap();
                let mut state = skills::load(&db, account, session.character.slot)?;
                if state.hp == 0 {
                    return Ok(None);
                }
                let before = state.hp;
                state.hp = state
                    .hp
                    .saturating_add(absorbed.min(32767) as u16)
                    .min(state.max_hp);
                let delta = state.hp - before;
                if delta > 0 {
                    skills::save(&db, account, session.character.slot, &state)?;
                }
                Ok(Some((state, delta)))
            })();
            if let Ok(Some((state, delta))) = healed
                && delta > 0
            {
                session.skills = state;
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &session.skills.hp_packet(delta as i16),
                    "MAGE_ABSORB_HP",
                )
                .await;
            }
        }
    }
    if spell.types[1] == 4
        && let Some(slow) = catalog.effects.get(&(4, spell.id))
        && combat::integer(slow, 1) == 6
    {
        for &(id, _) in &hits {
            if !crate::pvp::is_player_id(id) {
                world.slow(
                    id,
                    combat::integer(slow, 5).clamp(1, 200) as u16,
                    combat::integer(slow, 3).max(0) as u64 * 1000,
                    true,
                );
            }
        }
    }
    if physical {
        for &(_, damage) in &hits {
            crate::durability::attack(
                sock,
                peer,
                lobby,
                account,
                session,
                damage.min(32767) as u16,
            )
            .await;
        }
    }
    let mut effect = payload.to_vec();
    effect[12..14].copy_from_slice(&1u16.to_le_bytes());
    super::send_response(sock, peer, &None, &effect, "MAGE_EFFECT").await;
    tracing::info!(%peer,id=spell.id,name=%spell.name,targets=hits.len(),damage=hits.iter().map(|(_,d)|*d).sum::<u32>(),"mage spell applied");
    Ok(())
}
