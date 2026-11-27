//! Monster spells: original table data and known 1453 NPC magic rules.
//!
//! Monsters cast only their first magic slot. `byMagicAttack` 2 rolls a ~10%
//! single-target cast per melee swing, 4/5 roll the same cast as an area centred
//! on the victim, and `byDirectAttack` 1/2 cast at range. Monsters pay no mana
//! and ignore recast times; their attack delay paces casting. Second/third
//! slots and monster heals are never used, matching known 1453 server behavior.
use crate::{combat, skills};
use rusqlite::Connection;
use std::collections::HashMap;

/// Type4 debuff carried by a monster spell's second effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Debuff {
    pub category: u8,
    pub duration_ms: u64,
    pub speed: u16,
}

/// One castable monster spell resolved from the original client tables.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Spell {
    pub id: u32,
    /// 7 single enemy, 10 enemy area, 0 unset.
    pub moral: u8,
    pub cast_ms: u64,
    /// Metres; 0 means unlimited (the cast still needs the attack range).
    pub range: f32,
    /// Metres around the centre; 0 is single target.
    pub radius: f32,
    pub direct: u8,
    pub first: i16,
    pub over_time: i16,
    pub duration_s: u16,
    pub element: u8,
    pub debuff: Option<Debuff>,
}

impl Spell {
    pub fn resolve(catalog: &skills::Catalog, id: u32) -> Option<Self> {
        let spell = catalog.spells.get(&id)?;
        // Only enemy-targeted morals (single 7, area 10, unset 0) act on players;
        // self/friendly spells such as heals must never reach a player.
        if !matches!(spell.moral, 0 | 7 | 10) {
            return None;
        }
        let debuff = |table: u8| {
            catalog.effects.get(&(4, id)).and_then(|row| {
                let category = combat::integer(row, 1).clamp(0, 255) as u8;
                (table == 4 && category != 0).then(|| Debuff {
                    category,
                    duration_ms: combat::integer(row, 3).max(0) as u64 * 1000,
                    speed: combat::integer(row, 5).clamp(0, 200) as u16,
                })
            })
        };
        match spell.types {
            [3, second] => {
                let row = catalog.effects.get(&(3, id))?;
                Some(Self {
                    id,
                    moral: spell.moral,
                    cast_ms: spell.cast_ms,
                    range: f32::from(spell.range),
                    radius: combat::integer(row, 1).max(0) as f32,
                    direct: combat::integer(row, 2).clamp(0, 255) as u8,
                    first: combat::integer(row, 3).clamp(-32767, 32767) as i16,
                    over_time: combat::integer(row, 4).clamp(-32767, 32767) as i16,
                    duration_s: combat::integer(row, 5).clamp(0, 32767) as u16,
                    element: combat::integer(row, 6).clamp(0, 6) as u8,
                    debuff: debuff(second),
                })
            }
            [4, _] => debuff(4).map(|d| Self {
                id,
                moral: spell.moral,
                cast_ms: spell.cast_ms,
                range: f32::from(spell.range),
                radius: 0.0,
                direct: 0,
                first: 0,
                over_time: 0,
                duration_s: 0,
                element: 0,
                debuff: Some(d),
            }),
            _ => None,
        }
    }

    /// Area spells use the enemy-area moral and a radius. Known 1453 monsters
    /// only cast them as an area on the byMagicAttack 4/5 roll; long casts and
    /// mode-2 rolls hit the single target.
    pub fn area(&self) -> bool {
        self.moral == 10 && self.radius > 0.0
    }
}

/// Casting mode per prototype from migration 009 (`byMagicAttack`).
pub(crate) fn modes(db: &Connection) -> rusqlite::Result<HashMap<u16, u8>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='world_reference_rules')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(HashMap::new());
    }
    let mut query = db.prepare(
        "SELECT rule_id,CAST(json_extract(data,'$.byMagicAttack') AS INTEGER) FROM world_reference_rules WHERE table_name='MONSTER_MAGIC'",
    )?;
    query
        .query_map([], |r| {
            Ok((r.get::<_, u16>(0)?, r.get::<_, i64>(1)?.clamp(0, 255) as u8))
        })?
        .collect()
}

/// Known 1453 NPC magic damage against a player: no monster level, attack or
/// player armor; resistance only, the usual 0.85 + 0.3·random spread, then /3.
pub(crate) fn damage(amount: i16, resistance: u16, roll: impl FnOnce(u32) -> u32) -> u16 {
    let base = u32::from(amount.unsigned_abs());
    if base == 0 {
        return 0;
    }
    let scaled = 230 * base / (u32::from(resistance) + 250);
    let spread = (0.3 * roll(scaled) as f32) as u32 + (0.85 * scaled as f32) as u32;
    (spread / 3).clamp(1, 32000) as u16
}

/// Known 1453 lightning stun: about 21% below 125 lightning resistance, none above.
pub(crate) fn stuns(lightning_resistance: u16, roll: u32) -> bool {
    lightning_resistance < 125 && roll < 21
}

/// Resistance index for an element (1 fire .. 6 poison); 0 has none.
pub(crate) fn resistance(stats: &combat::Stats, element: u8) -> u16 {
    if (1..=6).contains(&element) {
        stats.resistance[usize::from(element - 1)]
    } else {
        0
    }
}

/// What a monster spell does to one player; applied by the victim's session.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Hit {
    pub skill: u32,
    /// Mana change (DirectType 2/3); negative drains.
    pub mana: i16,
    /// Percent of current HP drained to the monster (DirectType 9).
    pub drain_percent: u8,
    /// HP absorbed back into the monster (DirectType 8).
    pub absorb: bool,
    /// Per-tick HP change, duration and element for a damage-over-time.
    pub dot: Option<(i16, u64, u8)>,
    pub debuff: Option<Debuff>,
    /// Lightning stun that survived the resistance roll.
    pub stun_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npc_magic_damage_uses_resistance_spread_and_player_divisor() {
        // 230*1000/250 = 920 → 782..1058 before /3.
        assert_eq!(damage(-1000, 0, |_| 0), 260);
        assert_eq!(damage(-1000, 0, |max| max), 352);
        // Resistance 250 halves the base.
        assert_eq!(damage(-1000, 250, |_| 0), 130);
        assert_eq!(damage(0, 0, |max| max), 0);
        assert_eq!(damage(-32767, 0, |max| max), 11_555);
        assert!(damage(-1, 500, |_| 0) >= 1);
    }
}
