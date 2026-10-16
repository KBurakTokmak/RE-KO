//! Paid arrow volleys and bounded impact tokens.

use crate::{combat, lobby::Lobby, magic, monsters, resources, skills, world_entry::Session};
use std::net::SocketAddr;
use tokio::net::TcpStream;

#[derive(Clone)]
pub(crate) struct Volley {
    pub remaining: u8,
    pub expires_at: u64,
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
    session
        .arrows
        .retain(|_, volley| now <= volley.expires_at && volley.remaining > 0);
    let target = u16::from_le_bytes([payload[8], payload[9]]);
    if payload[1] == 1 {
        super::send_response(sock, peer, &None, payload, "ARROW_CASTING").await;
        return Ok(());
    }
    if payload[1] == 4 {
        // Per-projectile cleanup does not cancel the rest of a paid volley.
        super::send_response(sock, peer, &None, payload, "ARROW_CLEANUP").await;
        return Ok(());
    }
    if payload[1] == 2 {
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
        session.arrows.insert(
            spell.id,
            Volley {
                remaining: arrows,
                expires_at: now + 5000,
            },
        );
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
    let volley = session
        .arrows
        .get_mut(&spell.id)
        .ok_or("No paid arrows are in flight.")?;
    if volley.remaining == 0 || now > volley.expires_at {
        return Err("This arrow volley is exhausted or expired.");
    }
    volley.remaining -= 1;
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
            charisma: session.character.stats[4],
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
            position: session.position,
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
            crate::durability::attack(sock, peer, lobby, account, session, hit.damage).await;
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
            packet[12..14].copy_from_slice(&0i16.to_le_bytes());
            tracing::info!(%peer,id=spell.id,target,reason,"arrow impact missed");
        }
    }
    super::send_response(sock, peer, &None, &packet, "ARROW_EFFECT").await;
    Ok(())
}
