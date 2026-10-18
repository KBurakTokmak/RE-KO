//! v1453 chat framing; party/clan routing uses the shared social registry.

use crate::{movement::Position, world_entry::Session};

#[derive(Clone)]
pub(crate) struct Message {
    pub channel: u8,
    pub nation: u8,
    pub position: Position,
    pub packet: Vec<u8>,
}

impl Message {
    pub fn visible_to(&self, session: &Session) -> bool {
        if !session.entered
            || self.position.zone != session.position.zone
            || self.nation != session.character.nation()
        {
            return false;
        }
        if self.channel == 5 {
            return true;
        }
        let dx = f32::from(self.position.x) - f32::from(session.position.x);
        let dz = f32::from(self.position.z) - f32::from(session.position.z);
        dx.hypot(dz) <= 500.0
    }
}

pub(crate) fn parse(payload: &[u8]) -> Option<(u8, &[u8])> {
    if payload.len() < 4 || payload[0] != 0x10 || !matches!(payload[1], 1 | 3 | 5 | 6 | 13) {
        return None;
    }
    let len = u16::from_le_bytes([payload[2], payload[3]]) as usize;
    if len == 0 || len > 256 || payload.len() != len + 4 {
        return None;
    }
    let text = &payload[4..];
    if text.iter().any(|byte| *byte < 0x20 || *byte == 0x7F) {
        return None;
    }
    Some((payload[1], text))
}

pub(crate) fn packet(channel: u8, nation: u8, player_id: u16, name: &str, text: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x10, channel, nation];
    packet.extend_from_slice(&player_id.to_le_bytes());
    packet.push(name.len() as u8);
    packet.extend_from_slice(name.as_bytes());
    packet.extend_from_slice(&(text.len() as u16).to_le_bytes());
    packet.extend_from_slice(text);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_request_has_distinct_server_sender_fields() {
        let request = b"\x10\x01\x09\x00asdasdasd";
        assert_eq!(parse(request), Some((1, &b"asdasdasd"[..])));
        assert_eq!(
            packet(1, 1, 7, "admin", b"test"),
            b"\x10\x01\x01\x07\x00\x05admin\x04\x00test"
        );
        for end in 0..request.len() {
            assert!(parse(&request[..end]).is_none());
        }
    }
}
