use crate::{admin, chat, lobby::Lobby, social::Hub, world_entry::Session};
use std::time::{Duration, Instant};

const MAX_MUTE_MINUTES: u64 = 60;
const PLAYERS_PER_PAGE: usize = 5;

pub(crate) fn notice(text: &[u8]) -> Vec<u8> {
    chat::packet(7, 0, u16::MAX, "", text)
}

pub(crate) fn is_muted(hub: &mut Hub, account: &str) -> bool {
    let account = account.to_ascii_lowercase();
    let Some(until) = hub.chat_mutes.get(&account) else {
        return false;
    };
    if Instant::now() < *until {
        return true;
    }
    hub.chat_mutes.remove(&account);
    false
}

pub(crate) fn command(
    lobby: &Lobby,
    account: &str,
    actor: &Session,
    text: &[u8],
) -> Option<Vec<u8>> {
    // The chat parser trims neither whitespace nor encoding. Command names are ASCII,
    // and announcements preserve the client's raw text bytes.
    let mut words = text.splitn(3, |b| *b == b' ');
    let prefix = words.next()?;
    if !prefix.eq_ignore_ascii_case(b"op") && !prefix.eq_ignore_ascii_case(b"/op") {
        return None;
    }
    if !admin::is_admin(&lobby.db.lock().unwrap(), account, actor.character.slot).unwrap_or(false) {
        return Some(notice(
            b"Operator commands require an administrator character.",
        ));
    }
    let command = words.next().unwrap_or_default();
    let args = words.next().unwrap_or_default().trim_ascii();
    if command.eq_ignore_ascii_case(b"weather") {
        return Some(notice(
            lobby
                .sky
                .lock()
                .unwrap()
                .set_weather(args, &chrono::Local::now()),
        ));
    }
    if command.eq_ignore_ascii_case(b"invade") {
        return Some(notice(&invade(lobby, args)));
    }
    let mut hub = lobby.social.lock().unwrap();
    if command.eq_ignore_ascii_case(b"players") {
        return Some(players(&hub, args));
    }
    if command.eq_ignore_ascii_case(b"kick") {
        let Ok(name) = std::str::from_utf8(args) else {
            return Some(usage());
        };
        if name.is_empty() || name.contains(' ') {
            return Some(usage());
        }
        let Some(id) = hub.named(name) else {
            return Some(notice(b"That character is not online."));
        };
        if id == actor.player_id {
            return Some(notice(b"Use the game menu to log out of your own session."));
        }
        let name = hub.players[&id].s.character.name.clone();
        match hub.request_disconnect(id) {
            Ok(true) => {}
            Ok(false) => return Some(notice(b"That session is unavailable for disconnect.")),
            Err(error) => {
                tracing::warn!(%error, target=%name, "operator disconnect failed");
                return Some(notice(b"The server could not disconnect that session."));
            }
        }
        tracing::info!(operator=%actor.character.name, target=%name, "operator requested disconnect");
        return Some(notice(
            format!("Disconnect requested for {name}.").as_bytes(),
        ));
    }
    if command.eq_ignore_ascii_case(b"announce") && !args.is_empty() {
        let recipients: Vec<_> = hub
            .players
            .iter()
            .filter(|(id, player)| {
                **id != actor.player_id && player.s.entered && !player.s.zone_loading
            })
            .map(|(&id, _)| id)
            .collect();
        let mut message = b"[Server] ".to_vec();
        message.extend_from_slice(args);
        let packet = notice(&message);
        hub.send_many(&recipients, &packet);
        tracing::info!(operator=%actor.character.name, recipients=recipients.len() + 1, "operator announcement");
        return Some(packet);
    }
    let mute = command.eq_ignore_ascii_case(b"mute");
    if !mute && !command.eq_ignore_ascii_case(b"unmute") {
        return Some(usage());
    }
    let mut args = args.split(|b| *b == b' ').filter(|part| !part.is_empty());
    let Some(name) = args.next().and_then(|name| std::str::from_utf8(name).ok()) else {
        return Some(usage());
    };
    let minutes = if mute {
        let duration = args
            .next()
            .and_then(|number| std::str::from_utf8(number).ok())
            .and_then(|number| number.parse::<u64>().ok())
            .filter(|n| (1..=MAX_MUTE_MINUTES).contains(n));
        let Some(minutes) = duration else {
            return Some(usage());
        };
        Some(minutes)
    } else {
        None
    };
    if args.next().is_some() {
        return Some(usage());
    }
    let Some(target) = hub.named(name) else {
        return Some(notice(b"That character is not online."));
    };
    let player = &hub.players[&target];
    let target_account = player.account.to_ascii_lowercase();
    let name = player.s.character.name.clone();
    let now = Instant::now();
    hub.chat_mutes.retain(|_, until| now < *until);
    let reply = if let Some(minutes) = minutes {
        hub.chat_mutes
            .insert(target_account, now + Duration::from_secs(minutes * 60));
        hub.send(
            target,
            notice(format!("Your chat is muted for {minutes} minute(s).").as_bytes()),
        );
        format!("Muted {name} for {minutes} minute(s).")
    } else {
        if hub.chat_mutes.remove(&target_account).is_none() {
            return Some(notice(b"That account is not muted."));
        }
        hub.send(target, notice(b"Your chat mute was removed."));
        format!("Unmuted {name}.")
    };
    tracing::info!(operator=%actor.character.name, target=%name, ?minutes, "operator chat moderation");
    Some(notice(reply.as_bytes()))
}

fn usage() -> Vec<u8> {
    notice(b"Use op players [page], op kick <character>, op announce <message>, op mute <character> <1-60 minutes>, op unmute <character>, or op weather clear|rain|snow <0-100>.")
}

fn players(hub: &Hub, args: &[u8]) -> Vec<u8> {
    let page = if args.is_empty() {
        Some(1)
    } else {
        std::str::from_utf8(args)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
    };
    let mut players: Vec<_> = hub.players.values().map(|player| &player.s).collect();
    let pages = players.len().div_ceil(PLAYERS_PER_PAGE).max(1);
    let Some(page) = page.filter(|number| (1..=pages).contains(number)) else {
        return notice(format!("Use op players <page> from 1 to {pages}.").as_bytes());
    };
    if players.is_empty() {
        return notice(b"No characters are online.");
    }
    players.sort_by_cached_key(|s| s.character.name.to_ascii_lowercase());
    let mut message = format!("Online {} (page {page}/{pages}): ", players.len());
    for (index, player) in players
        .into_iter()
        .skip((page - 1) * PLAYERS_PER_PAGE)
        .take(PLAYERS_PER_PAGE)
        .enumerate()
    {
        if index > 0 {
            message.push_str(" | ");
        }
        message.push_str(&player.character.name);
        if player.zone_loading {
            message.push_str(" [loading]");
        } else {
            use std::fmt::Write;
            let _ = write!(message, " [zone {}]", player.position.zone);
        }
    }
    notice(message.as_bytes())
}

/// `op invade <zone> on|off`: make a homeland zone's guards and towers attack
/// enemy-nation players (events do the same during invasions).
fn invade(lobby: &Lobby, args: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(args);
    let mut parts = text.split_whitespace();
    let (Some(Ok(zone)), Some(state), None) = (
        parts.next().map(str::parse::<u8>),
        parts.next(),
        parts.next(),
    ) else {
        return b"Use op invade <zone> on|off.".to_vec();
    };
    let on = match state.to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => return b"Use op invade <zone> on|off.".to_vec(),
    };
    if matches!(zone, 71 | 72) {
        return b"Guards in zones 71 and 72 always attack enemy-nation players.".to_vec();
    }
    if !crate::population::GUARD_ZONES.contains(&zone)
        && !crate::population::TOWER_ZONES.contains(&zone)
    {
        return b"That zone has no simulated guards or towers.".to_vec();
    }
    let Some(world) = lobby.monsters.as_ref() else {
        return b"Monster world unavailable.".to_vec();
    };
    world.set_guard_invaded(zone, on);
    format!(
        "Zone {zone} guards now {} enemy-nation players.",
        if on { "attack" } else { "ignore" }
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_pages_are_bounded_and_loading_characters_have_no_stale_zone() {
        let lobby = crate::social::tests::lobby();
        for name in ["Charlie", "Echo", "Alpha", "Delta", "Bravo", "Foxtrot"] {
            let mut s = crate::social::tests::player(&lobby, name, 1, 80);
            if name == "Foxtrot" {
                s.zone_loading = true;
                s.entered = false;
                lobby.social.lock().unwrap().synchronize(name, &s);
            }
        }
        let hub = lobby.social.lock().unwrap();
        assert!(players(&hub, b"").ends_with(
            b"Online 6 (page 1/2): Alpha [zone 21] | Bravo [zone 21] | Charlie [zone 21] | Delta [zone 21] | Echo [zone 21]"
        ));
        assert!(players(&hub, b"2").ends_with(b"Online 6 (page 2/2): Foxtrot [loading]"));
        for page in [b"0".as_slice(), b"3", b"-1", b"18446744073709551616"] {
            assert!(players(&hub, page).ends_with(b"from 1 to 2."));
        }
    }
}
