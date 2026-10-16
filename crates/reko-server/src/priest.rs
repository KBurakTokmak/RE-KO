//! Priest HP/AC curses against combat NPCs; other player effects need a registry.
use crate::{
    combat, lobby::Lobby, monsters, movement::Position, resources, skills, world_entry::Session,
};
use std::net::SocketAddr;
use tokio::net::TcpStream;

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    session: &mut Session,
    spell: &skills::Spell,
    payload: &[u8],
) -> Option<Result<(), &'static str>> {
    if !matches!(session.character.class % 100, 4 | 11 | 12)
        || spell.types != [4, 0]
        || !matches!(spell.moral, 7 | 10)
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
    let row = lobby
        .skills
        .as_ref()
        .and_then(|c| c.effects.get(&(4, spell.id)))
        .ok_or("Curse data unavailable.")?;
    let category = combat::integer(row, 1);
    if !matches!(category, 1 | 2) {
        return Err(
            "This curse needs player attributes or active NPC behavior, which is not available yet.",
        );
    }
    let percent = combat::integer(row, if category == 1 { 11 } else { 7 }).clamp(1, 100) as u16;
    let curse = monsters::Curse {
        category: category as u8,
        percent,
        duration_ms: combat::integer(row, 3).max(0) as u64 * 1000,
    };
    let target = u16::from_le_bytes([payload[8], payload[9]]);
    let world = lobby
        .monsters
        .as_ref()
        .ok_or("Combat targets unavailable.")?;
    let mut paid = None;
    if spell.moral == 10 {
        if target != u16::MAX {
            return Err("Choose a ground area for this curse.");
        }
        let x = i16::from_le_bytes([payload[10], payload[11]]);
        let z = i16::from_le_bytes([payload[14], payload[15]]);
        let (Ok(x), Ok(z)) = (
            u16::try_from(i32::from(x) * 10),
            u16::try_from(i32::from(z) * 10),
        ) else {
            return Err("Invalid curse ground coordinates.");
        };
        if f32::from(x.abs_diff(session.position.x))
            .hypot(f32::from(z.abs_diff(session.position.z)))
            > (f32::from(spell.range) + 2.0) * 10.0
        {
            return Err("The curse area is too far away.");
        }
        let center = Position {
            x,
            z,
            ..session.position
        };
        let radius = combat::integer(row, 2).max(0) as f32;
        if radius == 0.0 {
            return Err("Invalid curse radius.");
        }
        paid = Some(resources::commit(
            lobby,
            account,
            session.character.slot,
            spell,
            1,
            |_| Ok(()),
        )?);
        for id in world.area_targets(center, radius) {
            let _ = world.curse(id, center, (radius - 2.0).max(0.0), curse, || Ok(()));
        }
    } else {
        world.curse(
            target,
            session.position,
            f32::from(spell.range),
            curse,
            || {
                paid = Some(resources::commit(
                    lobby,
                    account,
                    session.character.slot,
                    spell,
                    1,
                    |_| Ok(()),
                )?);
                Ok(())
            },
        )?;
    }
    resources::sync(sock, peer, lobby, account, session, paid.unwrap()).await;
    let mut effect = payload.to_vec();
    effect[12..14].copy_from_slice(&1u16.to_le_bytes());
    super::send_response(sock, peer, &None, &effect, "PRIEST_CURSE").await;
    tracing::info!(%peer,id=spell.id,target,category,percent,"priest NPC curse applied");
    Ok(())
}
