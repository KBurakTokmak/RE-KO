//! Original-table elemental spells, staff strikes and area impacts.

use crate::{
    combat, lobby::Lobby, magic, monsters, movement::Position, resources, skills,
    world_entry::Session,
};
use std::net::SocketAddr;
use tokio::net::TcpStream;

#[derive(Clone)]
pub(crate) struct Flight {
    target: u16,
    expires: u64,
    origin: Position,
    center: Option<Position>,
}

fn is_mage(class: u16) -> bool {
    matches!(class % 100, 3 | 9 | 10)
}

fn damage(
    session: &Session,
    lobby: &Lobby,
    row: &[reko_assets::CellValue],
) -> monsters::MagicDamage {
    let charisma = if is_mage(session.character.class) {
        u16::from(session.character.stats[4]).saturating_add(session.combat.bonus_stats[4])
    } else {
        60
    };
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
        if spell.cast_ms > 0 {
            state.buffs.remove(&23);
        }
        Ok(())
    })
}

fn center(origin: Position, spell: &skills::Spell, p: &[u8]) -> Result<Position, &'static str> {
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
    if !matches!(combat::integer(row, 2), 1 | 8)
        && !(combat::integer(row, 2) == 2 && combat::integer(row, 5) > 0)
    {
        return Err("This spell affects player resources; player combat is not available yet.");
    }
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
        extra: 0,
        skill: true,
        drain_percent: 0,
        arrow: false,
        penetrate: false,
        magic: Some(elemental),
    };
    if payload[1] == 2 {
        if area.is_none() {
            world.validate(target, origin, f32::from(spell.range))?;
        }
        let paid = pay(lobby, account, session, spell)?;
        session.pending_cast = None;
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
    let mut paid = None;
    let mut hits = Vec::new();
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
                hits.push((id, hit));
            }
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
        hits.push((target, hit));
    }
    session.pending_cast = None;
    if let Some(paid) = paid {
        resources::sync(sock, peer, lobby, account, session, paid).await;
    }
    if spell.types[1] == 4
        && let Some(slow) = catalog.effects.get(&(4, spell.id))
        && combat::integer(slow, 1) == 6
    {
        for (id, _) in &hits {
            world.slow(
                *id,
                combat::integer(slow, 5).clamp(1, 200) as u16,
                combat::integer(slow, 3).max(0) as u64 * 1000,
            );
        }
    }
    if physical {
        for (_, hit) in &hits {
            crate::durability::attack(
                sock,
                peer,
                lobby,
                account,
                session,
                hit.damage.min(32767) as u16,
            )
            .await;
        }
    }
    let mut effect = payload.to_vec();
    effect[12..14].copy_from_slice(&1u16.to_le_bytes());
    super::send_response(sock, peer, &None, &effect, "MAGE_EFFECT").await;
    tracing::info!(%peer,id=spell.id,name=%spell.name,targets=hits.len(),damage=hits.iter().map(|(_,h)|h.damage).sum::<u32>(),"mage spell applied");
    Ok(())
}
