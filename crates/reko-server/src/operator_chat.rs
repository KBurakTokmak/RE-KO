use crate::{admin, chat, lobby::Lobby, social::Hub, world_entry::Session};
use std::time::{Duration, Instant};

const MAX_MUTE_MINUTES: u64 = 60;

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
    let mut hub = lobby.social.lock().unwrap();
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
    notice(
        b"Use op announce <message>, op mute <character> <1-60 minutes>, or op unmute <character>.",
    )
}
