//! Paid arrow volleys and bounded impact tokens.

use crate::{combat, lobby::Lobby, magic, monsters, resources, skills, world_entry::Session};
use std::net::SocketAddr;
use tokio::net::TcpStream;

#[derive(Clone)]
pub(crate) struct Volley {
    pub expires_at: u64,
    origin: crate::movement::Position,
    indices: Vec<(i16, u8)>,
}

impl Volley {
    fn quota(&self, index: i16) -> Option<u8> {
        self.indices
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, n)| *n)
    }

    fn remaining(&self) -> u8 {
        self.indices.iter().map(|(_, n)| n).sum()
    }
}

pub(crate) fn resolve(lobby: &Lobby, s: &Session, payload: &[u8]) -> Option<u32> {
    let cleanup = payload[1] == 4;
    if cleanup && i16::from_le_bytes([payload[16], payload[17]]) != -101 {
        return None;
    }
    let offset = if cleanup { 18 } else { 16 };
    let index = i16::from_le_bytes([payload[offset], payload[offset + 1]]);
    let mut matches = s.arrows.iter().filter(|(id, volleys)| {
        lobby
            .skills
            .as_ref()
            .is_some_and(|c| c.spells[id].types[0] == 2)
            && volleys.iter().any(|v| {
                skills::now_ms() <= v.expires_at
                    && v.origin.zone == s.position.zone
                    && v.quota(index).is_some_and(|n| cleanup || n > 0)
            })
    });
    let (&id, _) = matches.next()?;
    matches.next().is_none().then_some(id)
}

pub(crate) async fn handle(
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
        .get(&(2, spell.id))
        .ok_or("Arrow effect data unavailable.")?;
    if spell.moral != 7 || !matches!(spell.types[1], 0 | 3 | 4) {
        return Err("This arrow target/effect family is not supported.");
    }
    if !matches!(session.combat.weapon_kind / 10, 7 | 8 | 10) {
        return Err("This skill requires an equipped bow.");
    }
    let arrows = combat::integer(row, 5).clamp(1, 5) as u8;
    let secondary = catalog.effects.get(&(spell.types[1], spell.id));
    if spell.types[1] == 3
        && !secondary.is_some_and(|row| {
            combat::integer(row, 2) == 1
                && combat::integer(row, 3) <= 0
                && combat::integer(row, 4) <= 0
        })
    {
        return Err("This arrow's secondary effect data is unavailable.");
    }
    if spell.types[1] == 4
        && !secondary.is_some_and(|row| matches!(combat::integer(row, 1), 6 | 21))
    {
        return Err("This arrow's secondary buff is not supported.");
    }
    let now = skills::now_ms();
    session.arrows.retain(|_, volleys| {
        volleys.retain(|v| now <= v.expires_at && v.origin.zone == session.position.zone);
        !volleys.is_empty()
    });
    let target = u16::from_le_bytes([payload[8], payload[9]]);
    if payload[1] == 1 {
        super::send_response(sock, peer, &None, payload, "ARROW_CASTING").await;
        return Ok(());
    }
    if payload[1] == 4 {
        // Central and first side arrows share an index in the original client.
        // Removing that mapping after the first collision loses the second skill ID.
        let index = i16::from_le_bytes([payload[18], payload[19]]);
        if i16::from_le_bytes([payload[16], payload[17]]) != -101 {
            super::send_response(sock, peer, &None, payload, "ARROW_CANCELLED").await;
        } else if session
            .arrows
            .get(&spell.id)
            .and_then(|volleys| volleys.iter().find_map(|v| v.quota(index)))
            .is_none_or(|quota| quota == 0)
        {
            super::send_response(sock, peer, &None, payload, "ARROW_CLEANUP").await;
        }
        return Ok(());
    }
    if payload[1] == 2 {
        let base = i16::from_le_bytes([payload[16], payload[17]]);
        let count = if arrows == 1 { 1 } else { arrows - 1 };
        if base < 0 || base.checked_add(i16::from(count - 1)).is_none() {
            return Err("Invalid arrow flight index.");
        }
        let indices: Vec<_> = (0..count)
            .map(|n| {
                (
                    base + i16::from(n),
                    if n == 0 && arrows > 1 { 2 } else { 1 },
                )
            })
            .collect();
        if session
            .arrows
            .values()
            .flatten()
            .filter(|v| v.remaining() > 0)
            .count()
            >= 64
            || session
                .arrows
                .values()
                .flatten()
                .any(|v| v.remaining() > 0 && indices.iter().any(|(i, _)| v.quota(*i).is_some()))
        {
            return Err("Arrow flight indices are already in use.");
        }
        if spell.skill_group.is_multiple_of(10) && now < session.next_attack {
            return Err("Your bow is not ready for another shot.");
        }
        if lobby
            .monsters
            .as_ref()
            .and_then(|world| world.health(target, session.position.zone))
            .is_none_or(|packet| u32::from_le_bytes(packet[8..12].try_into().unwrap()) == 0)
        {
            return Err("Select a known combat target.");
        }
        let paid = resources::commit(
            lobby,
            account,
            session.character.slot,
            spell,
            u16::from(arrows),
            |_| Ok(()),
        )?;
        resources::sync(sock, peer, lobby, account, session, paid).await;
        for volleys in session.arrows.values_mut() {
            volleys.retain(|v| {
                v.remaining() > 0 || !indices.iter().any(|(i, _)| v.quota(*i).is_some())
            });
        }
        session.arrows.entry(spell.id).or_default().push(Volley {
            expires_at: now + 5000,
            origin: session.position,
            indices,
        });
        if spell.skill_group.is_multiple_of(10) {
            session.next_attack = now + session.combat.delay_ms;
        }
        if magic::remove_stealth(lobby, account, session, false)
            .map_err(|_| "Unable to clear stealth.")?
        {
            super::send_response(sock, peer, &None, &[0x31, 5, 91], "STEALTH_BROKEN_ON_ARROW")
                .await;
        }
        super::send_response(sock, peer, &None, payload, "ARROW_FLYING").await;
        tracing::info!(%peer,id=spell.id,arrows,"arrow volley paid");
        return Ok(());
    }
    if payload[1] != 3 {
        return Err("Unknown arrow phase.");
    }
    let index = i16::from_le_bytes([payload[16], payload[17]]);
    let Some(volley) = session.arrows.get_mut(&spell.id).and_then(|volleys| {
        volleys
            .iter_mut()
            .find(|v| v.quota(index).is_some_and(|n| n > 0))
    }) else {
        // Side arrows can reach distant terrain after the paid hit window.
        // Stale/replayed completions remain harmless without a chat failure.
        tracing::debug!(%peer, id = spell.id, index, target, "unmatched arrow completion ignored");
        return Ok(());
    };
    volley
        .indices
        .iter_mut()
        .find(|(i, _)| *i == index)
        .unwrap()
        .1 -= 1;
    let origin = volley.origin;
    let world = lobby
        .monsters
        .as_ref()
        .ok_or("Combat targets are unavailable.")?;
    let mut packet = payload.to_vec();
    let magic = if spell.types[1] == 3 {
        secondary.map(|row| monsters::MagicDamage {
            element: combat::integer(row, 6).clamp(0, 6) as u8,
            first: combat::integer(row, 3).unsigned_abs().min(32767) as u16,
            over_time: combat::integer(row, 4).unsigned_abs().min(32767) as u16,
            duration: combat::integer(row, 5).clamp(0, 255) as u8,
            charisma: u16::from(session.character.stats[4])
                .saturating_add(session.combat.bonus_stats[4]),
            bonus: 0,
            raw_over_time: false,
            raw_first: false,
        })
    } else {
        None
    };
    let limit = ((session.combat.range + 4.0) * combat::integer(row, 4).max(0) as f32 / 100.0)
        .max(f32::from(spell.range) + 4.0);
    let result = world.hit_owned(
        target,
        monsters::Attack {
            position: origin,
            range: (limit - 2.0).max(0.0),
            attack: session.combat.attack,
            percent: combat::integer(row, 3).clamp(0, 32767) as u16,
            extra: 0,
            skill: true,
            drain_percent: 0,
            arrow: true,
            penetrate: combat::integer(row, 1) == 1,
            magic,
        },
        Some(&session.character.name),
        || Ok(()),
    );
    match result {
        Ok(hit) => {
            packet[16..18].copy_from_slice(&0i16.to_le_bytes());
            crate::durability::attack(
                sock,
                peer,
                lobby,
                account,
                session,
                hit.damage.min(32767) as u16,
            )
            .await;
            if spell.types[1] == 4
                && let Some(row) = secondary
                && combat::integer(row, 1) == 6
            {
                world.slow(
                    target,
                    combat::integer(row, 5).clamp(1, 200) as u16,
                    combat::integer(row, 3).max(0) as u64 * 1000,
                );
            }
            tracing::info!(%peer,id=spell.id,target,damage=hit.damage,"arrow impact applied");
        }
        Err(reason) => {
            packet[16..18].copy_from_slice(&(-104i16).to_le_bytes());
            tracing::info!(%peer,id=spell.id,target,reason,"arrow impact missed");
        }
    }
    super::send_response(sock, peer, &None, &packet, "ARROW_EFFECT").await;
    Ok(())
}
