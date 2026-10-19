//! Native world-entry packets and authoritative character session state.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use reko_assets::{CellValue, Table};

use crate::characters::Character;

static NEXT_PLAYER: AtomicU32 = AtomicU32::new(1);

pub(crate) struct Bootstrap {
    pub next_level_exp: u32,
}

impl Bootstrap {
    pub fn load(game_dir: &Path) -> anyhow::Result<Self> {
        let path = game_dir.join("Data/Player_experience.tbl");
        let table = Table::from_encrypted_bytes(&std::fs::read(path)?)?;
        let number = |cell: &CellValue| -> Option<u32> {
            match cell {
                CellValue::U32(value) => Some(*value),
                CellValue::I32(value) => (*value).try_into().ok(),
                CellValue::I64(value) => (*value).try_into().ok(),
                CellValue::U64(value) => (*value).try_into().ok(),
                _ => None,
            }
        };
        let next_level_exp = table
            .rows
            .iter()
            .find_map(|row| {
                if row.len() >= 2 && number(&row[0]) == Some(1) {
                    number(&row[1])
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("missing level-one experience entry"))?;
        Ok(Self { next_level_exp })
    }
}

#[derive(Clone)]
pub(crate) struct Session {
    pub clan: crate::clans::Info,
    pub is_admin: bool,
    pub rewards: crate::quest_rewards::Totals,
    pub max_exp: u32,
    pub gate: Option<crate::travel::Gate>,
    pub zone_loading: bool,
    pub visible_entities: std::collections::HashSet<u16>,
    pub entity_region: Option<(u8, u16, u16)>,
    pub merchant: Option<u16>,
    pub npc_dialog: Option<crate::npc_scripts::Context>,
    pub anvil: Option<(u16, u16, crate::movement::Position)>,
    pub stat_points: u16,
    pub character: Character,
    pub player_id: u16,
    pub snapshot_sent: bool,
    pub entered: bool,
    pub position: crate::movement::Position,
    pub movement: crate::movement::Validator,
    pub equipment: [crate::starter::Item; 14],
    pub inventory: [crate::starter::Item; 28],
    pub coins: u32,
    pub skills: crate::skills::State,
    pub sitting: bool,
    pub next_regen: u64,
    pub progress: crate::progress::Progress,
    pub combat: crate::combat::Stats,
    pub base_combat: crate::combat::Stats,
    pub next_attack: u64,
    pub pending_cast: Option<crate::magic::Cast>,
    pub magic_projectiles: std::collections::HashMap<u32, crate::mage::Flight>,
    pub arrows: std::collections::HashMap<u32, crate::archery::Volley>,
}

impl Session {
    pub fn new(character: Character) -> Option<Self> {
        let player_id = NEXT_PLAYER.fetch_add(1, Ordering::Relaxed);
        if player_id >= 10000 {
            return None;
        }
        Some(Self {
            clan: crate::clans::Info::default(),
            is_admin: false,
            rewards: crate::quest_rewards::Totals::default(),
            max_exp: 0,
            gate: None,
            zone_loading: false,
            visible_entities: std::collections::HashSet::new(),
            entity_region: None,
            merchant: None,
            npc_dialog: None,
            anvil: None,
            stat_points: 0,
            character,
            player_id: player_id as u16,
            snapshot_sent: false,
            entered: false,
            position: crate::movement::Position::default(),
            movement: crate::movement::Validator::default(),
            equipment: [crate::starter::Item::default(); 14],
            inventory: [crate::starter::Item::default(); 28],
            coins: 0,
            skills: crate::skills::State::default(),
            sitting: false,
            next_regen: 0,
            progress: crate::progress::Progress::default(),
            combat: crate::combat::Stats::default(),
            base_combat: crate::combat::Stats::default(),
            next_attack: 0,
            pending_cast: None,
            magic_projectiles: std::collections::HashMap::new(),
            arrows: std::collections::HashMap::new(),
        })
    }

    pub fn select_response(&self) -> Vec<u8> {
        let mut packet = vec![0x04, 1, self.position.zone];
        put_u16(&mut packet, self.position.x);
        put_u16(&mut packet, self.position.z);
        put_u16(&mut packet, self.position.y as u16);
        packet.push(0);
        packet
    }

    pub fn zone_ability(&self) -> Option<[u8; 7]> {
        match self.position.zone {
            21 => Some([0x5e, 1, 1, 0, 1, 10, 0]),
            // TODO(re): Load trade, hostility and tariff rules for other zones.
            _ => None,
        }
    }

    pub fn my_info(&self, bootstrap: &Bootstrap) -> Vec<u8> {
        let mut packet = vec![0x0E];
        put_u16(&mut packet, self.player_id);
        packet.push(self.character.name.len() as u8);
        packet.extend_from_slice(self.character.name.as_bytes());
        // TODO(re): Establish the original spawn, HP/MP, weight, and combat formulas.
        for value in [self.position.x, self.position.z, self.position.y as u16] {
            put_u16(&mut packet, value);
        }
        packet.extend_from_slice(&[self.character.nation(), self.character.race]);
        put_u16(&mut packet, self.character.class);
        packet.extend_from_slice(&[
            self.character.face,
            self.character.hair,
            0,
            0,
            self.progress.level,
        ]);
        put_u16(&mut packet, self.stat_points);
        put_u32(
            &mut packet,
            if self.max_exp > 0 {
                self.max_exp
            } else {
                bootstrap.next_level_exp
            },
        );
        put_u32(&mut packet, self.rewards.experience);
        put_u32(&mut packet, self.rewards.loyalty);
        put_u32(&mut packet, self.rewards.monthly);
        packet.push(0);
        put_u16(&mut packet, self.clan.id);
        packet.push(self.clan.fame);
        put_u16(&mut packet, 0);
        packet.extend_from_slice(&[self.clan.rank, self.clan.name.len() as u8]);
        packet.extend_from_slice(self.clan.name.as_bytes());
        packet.extend_from_slice(&[self.clan.grade, 0]);
        put_u16(&mut packet, 0);
        put_u16(&mut packet, self.clan.cape);
        for value in [
            self.skills.max_hp,
            self.skills.hp,
            self.skills.max_mana,
            self.skills.mana,
            self.combat.max_weight,
            self.combat.weight,
        ] {
            put_u16(&mut packet, value);
        }
        for (&stat, bonus) in self.character.stats.iter().zip(self.combat.bonus_stats) {
            packet.extend_from_slice(&[stat, bonus.min(255) as u8]);
        }
        put_u16(&mut packet, self.combat.attack);
        put_u16(&mut packet, self.combat.defence);
        for value in self.combat.resistance {
            packet.push(value.min(255) as u8);
        }
        put_u32(&mut packet, self.coins);
        packet.extend_from_slice(&[u8::from(!self.is_admin), 0, 0]);
        packet.extend_from_slice(&self.progress.points);
        for item in self.equipment {
            put_u32(&mut packet, item.id);
            put_u16(&mut packet, item.durability);
            put_u16(&mut packet, item.count);
            packet.extend_from_slice(&[0; 11]);
        }
        packet.extend_from_slice(&[0; 2 * 15]);
        for item in self.inventory {
            put_u32(&mut packet, item.id);
            put_u16(&mut packet, item.durability);
            put_u16(&mut packet, item.count);
            packet.extend_from_slice(&[0; 11]);
        }
        packet.push(0);
        packet.push(0);
        put_u16(&mut packet, 0);
        packet.push(0);
        put_u32(&mut packet, 0);
        packet
    }
}

fn put_u16(packet: &mut Vec<u8>, value: u16) {
    packet.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(packet: &mut Vec<u8>, value: u32) {
    packet.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn selected_name(payload: &[u8]) -> Option<&str> {
    if payload.first() != Some(&4) || payload.len() < 7 {
        return None;
    }
    let account_len = u16::from_le_bytes([payload[1], payload[2]]) as usize;
    let start = 3usize.checked_add(account_len)?;
    let length_bytes = payload.get(start..start + 2)?;
    let name_len = u16::from_le_bytes([length_bytes[0], length_bytes[1]]) as usize;
    let name_start = start + 2;
    if name_len == 0 || name_len > 20 || payload.len() != name_start + name_len + 2 {
        return None;
    }
    std::str::from_utf8(payload.get(name_start..name_start + name_len)?).ok()
}

pub(crate) fn start_matches(payload: &[u8], session: &Session) -> bool {
    payload.len() == session.character.name.len() + 3
        && payload[..3] == [0x0D, 1, session.character.name.len() as u8]
        && payload[3..] == *session.character.name.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_sends_equipment_bonuses_beside_base_stats() {
        let mut player = session();
        let normal = player.my_info(&Bootstrap { next_level_exp: 50 });
        player.combat.bonus_stats = [1, 2, 16, 4, 300];
        let with_bonuses = player.my_info(&Bootstrap { next_level_exp: 50 });
        let changes: Vec<_> = normal
            .iter()
            .zip(&with_bonuses)
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some((i, *b)))
            .collect();
        assert_eq!(changes, [(74, 1), (76, 2), (78, 16), (80, 4), (82, 255)]);
    }

    #[test]
    fn admin_entry_changes_only_the_native_authority_byte() {
        let mut player = session();
        let normal = player.my_info(&Bootstrap { next_level_exp: 50 });
        player.is_admin = true;
        let admin = player.my_info(&Bootstrap { next_level_exp: 50 });
        let changed: Vec<_> = normal
            .iter()
            .zip(&admin)
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect();
        assert_eq!(changed, [97]);
        assert_eq!((normal[97], admin[97]), (1, 0));
    }

    fn session() -> Session {
        Session {
            clan: crate::clans::Info::default(),
            is_admin: false,
            rewards: crate::quest_rewards::Totals::default(),
            max_exp: 0,
            gate: None,
            zone_loading: false,
            visible_entities: std::collections::HashSet::new(),
            entity_region: None,
            merchant: None,
            npc_dialog: None,
            anvil: None,
            stat_points: 0,
            character: Character {
                slot: 0,
                name: "AdminSiken".into(),
                race: 2,
                class: 102,
                face: 0,
                hair: 0,
                stats: [60, 60, 80, 50, 50],
            },
            player_id: 7,
            snapshot_sent: false,
            entered: false,
            position: crate::movement::Position::default(),
            movement: crate::movement::Validator::default(),
            equipment: [crate::starter::Item::default(); 14],
            inventory: [crate::starter::Item::default(); 28],
            coins: 0,
            skills: crate::skills::State::default(),
            sitting: false,
            next_regen: 0,
            progress: crate::progress::Progress::default(),
            combat: crate::combat::Stats::default(),
            base_combat: crate::combat::Stats::default(),
            next_attack: 0,
            pending_cast: None,
            magic_projectiles: std::collections::HashMap::new(),
            arrows: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn selection_and_start_match_captured_names() {
        let mut selection = vec![4, 0, 0, 10, 0];
        selection.extend_from_slice(b"AdminSiken");
        selection.extend_from_slice(&[1, 21]);
        assert_eq!(selected_name(&selection), Some("AdminSiken"));
        for end in 0..selection.len() {
            assert_eq!(selected_name(&selection[..end]), None);
        }
        let mut start = vec![0x0D, 1, 10];
        start.extend_from_slice(b"AdminSiken");
        assert!(start_matches(&start, &session()));
        start[3] = b'x';
        assert!(!start_matches(&start, &session()));
    }

    #[test]
    fn promoted_clan_type_grade_and_saved_cape_match_native_myinfo_fields() {
        let mut s = session();
        s.clan = crate::clans::Info {
            id: 42,
            name: "Capes".into(),
            fame: 1,
            grade: 3,
            rank: 2,
            cape: 4,
        };
        let p = s.my_info(&Bootstrap { next_level_exp: 50 });
        assert_eq!(
            &p[48..66],
            b"\x2a\x00\x01\x00\x00\x02\x05Capes\x03\x00\x00\x00\x04\x00"
        );
    }

    #[test]
    fn equipped_weapon_and_noah_are_in_the_full_player_packet() {
        let mut player = session();
        player.equipment[6] = crate::starter::Item {
            id: 110110001,
            durability: 4000,
            count: 1,
        };
        player.coins = 20000;
        let packet = player.my_info(&Bootstrap { next_level_exp: 50 });
        assert_eq!(&packet[93..97], &20000u32.to_le_bytes());
        assert_eq!(&packet[223..227], &110110001u32.to_le_bytes());
        assert_eq!(&packet[227..229], &4000u16.to_le_bytes());
        assert_eq!(&packet[229..231], &1u16.to_le_bytes());
        assert_eq!(packet.len(), 946);
    }

    #[test]
    fn level_one_experience_matches_original_table_when_available() {
        let game_dir = reko_common::game_dir();
        if !game_dir.join("Data/Player_experience.tbl").exists() {
            return;
        }
        assert_eq!(Bootstrap::load(&game_dir).unwrap().next_level_exp, 50);
    }

    #[test]
    fn snapshot_matches_recovered_parser_widths() {
        let packet = session().my_info(&Bootstrap { next_level_exp: 50 });
        let mut position = 0;
        let mut read = |count: usize| {
            let part = &packet[position..position + count];
            position += count;
            part
        };
        assert_eq!(read(1), [0x0E]);
        assert_eq!(read(2), 7u16.to_le_bytes());
        assert_eq!(read(1), [10]);
        assert_eq!(read(10), b"AdminSiken");
        assert_eq!(read(6), [0x80, 0x0C, 0xA0, 0x0F, 0, 0]);
        assert_eq!(read(2), [1, 2]);
        assert_eq!(read(2), 102u16.to_le_bytes());
        assert_eq!(read(5), [0, 0, 0, 0, 1]);
        assert_eq!(read(2), [0, 0]);
        assert_eq!(read(4), 50u32.to_le_bytes());
        assert_eq!(read(12), [0; 12]);
        assert_eq!(read(1 + 2 + 1 + 2 + 1), [0; 7]);
        assert_eq!(read(1), [0]);
        assert_eq!(read(2 + 2 + 2), [0; 6]);
        assert_eq!(read(12), [100, 0, 100, 0, 100, 0, 100, 0, 0xDC, 5, 0, 0]);
        assert_eq!(read(10), [60, 0, 60, 0, 80, 0, 50, 0, 50, 0]);
        read(4 + 6 + 4 + 3 + 9);
        for _ in 0..14 {
            assert_eq!(read(19), [0; 19]);
        }
        for _ in 0..2 {
            assert_eq!(read(15), [0; 15]);
        }
        for _ in 0..28 {
            assert_eq!(read(19), [0; 19]);
        }
        assert_eq!(read(9), [0; 9]);
        assert_eq!(position, packet.len());
    }
}
