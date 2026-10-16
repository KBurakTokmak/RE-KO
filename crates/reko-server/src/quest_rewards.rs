//! Persisted quest/monster XP/NP and original-table level rewards.
use crate::{combat, progress, world_entry::Session};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use tokio::net::TcpStream;
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Totals {
    pub experience: u32,
    pub loyalty: u32,
    pub monthly: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub totals: Totals,
    pub progress: progress::Progress,
    pub stat_points: u16,
    pub max_exp: u32,
    stat_total: u16,
}
pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(include_str!("quest_rewards.sql"))
}
pub(crate) fn load(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<Totals> {
    Ok(db.query_row("SELECT experience,loyalty,monthly FROM character_quest_rewards WHERE account=?1 AND slot=?2",params![account,slot],|r|Ok(Totals{experience:r.get(0)?,loyalty:r.get(1)?,monthly:r.get(2)?})).optional()?.unwrap_or(Totals { loyalty: 300, ..Totals::default() }))
}
fn thresholds() -> anyhow::Result<BTreeMap<u8, u32>> {
    let table = reko_assets::Table::from_encrypted_bytes(&std::fs::read(
        reko_common::game_dir().join("Data/Player_experience.tbl"),
    )?)?;
    table
        .rows
        .iter()
        .map(|r| {
            Ok((
                u8::try_from(combat::integer(r, 0))?,
                u32::try_from(combat::integer(r, 1))?,
            ))
        })
        .collect()
}

pub(crate) fn verify_thresholds(db: &Connection) -> anyhow::Result<()> {
    let rates = thresholds()?;
    let mut q =
        db.prepare("SELECT rule_id,data FROM world_reference_rules WHERE table_name='LEVEL_UP'")?;
    let mut count = 0;
    for row in q.query_map([], |r| Ok((r.get::<_, u8>(0)?, r.get::<_, String>(1)?)))? {
        let (level, data) = row?;
        let data: serde_json::Value = serde_json::from_str(&data)?;
        let amount = data["Exp"].as_str().and_then(|n| n.parse::<u32>().ok());
        anyhow::ensure!(
            rates.get(&level).copied() == amount,
            "1453 DB/client XP threshold mismatch at level {level}"
        );
        count += 1;
    }
    anyhow::ensure!(
        count == usize::from(progress::LEVEL_CAP),
        "import all 80 original LEVEL_UP rules before starting"
    );
    Ok(())
}

fn amounts(
    credit: &crate::monsters::ExperienceCredit,
    player_level: u8,
    xp: u32,
    np: u32,
) -> (u32, u32) {
    if credit.total_damage == 0 {
        return (0, 0);
    }
    let share =
        |value: u32| (u64::from(value) * u64::from(credit.damage)).div_ceil(credit.total_damage);
    let difference = i16::from(credit.level) - i16::from(player_level);
    let percent = match difference {
        ..=-14 => 20,
        -13..=-8 => 50,
        -7..=-2 => 80,
        _ => 100,
    };
    (
        ((share(xp) * percent).div_ceil(100)).min(u64::from(u32::MAX)) as u32,
        share(np).min(u64::from(u32::MAX)) as u32,
    )
}

fn apply_monster(
    lobby: &crate::lobby::Lobby,
    account: &str,
    s: &Session,
    credit: &crate::monsters::ExperienceCredit,
) -> anyhow::Result<Option<Session>> {
    let dx = i32::from(s.position.x) - i32::from(credit.position.x);
    let dz = i32::from(s.position.z) - i32::from(credit.position.z);
    if !s.entered
        || s.skills.hp == 0
        || s.position.zone != credit.position.zone
        || i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz) > 500 * 500
    {
        return Ok(None);
    }
    let mut db = lobby.db.lock().unwrap();
    let data: Option<String> = db.query_row("SELECT data FROM world_reference_rules WHERE table_name='MONSTER_REWARD' AND rule_id=?1", [credit.prototype], |r| r.get(0)).optional()?;
    let Some(data) = data else {
        return Ok(None);
    };
    let data: serde_json::Value = serde_json::from_str(&data)?;
    let value = |name: &str| -> anyhow::Result<u32> {
        Ok(data[name]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing monster reward {name}"))?
            .parse()?)
    };
    let (xp, np) = amounts(credit, s.progress.level, value("iExp")?, value("iLoyalty")?);
    if xp == 0 && np == 0 {
        return Ok(None);
    }
    let tx = db.transaction()?;
    let mut state = State::new(&tx, account, s)?;
    state.gain(900001000, xp)?;
    state.gain(900002000, np)?;
    state.save(&tx, account, s.character.slot)?;
    let mut next = s.clone();
    next.rewards = state.totals;
    next.progress = state.progress;
    next.max_exp = state.max_exp;
    next.stat_points = state.stat_points;
    if next.progress.level != s.progress.level {
        let catalog = lobby
            .combat
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing combat catalog"))?;
        crate::durability::refresh(&mut next, catalog, lobby.skills.as_ref());
        next.skills.set_limits(next.combat.hp, next.combat.mp);
        next.skills.hp = next.skills.max_hp;
        next.skills.mana = next.skills.max_mana;
        crate::skills::save_in(&tx, account, s.character.slot, &next.skills)?;
    }
    tx.commit()?;
    tracing::info!(character=%s.character.name,prototype=credit.prototype,xp,np,level=next.progress.level,experience=next.rewards.experience,"monster experience committed");
    Ok(Some(next))
}

pub(crate) async fn monster(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &crate::lobby::Lobby,
    account: &str,
    s: &mut Session,
    credit: &crate::monsters::ExperienceCredit,
) {
    match apply_monster(lobby, account, s, credit) {
        Ok(Some(next)) => {
            let level = next.progress.level != s.progress.level;
            let loyalty = next.rewards.loyalty != s.rewards.loyalty;
            *s = next;
            let packet = if level {
                level_packet(s)
            } else {
                experience_packet(s)
            };
            super::send_response(sock, peer, &None, &packet, "MONSTER_EXPERIENCE").await;
            if level {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &crate::inventory::attributes(&s.combat, 2),
                    "LEVEL_ATTRIBUTES",
                )
                .await;
            }
            if loyalty {
                super::send_response(
                    sock,
                    peer,
                    &None,
                    &loyalty_packet(s),
                    "MONSTER_NATIONAL_POINTS",
                )
                .await;
            }
        }
        Ok(None) => (),
        Err(error) => {
            tracing::error!(%error,character=%s.character.name,"monster experience persistence failed")
        }
    }
}
pub(crate) fn maximum(level: u8) -> anyhow::Result<u32> {
    thresholds()?
        .get(&level)
        .copied()
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow::anyhow!("missing original experience threshold"))
}
impl State {
    pub fn new(db: &Connection, account: &str, s: &Session) -> rusqlite::Result<Self> {
        Ok(Self {
            totals: load(db, account, s.character.slot)?,
            progress: s.progress.clone(),
            stat_points: s.stat_points,
            max_exp: s.max_exp,
            stat_total: s.character.stats.iter().map(|v| u16::from(*v)).sum(),
        })
    }
    pub fn gain(&mut self, id: u32, amount: u32) -> anyhow::Result<bool> {
        match id {
            900001000 => self.experience(amount, &thresholds()?)?,
            900002000 | 900003000 => {
                self.totals.loyalty = self.totals.loyalty.saturating_add(amount).min(2100000000);
                self.totals.monthly = self.totals.monthly.saturating_add(amount).min(2100000000);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn experience(&mut self, amount: u32, rates: &BTreeMap<u8, u32>) -> anyhow::Result<()> {
        let mut xp = u64::from(self.totals.experience) + u64::from(amount);
        loop {
            self.max_exp = *rates
                .get(&self.progress.level)
                .filter(|n| **n > 0)
                .ok_or_else(|| anyhow::anyhow!("missing original experience threshold"))?;
            if xp < u64::from(self.max_exp) {
                break;
            }
            if self.progress.level >= progress::LEVEL_CAP {
                xp = u64::from(self.max_exp);
                break;
            }
            xp -= u64::from(self.max_exp);
            self.progress.level += 1;
            let level = u16::from(self.progress.level);
            if self.stat_total + self.stat_points < 297 + 3 * level + 2 * level.saturating_sub(60) {
                self.stat_points = self
                    .stat_points
                    .saturating_add(if level > 60 { 5 } else { 3 });
            }
            if level >= 10
                && self
                    .progress
                    .points
                    .iter()
                    .map(|v| u16::from(*v))
                    .sum::<u16>()
                    < 2 * (level - 9)
            {
                self.progress.points[0] = self.progress.points[0].saturating_add(2);
            }
        }
        self.totals.experience = u32::try_from(xp)?;
        Ok(())
    }
    pub fn save(&self, db: &Connection, account: &str, slot: u8) -> rusqlite::Result<()> {
        db.execute("INSERT INTO character_quest_rewards VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account,slot) DO UPDATE SET experience=excluded.experience,loyalty=excluded.loyalty,monthly=excluded.monthly",params![account,slot,self.totals.experience,self.totals.loyalty,self.totals.monthly])?;
        progress::save(db, account, slot, &self.progress)?;
        db.execute("INSERT INTO character_stat_points VALUES(?1,?2,?3) ON CONFLICT(account,slot) DO UPDATE SET remaining=excluded.remaining",params![account,slot,self.stat_points])?;
        Ok(())
    }
}
pub(crate) fn experience_packet(s: &Session) -> Vec<u8> {
    let mut p = vec![0x1a, 0];
    p.extend(s.rewards.experience.to_le_bytes());
    p
}
pub(crate) fn loyalty_packet(s: &Session) -> Vec<u8> {
    let mut p = vec![0x2a, 1];
    p.extend(s.rewards.loyalty.to_le_bytes());
    p.extend(s.rewards.monthly.to_le_bytes());
    p
}
pub(crate) fn level_packet(s: &Session) -> Vec<u8> {
    let mut p = vec![0x1b];
    p.extend(s.player_id.to_le_bytes());
    p.push(s.progress.level);
    p.extend(s.stat_points.to_le_bytes());
    p.push(s.progress.points[0]);
    p.extend(s.max_exp.to_le_bytes());
    p.extend(s.rewards.experience.to_le_bytes());
    for v in [
        s.skills.max_hp,
        s.skills.hp,
        s.skills.max_mana,
        s.skills.mana,
        s.combat.max_weight,
        s.combat.weight,
    ] {
        p.extend(v.to_le_bytes());
    }
    p
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_characters_start_with_300_np_without_resetting_spent_or_monthly_points() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        crate::characters::initialize(&db).unwrap();
        initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "leveltest").unwrap();
        let insert = || {
            db.execute(
                "INSERT INTO lobby_characters VALUES('leveltest',0,'LevelTest',2,102,0,0,?1)",
                [[60u8; 5].as_slice()],
            )
            .unwrap()
        };
        insert();
        assert_eq!(
            load(&db, "leveltest", 0).unwrap(),
            Totals {
                experience: 0,
                loyalty: 300,
                monthly: 0
            }
        );
        db.execute(
            "UPDATE character_quest_rewards SET experience=42,loyalty=17,monthly=9",
            [],
        )
        .unwrap();
        initialize(&db).unwrap();
        assert_eq!(
            load(&db, "leveltest", 0).unwrap(),
            Totals {
                experience: 42,
                loyalty: 17,
                monthly: 9
            }
        );
        db.execute("DELETE FROM lobby_characters WHERE account='leveltest'", [])
            .unwrap();
        insert();
        assert_eq!(
            load(&db, "leveltest", 0).unwrap(),
            Totals {
                experience: 0,
                loyalty: 300,
                monthly: 0
            }
        );
    }
    #[test]
    fn primary_database_thresholds_match_native_client() {
        let path = std::path::Path::new("../../../PV_1453/inspection/LEVEL_UP.csv");
        if !path.exists()
            || !reko_common::game_dir()
                .join("Data/Player_experience.tbl")
                .exists()
        {
            return;
        }
        let rates = thresholds().unwrap();
        let mut count = 0;
        for line in std::fs::read_to_string(path).unwrap().lines().skip(1) {
            let fields: Vec<_> = line.split(',').map(|v| v.trim_matches('"')).collect();
            let level: u8 = fields[0].parse().unwrap();
            let xp: u32 = fields[1].parse().unwrap();
            assert_eq!(rates[&level], xp, "level {level}");
            count += 1;
        }
        assert_eq!(count, 80);
    }
    #[test]
    fn monster_shares_modifiers_range_and_persistence() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        let mut credit = crate::monsters::ExperienceCredit {
            prototype: 100,
            level: 6,
            position: s.position,
            damage: 32,
            total_damage: 32,
        };
        assert_eq!(amounts(&credit, 6, 593, 7), (593, 7));
        assert_eq!(amounts(&credit, 8, 593, 7), (475, 7));
        assert_eq!(amounts(&credit, 14, 593, 7), (297, 7));
        assert_eq!(amounts(&credit, 20, 593, 7), (119, 7));
        credit.damage = 1;
        assert_eq!(amounts(&credit, 6, 593, 7), (19, 1));
        credit.damage = 32;
        s.entered = true;
        s.progress.level = 1;
        s.progress.points = [0; 9];
        s.stat_points = 0;
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO world_reference_rules VALUES('MONSTER_REWARD',100,?1)",
                [r#"{"iExp":"593","iLoyalty":"0"}"#],
            )
            .unwrap();
        }
        s.skills.hp = 0;
        assert!(
            apply_monster(&lobby, "test", &s, &credit)
                .unwrap()
                .is_none()
        );
        s.skills.hp = 1;
        credit.position.zone = s.position.zone + 1;
        assert!(
            apply_monster(&lobby, "test", &s, &credit)
                .unwrap()
                .is_none()
        );
        credit.position = s.position;
        credit.position.x += 501;
        assert!(
            apply_monster(&lobby, "test", &s, &credit)
                .unwrap()
                .is_none()
        );
        credit.position = s.position;
        let next = apply_monster(&lobby, "test", &s, &credit).unwrap().unwrap();
        assert_eq!((next.progress.level, next.rewards.experience), (4, 253));
        assert_eq!(next.skills.hp, next.skills.max_hp);
        assert_eq!(next.skills.mana, next.skills.max_mana);
        let db = lobby.db.lock().unwrap();
        assert_eq!(load(&db, "test", s.character.slot).unwrap(), next.rewards);
        assert_eq!(
            progress::load(&db, "test", s.character.slot).unwrap(),
            next.progress
        );
    }
    #[test]
    fn thresholds_level_rewards_cap_and_reload() {
        let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        s.progress.level = 1;
        s.progress.points = [0; 9];
        s.stat_points = 0;
        let db = lobby.db.lock().unwrap();
        let mut state = State::new(&db, "test", &s).unwrap();
        let rates = thresholds().unwrap();
        state.experience(50, &rates).unwrap();
        assert_eq!((state.progress.level, state.totals.experience), (2, 0));
        assert_eq!(state.stat_points, 0);
        state.stat_total = 297;
        state.progress.level = 9;
        state.totals.experience = 0;
        state.experience(rates[&9], &rates).unwrap();
        assert_eq!(
            (
                state.progress.level,
                state.progress.points[0],
                state.stat_points
            ),
            (10, 2, 3)
        );
        state.progress.level = 80;
        state.totals.experience = 0;
        state.experience(u32::MAX, &rates).unwrap();
        assert_eq!(
            (state.progress.level, state.totals.experience),
            (80, rates[&80])
        );
        state.gain(900002000, 50).unwrap();
        state.save(&db, "test", 0).unwrap();
        assert_eq!(load(&db, "test", 0).unwrap(), state.totals);
        assert_eq!(progress::load(&db, "test", 0).unwrap(), state.progress);
    }
}
