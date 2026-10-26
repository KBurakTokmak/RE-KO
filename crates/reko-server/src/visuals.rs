//! Replicate only server-produced visual replies, never raw skill requests or vitals.
use crate::{lobby::Lobby, world_entry::Session};
use std::cell::RefCell;

tokio::task_local! { static OUTBOX: RefCell<Vec<Vec<u8>>>; }

pub(crate) fn record(packet: &[u8]) {
    if matches!(packet.first(), Some(0x08 | 0x1b | 0x31)) {
        let _ = OUTBOX.try_with(|out| {
            let mut out = out.borrow_mut();
            if out.len() < 256 {
                out.push(packet.to_vec());
            }
        });
    }
}

pub(crate) async fn capture(future: impl std::future::Future<Output = ()>) -> Vec<Vec<u8>> {
    OUTBOX
        .scope(RefCell::new(Vec::new()), async {
            future.await;
            OUTBOX.with(|out| std::mem::take(&mut *out.borrow_mut()))
        })
        .await
}

pub(crate) fn publish(lobby: &Lobby, s: &Session, packets: Vec<Vec<u8>>) {
    if !s.entered || s.zone_loading {
        return;
    }
    let mut hub = lobby.social.lock().unwrap();
    for packet in packets {
        let target = match packet.first() {
            Some(0x31)
                if packet.len() == 22
                    && matches!(packet[1], 1..=4)
                    && u16::from_le_bytes([packet[6], packet[7]]) == s.player_id
                    && lobby.skills.as_ref().is_some_and(|c| {
                        c.spells
                            .contains_key(&u32::from_le_bytes(packet[2..6].try_into().unwrap()))
                    }) =>
            {
                Some(u16::from_le_bytes([packet[8], packet[9]]))
            }
            Some(0x08)
                if packet.len() == 7
                    && u16::from_le_bytes([packet[3], packet[4]]) == s.player_id =>
            {
                Some(u16::from_le_bytes([packet[5], packet[6]]))
            }
            Some(0x1b)
                if packet.len() >= 4
                    && u16::from_le_bytes([packet[1], packet[2]]) == s.player_id =>
            {
                None
            }
            _ => continue,
        };
        // Friendly/player-target effects are already delivered directly to the
        // recipient by their authoritative handler. Observers need the same FX.
        let direct_recipient = target.filter(|id| {
            packet.first() == Some(&0x31)
                && packet[1] == 3
                && *id != s.player_id
                && hub.players.contains_key(id)
        });
        hub.broadcast_visual(s.player_id, target, direct_recipient, &packet);
    }
}
