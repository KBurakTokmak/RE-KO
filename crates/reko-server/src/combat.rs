//! Class coefficients and original-item contributions for combat attributes.

use crate::{characters::Character, progress::Progress, starter::Item};
use reko_assets::{CellValue, Table};
use std::{collections::HashMap, path::Path};

pub(crate) fn integer(row: &[CellValue], column: usize) -> i32 {
    match row.get(column) {
        Some(CellValue::U8(v)) => i32::from(*v),
        Some(CellValue::I8(v)) => i32::from(*v),
        Some(CellValue::U16(v)) => i32::from(*v),
        Some(CellValue::I16(v)) => i32::from(*v),
        Some(CellValue::U32(v)) => *v as i32,
        Some(CellValue::I32(v)) => *v,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_equip_requirements_preserve_starter_dagger_and_gate_master_armor() {
        let game = reko_common::game_dir();
        let rules = Path::new("../../logs/reference-coefficients.csv");
        if !game.join("Data/item_org_us.tbl").exists() || !rules.exists() {
            return;
        }
        let mut catalog = Catalog::load(&game, rules).unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::loot::initialize(&db).unwrap();
        db.execute_batch("INSERT INTO world_reference_rules VALUES ('ITEM_LEVEL',110110001,'[1,80]'),('ITEM_LEVEL',246002001,'[60,80]');").unwrap();
        catalog.load_level_bounds(&db).unwrap();
        let mut c = Character {
            slot: 0,
            name: "Requirements".into(),
            race: 2,
            class: 108,
            face: 0,
            hair: 0,
            stats: [60, 150, 255, 50, 50],
        };
        assert!(
            catalog.eligible(110110001, &c, 1).is_ok(),
            "server level1 dagger rule takes precedence over unused client gear-level value"
        );
        assert!(catalog.eligible(246002001, &c, 80).is_ok());
        assert!(catalog.eligible(246002001, &c, 59).is_err());
        assert!(
            catalog.eligible(206002001, &c, 80).is_err(),
            "warrior pads cannot equip on rogue"
        );
        c.class = 208;
        c.race = 12;
        assert!(
            catalog.eligible(246002001, &c, 80).is_ok(),
            "other nation uses same class family"
        );
        c.class = 107;
        assert!(
            catalog.eligible(246002001, &c, 80).is_err(),
            "unmastered rogue cannot wear master armor"
        );
        c.class = 108;
        c.stats[2] = 65;
        assert!(
            catalog.eligible(110110001, &c, 80).is_err(),
            "base stats alone must meet requirements"
        );
        c.stats[2] = 66;
        assert!(catalog.eligible(110110001, &c, 80).is_ok());
        assert!(catalog.eligible(110110001, &c, 81).is_err());
        catalog.items.get_mut(&110110000).unwrap()[13] = CellValue::U8(2);
        c.race = 12;
        assert!(catalog.eligible(110110001, &c, 80).is_err());
        c.race = 2;
        assert!(catalog.eligible(110110001, &c, 80).is_ok());
    }

    #[test]
    fn equipment_attributes_feed_formulas_and_stack_with_timed_bonuses() {
        let mut base = vec![CellValue::I32(0); 37];
        base[1] = CellValue::I32(23);
        let mut ext = vec![CellValue::I32(0); 53];
        for (col, value) in [
            (30, 10),
            (31, 20),
            (32, 30),
            (33, 40),
            (34, 50),
            (35, 150),
            (36, 200),
            (10, 25),
            (11, 50),
            (37, 11),
            (38, 12),
            (39, 13),
            (40, 14),
            (41, 15),
            (42, 16),
        ] {
            ext[col] = CellValue::I32(value);
        }
        let catalog = Catalog {
            level_bounds: HashMap::new(),
            coefficients: HashMap::from([(103, [0.001; 14])]),
            items: HashMap::from([(330310000, base)]),
            extensions: HashMap::from([(23, HashMap::from([(1, ext)]))]),
        };
        let mut character = Character {
            slot: 0,
            name: "Bonuses".into(),
            race: 3,
            class: 103,
            face: 0,
            hair: 0,
            stats: [60; 5],
        };
        let progress = Progress {
            level: 80,
            points: [0; 9],
        };
        let empty = [Item::default(); 14];
        let mut gear = empty;
        gear[9] = Item {
            id: 330310001,
            durability: 0,
            count: 1,
        };
        let actual = catalog
            .calculate_bonuses(&character, &progress, &gear, [1, 2, 3, 4, 5])
            .unwrap();
        assert_eq!(actual.bonus_stats, [11, 22, 33, 44, 55]);
        character.stats = [71, 82, 93, 104, 115];
        let reference = catalog.calculate(&character, &progress, &empty).unwrap();
        assert_eq!(actual.hp, reference.hp + 150);
        assert_eq!(actual.mp, reference.mp + 200);
        assert_eq!(actual.max_weight, reference.max_weight);
        assert_eq!(actual.attack, reference.attack);
        assert!((actual.hit_rate - reference.hit_rate * 1.25).abs() < 0.0001);
        assert!((actual.evasion - reference.evasion * 1.5).abs() < 0.0001);
        assert_eq!(actual.resistance, [11, 12, 13, 14, 16, 15]);
    }

    #[test]
    fn master_rogue_attributes_use_original_daggers_and_reference_coefficients() {
        let game = reko_common::game_dir();
        let rules = Path::new("../../logs/reference-coefficients.csv");
        let alternative = Path::new("logs/reference-coefficients.csv");
        let rules = if rules.exists() { rules } else { alternative };
        if !game.join("Data/item_org_us.tbl").exists() || !rules.exists() {
            return;
        }
        let catalog = Catalog::load(&game, rules).unwrap();
        assert!(catalog.equip_slot(160100001, 8));
        assert!(!catalog.equip_slot(160100001, 6));
        assert!(catalog.two_handed(160100001));
        for id in [391010000, 370004000, 379064000, 379060000] {
            assert!(catalog.item(id).is_some(), "missing test item {id}");
        }
        let character = Character {
            slot: 1,
            name: "Rogue80".into(),
            race: 2,
            class: 108,
            face: 0,
            hair: 0,
            stats: [60, 150, 255, 50, 50],
        };
        let mut gear = [Item::default(); 14];
        gear[6] = Item {
            id: 110110001,
            durability: 4000,
            count: 1,
        };
        gear[8] = gear[6];
        let stats = catalog
            .calculate(
                &character,
                &Progress {
                    level: 80,
                    points: [0; 9],
                },
                &gear,
            )
            .unwrap();
        assert_eq!(
            (stats.hp, stats.mp, stats.attack, stats.defence),
            (2690, 4110, 75, 80)
        );
        assert_eq!((stats.weight, stats.max_weight), (40, 7000));
        let progress = Progress {
            level: 80,
            points: [0; 9],
        };
        gear[9] = Item {
            id: 330310014,
            durability: 1,
            count: 1,
        };
        let one_ring = catalog.calculate(&character, &progress, &gear).unwrap();
        assert_eq!(one_ring.bonus_stats, [0, 0, 8, 0, 0]);
        gear[11] = gear[9];
        let two_rings = catalog.calculate(&character, &progress, &gear).unwrap();
        assert_eq!(two_rings.bonus_stats, [0, 0, 16, 0, 0]);
        assert_eq!(two_rings.resistance, [30, 30, 30, 0, 0, 0]);
        assert!(two_rings.attack > one_ring.attack && one_ring.attack > stats.attack);
        assert!(two_rings.hit_rate > one_ring.hit_rate);
        let buffed = catalog
            .calculate_bonuses(&character, &progress, &gear, [0, 0, 15, 0, 0])
            .unwrap();
        assert_eq!(buffed.bonus_stats[2], 31);
        assert!(buffed.attack > two_rings.attack);
        gear[9] = Item::default();
        gear[11] = Item::default();
        gear[6].durability = 0;
        gear[8].durability = 0;
        let broken = catalog
            .calculate(
                &character,
                &Progress {
                    level: 80,
                    points: [0; 9],
                },
                &gear,
            )
            .unwrap();
        assert!(broken.attack < stats.attack);
        gear[6].durability = 4000;
        gear[8].durability = 4000;
        assert_eq!(
            catalog
                .calculate(
                    &character,
                    &Progress {
                        level: 80,
                        points: [0; 9]
                    },
                    &gear
                )
                .unwrap()
                .attack,
            stats.attack
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Stats {
    pub hp: u16,
    pub mp: u16,
    pub attack: u16,
    pub defence: u16,
    pub max_weight: u16,
    pub weight: u16,
    pub delay_ms: u64,
    pub range: f32,
    pub hit_rate: f32,
    pub evasion: f32,
    pub weapon_kind: u8,
    pub resistance: [u16; 6],
    pub bonus_stats: [u16; 5],
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            hp: 100,
            mp: 100,
            attack: 1,
            defence: 0,
            max_weight: 1500,
            weight: 0,
            delay_ms: 1100,
            range: 2.0,
            hit_rate: 1.0,
            evasion: 1.0,
            weapon_kind: 0,
            resistance: [0; 6],
            bonus_stats: [0; 5],
        }
    }
}

pub(crate) struct Catalog {
    level_bounds: HashMap<u32, (u8, u8)>,
    coefficients: HashMap<u16, [f64; 14]>,
    items: HashMap<u32, Vec<CellValue>>,
    extensions: HashMap<u8, HashMap<u32, Vec<CellValue>>>,
}

impl Catalog {
    pub fn load_level_bounds(&mut self, db: &rusqlite::Connection) -> anyhow::Result<()> {
        let mut q = db.prepare(
            "SELECT rule_id,data FROM world_reference_rules WHERE table_name='ITEM_LEVEL'",
        )?;
        for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? {
            let (id, data) = row?;
            self.level_bounds.insert(id, serde_json::from_str(&data)?);
        }
        Ok(())
    }
    pub fn eligible(&self, id: u32, character: &Character, level: u8) -> anyhow::Result<()> {
        let (base, Some(ext)) = self
            .item(id)
            .ok_or_else(|| anyhow::anyhow!("unknown equipment"))?
        else {
            anyhow::bail!("unknown equipment extension");
        };
        let race = integer(base, 13);
        anyhow::ensure!(
            race == 0
                || race == i32::from(character.race)
                || (matches!(integer(base, 12), 105 | 107) && matches!(race, 20 | 30)),
            "equipment race requirement"
        );
        let required = integer(base, 14);
        let class = i32::from(character.class % 100);
        let class_ok = match required {
            0 => true,
            1 => matches!(class, 1 | 5 | 6),
            2 => matches!(class, 2 | 7 | 8),
            3 => matches!(class, 3 | 9 | 10),
            4 => matches!(class, 4 | 11 | 12),
            5..=12 => class == required,
            _ => i32::from(character.class) == required,
        };
        anyhow::ensure!(class_ok, "equipment class requirement");
        // Gear level bounds are server ITEM rules; native UI does not enforce
        // its different ItemOrg level value on ordinary equipment.
        if let Some(&(minimum, maximum)) = self.level_bounds.get(&id) {
            anyhow::ensure!(
                (minimum..=maximum).contains(&level),
                "equipment level requirement"
            );
        }
        for ((&stat, column), ext_column) in character.stats.iter().zip(30..35).zip(48..53) {
            let base_need = integer(base, column);
            let need = if base_need == 0 {
                0
            } else {
                base_need + integer(ext, ext_column)
            };
            anyhow::ensure!(i32::from(stat) >= need, "equipment stat requirement");
        }
        // Rank/title are currently sent as zero; they cannot grant eligibility.
        anyhow::ensure!(
            integer(base, 28) + integer(ext, 46) <= 0 && integer(base, 29) + integer(ext, 47) <= 0,
            "equipment rank/title requirement"
        );
        Ok(())
    }
    pub fn equip_slot(&self, id: u32, slot: usize) -> bool {
        let Some((row, Some(_))) = self.item(id) else {
            return false;
        };
        match integer(row, 12) {
            0 => matches!(slot, 6 | 8),
            1 | 3 => slot == 6,
            2 | 4 => slot == 8,
            5 => slot == 4,
            6 => slot == 10,
            7 => slot == 1,
            8 => slot == 12,
            9 => slot == 13,
            10 => matches!(slot, 0 | 2),
            11 => slot == 3,
            12 => matches!(slot, 9 | 11),
            13 => slot == 5,
            14 => slot == 7,
            _ => false,
        }
    }

    pub fn two_handed(&self, id: u32) -> bool {
        self.item(id)
            .is_some_and(|(row, _)| matches!(integer(row, 12), 3 | 4))
    }

    pub fn equipment_mask(&self, id: u32, slot: usize) -> Option<u16> {
        if !self.equip_slot(id, slot) {
            return None;
        }
        Some(if self.two_handed(id) {
            (1 << 6) | (1 << 8)
        } else {
            1 << slot
        })
    }

    pub fn bag_weight(&self, bag: &[Item]) -> u16 {
        bag.iter()
            .filter_map(|item| {
                self.item(item.id)
                    .map(|(row, _)| integer(row, 18).max(0) as u32 * u32::from(item.count))
            })
            .sum::<u32>()
            .min(32767) as u16
    }

    pub fn load(game: &Path, rules: &Path) -> anyhow::Result<Self> {
        let mut coefficients = HashMap::new();
        for line in std::fs::read_to_string(rules)?
            .lines()
            .filter(|line| !line.trim().is_empty())
        {
            let cells: Vec<_> = line
                .trim()
                .trim_start_matches('\u{feff}')
                .split(',')
                .collect();
            anyhow::ensure!(cells.len() == 15, "invalid coefficient record");
            let class = cells[0].parse()?;
            let mut values = [0.0; 14];
            for (out, input) in values.iter_mut().zip(&cells[1..]) {
                *out = input.parse()?;
            }
            coefficients.insert(class, values);
        }
        let table =
            Table::from_encrypted_bytes(&std::fs::read(game.join("Data/item_org_us.tbl"))?)?;
        let items = table
            .rows
            .into_iter()
            .map(|row| (integer(&row, 0) as u32, row))
            .collect::<HashMap<_, _>>();
        let mut extensions = HashMap::new();
        for row in items.values() {
            let group = integer(row, 1) as u8;
            if extensions.contains_key(&group) {
                continue;
            }
            let file = game.join(format!("Data/Item_Ext_{group}_us.tbl"));
            if file.exists() {
                let table = Table::from_encrypted_bytes(&std::fs::read(file)?)?;
                extensions.insert(
                    group,
                    table
                        .rows
                        .into_iter()
                        .map(|row| (integer(&row, 0) as u32, row))
                        .collect(),
                );
            }
        }
        Ok(Self {
            level_bounds: HashMap::new(),
            coefficients,
            items,
            extensions,
        })
    }

    pub(crate) fn item(&self, id: u32) -> Option<(&[CellValue], Option<&[CellValue]>)> {
        let base = self.items.get(&(id / 1000 * 1000))?;
        let extension = self
            .extensions
            .get(&(integer(base, 1) as u8))
            .and_then(|table| table.get(&(id % 1000)));
        Some((base, extension.map(Vec::as_slice)))
    }

    pub(crate) fn item_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.items.iter().flat_map(|(id, base)| {
            self.extensions
                .get(&(integer(base, 1) as u8))
                .into_iter()
                .flat_map(move |table| {
                    table
                        .keys()
                        .filter(|ext| **ext < 1000)
                        .map(move |ext| id + ext)
                })
        })
    }

    pub fn calculate(
        &self,
        character: &Character,
        progress: &Progress,
        gear: &[Item; 14],
    ) -> anyhow::Result<Stats> {
        self.calculate_bonuses(character, progress, gear, [0; 5])
    }

    pub fn calculate_bonuses(
        &self,
        character: &Character,
        progress: &Progress,
        gear: &[Item; 14],
        mut bonuses: [u16; 5],
    ) -> anyhow::Result<Stats> {
        let c = self
            .coefficients
            .get(&character.class)
            .ok_or_else(|| anyhow::anyhow!("missing coefficient class {}", character.class))?;
        let mut item_hp = 0u32;
        let mut item_mp = 0u32;
        let mut resistance = [0u16; 6];
        let mut accuracy = 100u32;
        let mut evasion = 100u32;
        for item in gear.iter().filter(|item| item.id != 0) {
            let (_, extension) = self
                .item(item.id)
                .ok_or_else(|| anyhow::anyhow!("unresolved equipped item {}", item.id))?;
            if let Some(extension) = extension {
                for (bonus, column) in bonuses.iter_mut().zip(30..35) {
                    *bonus = bonus.saturating_add(integer(extension, column).max(0) as u16);
                }
                item_hp += integer(extension, 35).max(0) as u32;
                item_mp += integer(extension, 36).max(0) as u32;
                accuracy += integer(extension, 10).max(0) as u32;
                evasion += integer(extension, 11).max(0) as u32;
                // Wire order ends in curse/poison; Item_Ext ends in poison/curse.
                for (value, column) in resistance.iter_mut().zip([37, 38, 39, 40, 42, 41]) {
                    *value = value.saturating_add(integer(extension, column).max(0) as u16);
                }
            }
        }
        let level = f64::from(progress.level);
        let [strength, stamina, dexterity, intelligence, _] =
            std::array::from_fn(|i| f64::from(character.stats[i]) + f64::from(bonuses[i]));
        let rogue = matches!(character.class % 100, 2 | 7 | 8);
        let mut damage = 0;
        let mut kind = 0;
        let mut armor = 0;
        let mut weight = 0;
        let mut delay_ms = 1100;
        let mut range = 2.0;
        for (slot, item) in gear.iter().enumerate().filter(|(_, item)| item.id != 0) {
            let Some((base, extension)) = self.item(item.id) else {
                anyhow::bail!("unresolved equipped item {}", item.id);
            };
            let extra = |column| extension.map_or(0, |row| integer(row, column));
            weight += integer(base, 18).max(0);
            let condition = if item.durability == 0 { 2 } else { 1 };
            armor += (integer(base, 22).max(0) + extra(14).max(0)) / condition;
            if matches!(slot, 6 | 8) {
                let attack = (integer(base, 15).max(0) + extra(8).max(0)) / condition;
                let weapon_kind = integer(base, 10) as u8;
                let bow = matches!(weapon_kind / 10, 7 | 8 | 10);
                if slot == 6 || bow {
                    kind = weapon_kind;
                    damage = attack;
                    delay_ms = (integer(base, 16).max(1) as u64) * 10;
                    range = integer(base, 17).max(10) as f32 / 10.0;
                } else if weapon_kind / 10 != 6 {
                    damage += attack / 2;
                }
            }
        }
        damage = damage.max(3);
        let coefficient = match kind / 10 {
            1 => c[0],
            2 => c[1],
            3 => c[2],
            4 | 5 => c[3],
            6 => c[4],
            7 | 8 | 10 => c[7],
            11 => c[6],
            _ => 0.0,
        };
        let ap_stat = if rogue { dexterity } else { strength };
        let extra_ap = if rogue {
            3.0
        } else {
            3.0 + (strength - 150.0).max(0.0) - if strength == 160.0 { 1.0 } else { 0.0 }
        };
        let ap = (0.005 * f64::from(damage) * (ap_stat + 40.0)
            + coefficient * f64::from(damage) * level * ap_stat)
            .trunc()
            + extra_ap;
        let hp = (c[8] * level * level * stamina
            + 0.1 * level * stamina
            + (stamina / 5.0).trunc()
            + 20.0
            + f64::from(item_hp))
        .clamp(1.0, 14000.0) as u16;
        let mp = if c[9] != 0.0 {
            let intellect = intelligence + 30.0;
            c[9] * level * level * intellect
                + 0.2 * level * intellect
                + (intellect / 5.0).trunc()
                + 20.0
        } else {
            c[10] * level * level * stamina + 0.1 * level * stamina + (stamina / 5.0).trunc()
        };
        Ok(Stats {
            hp,
            mp: (mp + f64::from(item_mp)).clamp(1.0, 32767.0) as u16,
            attack: ap.clamp(1.0, 32767.0) as u16,
            defence: (c[11] * (level + f64::from(armor))).clamp(0.0, 32767.0) as u16,
            max_weight: ((strength + level) * 50.0).min(32767.0) as u16,
            weight: weight.min(32767) as u16,
            delay_ms,
            range,
            hit_rate: ((1.0 + c[12] * level * dexterity) * f64::from(accuracy) / 100.0) as f32,
            evasion: ((1.0 + c[13] * level * dexterity) * f64::from(evasion) / 100.0) as f32,
            weapon_kind: kind,
            resistance,
            bonus_stats: bonuses,
        })
    }
}
