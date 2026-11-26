//! v1453 chat framing; party/clan routing uses the shared social registry.

use crate::{movement::Position, world_entry::Session};

pub(crate) const MERCHANT_CHANNEL: u8 = 14;

#[derive(Clone)]
pub(crate) struct Message {
    pub channel: u8,
    pub position: Position,
    pub packet: Vec<u8>,
}

impl Message {
    pub fn visible_to(&self, session: &Session) -> bool {
        if !session.entered || self.position.zone != session.position.zone {
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
    if payload.len() < 4
        || payload[0] != 0x10
        || !matches!(payload[1], 1 | 2 | 3 | 5 | 6 | 13 | MERCHANT_CHANNEL)
    {
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

    #[test]
    fn parses_native_merchant_advert_with_coordinates() {
        assert_eq!(
            parse(b"\x10\x0e\x17\x00test(Location:992,1051)"),
            Some((MERCHANT_CHANNEL, b"test(Location:992,1051)".as_slice()))
        );
    }

    #[test]
    fn all_chat_and_shout_reach_opposite_nation_in_same_zone() {
        let mut elmo = Session::new(crate::characters::Character {
            slot: 0,
            name: "ElmoPeer".into(),
            race: 11,
            class: 206,
            face: 0,
            hair: 0,
            stats: [60; 5],
        })
        .unwrap();
        elmo.entered = true;
        elmo.position = Position {
            zone: 21,
            x: 3200,
            z: 4000,
            y: 0,
        };
        let all = Message {
            channel: 1,
            position: Position {
                zone: 21,
                x: 3300,
                z: 4000,
                y: 0,
            },
            packet: packet(1, 1, 7, "KarusPeer", b"hello"),
        };
        assert!(all.visible_to(&elmo));
        let far = Message {
            position: Position {
                zone: 21,
                x: 3800,
                z: 4000,
                y: 0,
            },
            ..all.clone()
        };
        assert!(!far.visible_to(&elmo));
        let shout = Message { channel: 5, ..far };
        assert!(shout.visible_to(&elmo));
        let other_zone = Message {
            position: Position {
                zone: 1,
                x: 3200,
                z: 4000,
                y: 0,
            },
            ..all
        };
        assert!(!other_zone.visible_to(&elmo));
    }
}
