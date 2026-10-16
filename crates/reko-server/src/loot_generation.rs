//! Legacy MAKE_* procedural item number construction.
use crate::loot::roll;
use rusqlite::Connection;
use std::collections::HashMap;

pub(crate) struct Generator {
    weights: HashMap<(u8, u32), Vec<u32>>,
}
fn choose(weights: &[u32], value: u32) -> Option<u32> {
    let mut sum = 0;
    for (i, &weight) in weights.iter().enumerate() {
        if value >= sum && value < sum + weight {
            return Some(i as u32 + 1);
        }
        sum += weight;
    }
    None
}
impl Generator {
    pub fn load(db: &Connection) -> anyhow::Result<Self> {
        let mut weights = HashMap::new();
        for (kind, table, columns) in [
            (
                0,
                "MAKE_ITEM_GRADECODE",
                (1..=9).map(|i| format!("byGrade_{i}")).collect::<Vec<_>>(),
            ),
            (
                1,
                "MAKE_WEAPON",
                (1..=12).map(|i| format!("sClass_{i}")).collect(),
            ),
            (
                2,
                "MAKE_DEFENSIVE",
                (1..=7).map(|i| format!("sClass_{i}")).collect(),
            ),
            (
                3,
                "MAKE_ITEM_LARECODE",
                ["sLareItem", "sMagicItem", "sGereralItem"]
                    .map(str::to_owned)
                    .to_vec(),
            ),
        ] {
            let sql = format!(
                "SELECT rule_id,{} FROM world_reference_rules WHERE table_name=?1",
                columns
                    .iter()
                    .map(|c| format!("CAST(json_extract(data,'$.{c}') AS INTEGER)"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            for row in db.prepare(&sql)?.query_map([table], |r| {
                let values = (1..=columns.len())
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<u32>>>()?;
                Ok((r.get(0)?, values))
            })? {
                let (key, value) = row?;
                weights.insert((kind, key), value);
            }
        }
        Ok(Self { weights })
    }
    fn weighted(&self, kind: u8, key: u32, value: u32) -> Option<u32> {
        choose(self.weights.get(&(kind, key))?, value)
    }
    fn rarity(&self, level: u32, kind: u8, seed: &mut u64) -> Option<u32> {
        let weights = self.weights.get(&(3, level))?;
        let rarity = choose(weights, roll(seed, 0, 1000)).unwrap_or(0);
        Some(match (rarity, kind) {
            (1, 1) => roll(seed, 16, 24),
            (1, 2) => roll(seed, 12, 24),
            (2, 1) => roll(seed, 6, 15),
            (2, 2) => roll(seed, 6, 11),
            (1..=3, 3) => roll(seed, 0, 10),
            (3, 1 | 2) => 5,
            _ => 0,
        })
    }
    pub fn generate(&self, group: u32, level: u8, nation: u8, seed: &mut u64) -> u32 {
        let category = roll(seed, 1, 10000);
        let Some(grade) = self.weighted(0, group, roll(seed, 1, 1000)) else {
            return 0;
        };
        let level = u32::from(level);
        if category <= 4000 {
            let weapon = choose(
                &[700, 700, 700, 700, 700, 2000, 1000, 2000, 1500],
                roll(seed, 1, 10000) - 1,
            )
            .unwrap();
            let Some(class) = self.weighted(1, level / 10, roll(seed, 0, 1000)) else {
                return 0;
            };
            let nation = if roll(seed, 1, 10000) < 5000 {
                10000
            } else {
                50000
            };
            let hands = if roll(seed, 1, 10000) < 5000 {
                0
            } else {
                5000000
            };
            let Some(rarity) = self.rarity(level / 5, 1, seed) else {
                return 0;
            };
            100000000 + class * 100000 + weapon * 10000000 + nation + hands + rarity * 10 + grade
        } else if category <= 8000 {
            let Some(class) = self.weighted(2, level / 10, roll(seed, 0, 1000)) else {
                return 0;
            };
            let r = roll(seed, 0, 10000);
            let (job, race) = match nation {
                1 => match r {
                    0..2000 => (0, 10000),
                    2000..4000 => (40000000, 20000),
                    4000..6000 => (60000000, 30000),
                    _ => (
                        80000000,
                        if roll(seed, 0, 10000) < 5000 {
                            20000
                        } else {
                            40000
                        },
                    ),
                },
                2 => match r {
                    0..3300 => (
                        0,
                        match roll(seed, 0, 10000) {
                            0..3333 => 110000,
                            3333..6666 => 120000,
                            _ => 130000,
                        },
                    ),
                    3300..5600 => (
                        40000000,
                        if roll(seed, 0, 10000) < 5000 {
                            120000
                        } else {
                            130000
                        },
                    ),
                    5600..7800 => (
                        60000000,
                        if roll(seed, 0, 10000) < 5000 {
                            120000
                        } else {
                            130000
                        },
                    ),
                    _ => (
                        80000000,
                        if roll(seed, 0, 10000) < 5000 {
                            120000
                        } else {
                            130000
                        },
                    ),
                },
                _ => return 0,
            };
            let part = (roll(seed, 0, 10000) / 2000 + 1).min(5) * 1000;
            let Some(rarity) = self.rarity(level / 5, 2, seed) else {
                return 0;
            };
            200000000 + job + class * 1000000 + race + part + rarity * 10 + grade
        } else {
            let kind = (roll(seed, 0, 10000) / 2500 + 1).min(4) * 10000000;
            let nation = if roll(seed, 1, 10000) < 5000 {
                110000
            } else {
                150000
            };
            let Some(rarity) = self.rarity(level / 5, 3, seed) else {
                return 0;
            };
            300000000 + kind + nation + rarity * 10 + grade
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_intervals_are_half_open_and_keep_empty_weight() {
        assert_eq!(choose(&[800, 200, 0], 0), Some(1));
        assert_eq!(choose(&[800, 200, 0], 799), Some(1));
        assert_eq!(choose(&[800, 200, 0], 800), Some(2));
        assert_eq!(choose(&[800, 200, 0], 999), Some(2));
        assert_eq!(choose(&[800, 200, 0], 1000), None);
    }
}
