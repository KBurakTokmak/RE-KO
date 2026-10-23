//! Original-table Sprint and persisted mana/buff state.

use reko_assets::{CellValue, Table};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
pub(crate) struct SpeedSkill {
    pub mana: u16,
    pub duration_ms: u64,
    pub cooldown_ms: u64,
    pub speed_percent: u16,
    pub skill_group: u16,
    pub required_points: u16,
}

impl SpeedSkill {
    pub fn learned_by(self, class: u16, level: u8, points: &[u8; 9]) -> bool {
        if self.skill_group / 10 != class {
            return false;
        }
        let branch = usize::from(self.skill_group % 10);
        if branch == 0 {
            u16::from(level) >= self.required_points
        } else {
            points
                .get(branch)
                .is_some_and(|value| u16::from(*value) >= self.required_points)
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Spell {
    pub id: u32,
    pub name: String,
    pub moral: u8,
    pub skill_group: u16,
    pub required_points: u16,
    pub mana: u16,
    pub hp: u16,
    pub item_group: u8,
    pub use_item: u32,
    pub cast_ms: u64,
    pub cooldown_ms: u64,
    pub types: [u8; 2],
    pub range: u16,
    pub required_quest: u16,
    pub passive_defence: u16,
    pub passive_resistance: u16,
}

impl Spell {
    pub fn consumed_item(&self) -> u32 {
        if self.skill_group % 10 == 8
            && self.required_points == 2
            && matches!(self.use_item, 379063000 | 379064000 | 379065000 | 379066000)
        {
            self.use_item - 4000
        } else if self.requires_scroll() {
            0
        } else {
            self.use_item
        }
    }
    pub fn requires_scroll(&self) -> bool {
        matches!(self.skill_group / 10 % 100, 3 | 9 | 10)
            && matches!(self.required_points, 35 | 54 | 57)
            && self.types[0] == 3
            && matches!(
                self.use_item,
                370001000 | 370002000 | 370003000 | 379069000 | 379070000
            )
    }

    pub fn learned_by(&self, class: u16, level: u8, points: &[u8; 9]) -> bool {
        if self.skill_group == 0 {
            return self.use_item != 0
                && (self.moral == 1 || self.types == [4, 0] && matches!(self.moral, 2 | 4))
                && matches!(self.types, [3, 0] | [4, 0])
                && u16::from(level) >= self.required_points;
        }
        SpeedSkill {
            mana: self.mana,
            duration_ms: 0,
            cooldown_ms: self.cooldown_ms,
            speed_percent: 100,
            skill_group: self.skill_group,
            required_points: self.required_points,
        }
        .learned_by(class, level, points)
    }
}

pub(crate) struct Catalog {
    pub speeds: HashMap<u32, SpeedSkill>,
    pub spells: HashMap<u32, Spell>,
    pub effects: HashMap<(u8, u32), Vec<CellValue>>,
}

impl Catalog {
    #[cfg(test)]
    pub fn fixture(speeds: HashMap<u32, SpeedSkill>) -> Self {
        Self {
            speeds,
            spells: HashMap::new(),
            effects: HashMap::new(),
        }
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let main = Table::from_encrypted_bytes(&std::fs::read(
            path.join("Data/Skill_Magic_Main_us.tbl"),
        )?)?;
        let mut effects = HashMap::new();
        for kind in 1..=9u8 {
            let file = path.join(format!("Data/Skill_Magic_{kind}.tbl"));
            if !file.exists() {
                continue;
            }
            let table = Table::from_encrypted_bytes(&std::fs::read(file)?)?;
            for row in table.rows {
                effects.insert((kind, crate::combat::integer(&row, 0) as u32), row);
            }
        }
        let mut speeds = HashMap::new();
        let cures = Path::new("logs/reference-rules/MAGIC_TYPE5.csv");
        let alternative = Path::new("../../logs/reference-rules/MAGIC_TYPE5.csv");
        if let Some(file) = [cures, alternative].into_iter().find(|p| p.exists()) {
            for line in std::fs::read_to_string(file)?
                .lines()
                .skip(1)
                .filter(|l| !l.trim().is_empty())
            {
                let values = line
                    .trim_start_matches('\u{feff}')
                    .split(',')
                    .map(|v| v.trim().trim_matches('"').parse::<i32>())
                    .collect::<Result<Vec<_>, _>>()?;
                anyhow::ensure!(values.len() == 4, "invalid reference cure row");
                let id = values[0] as u32;
                effects
                    .entry((5, id))
                    .or_insert_with(|| values.into_iter().map(CellValue::I32).collect());
            }
        }
        let mut spells = HashMap::new();
        for row in main.rows {
            let get = |column| crate::combat::integer(&row, column).max(0) as u32;
            let id = get(0);
            let Some(CellValue::String(name)) = row.get(1) else {
                anyhow::bail!("missing skill name");
            };
            let spell = Spell {
                id,
                name: name.clone(),
                moral: get(14) as u8,
                skill_group: get(16) as u16,
                required_points: get(15) as u16,
                mana: get(17) as u16,
                hp: get(18) as u16,
                item_group: get(19) as u8,
                use_item: get(20),
                cast_ms: u64::from(get(21)) * 100,
                cooldown_ms: u64::from(get(22)) * 100,
                types: [get(26) as u8, get(27) as u8],
                range: get(28) as u16,
                required_quest: get(29) as u16,
                passive_defence: passive_amount(&row, "defense"),
                passive_resistance: passive_amount(&row, "resistance"),
            };
            if spell.types == [4, 0]
                && matches!(spell.moral, 1 | 2 | 6)
                && let Some(buff) = effects.get(&(4, id))
                && crate::combat::integer(buff, 1) == 6
            {
                speeds.insert(
                    id,
                    SpeedSkill {
                        mana: spell.mana,
                        duration_ms: crate::combat::integer(buff, 3).max(0) as u64 * 1000,
                        cooldown_ms: spell.cooldown_ms,
                        speed_percent: crate::combat::integer(buff, 5) as u16,
                        skill_group: spell.skill_group,
                        required_points: spell.required_points,
                    },
                );
            }
            spells.insert(id, spell);
        }
        anyhow::ensure!(!spells.is_empty(), "no skill definitions");
        Ok(Self {
            speeds,
            spells,
            effects,
        })
    }
}

fn passive_amount(row: &[CellValue], attribute: &str) -> u16 {
    let Some(CellValue::String(description)) = row.get(3) else {
        return 0;
    };
    if !description.starts_with("[Passive]") || !description.contains(attribute) {
        return 0;
    }
    description
        .split(|c: char| !c.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Buff {
    pub skill: u32,
    pub until: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Periodic {
    pub amount: i16,
    pub next: u64,
    pub until: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct State {
    pub max_hp: u16,
    pub max_mana: u16,
    pub hp: u16,
    pub mana: u16,
    pub cooldowns: HashMap<u32, u64>,
    pub buffs: HashMap<u8, Buff>,
    pub periodic: HashMap<u32, Periodic>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            max_hp: 100,
            max_mana: 100,
            hp: 100,
            mana: 100,
            cooldowns: HashMap::new(),
            buffs: HashMap::new(),
            periodic: HashMap::new(),
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl State {
    pub fn tick_periodic(&mut self, now: u64) {
        for effect in self.periodic.values_mut() {
            while self.hp != 0 && effect.next <= now && effect.next <= effect.until {
                self.hp = (i32::from(self.hp) + i32::from(effect.amount))
                    .clamp(0, i32::from(self.max_hp)) as u16;
                effect.next += 2000;
            }
        }
        self.periodic
            .retain(|_, effect| now < effect.until && self.hp != 0);
    }
    pub fn spend(&mut self, spell: &Spell, now: u64) -> Result<(), &'static str> {
        if self
            .cooldowns
            .get(&spell.id)
            .is_some_and(|deadline| now < *deadline)
        {
            return Err("This skill is still cooling down.");
        }
        if self.hp == 0 {
            return Err("You cannot use skills while dead.");
        }
        if self.mana < spell.mana {
            return Err("Not enough mana.");
        }
        if spell.hp > 0 && self.hp <= spell.hp {
            return Err("Not enough health.");
        }
        self.mana -= spell.mana;
        self.hp -= spell.hp;
        self.cooldowns.insert(spell.id, now + spell.cooldown_ms);
        Ok(())
    }
    pub fn set_limits(&mut self, hp: u16, mana: u16) {
        self.hp = if self.hp == self.max_hp {
            hp
        } else {
            self.hp.min(hp)
        };
        self.mana = if self.mana == self.max_mana {
            mana
        } else {
            self.mana.min(mana)
        };
        self.max_hp = hp;
        self.max_mana = mana;
    }
    pub fn cast(&mut self, id: u32, sprint: SpeedSkill, now: u64) -> Result<(), &'static str> {
        if self.cooldowns.get(&id).is_some_and(|until| now < *until) {
            return Err("This skill is still cooling down.");
        }
        if self.mana < sprint.mana {
            return Err("Not enough mana.");
        }
        self.mana -= sprint.mana;
        self.buffs.insert(
            6,
            Buff {
                skill: id,
                until: now + sprint.duration_ms,
            },
        );
        self.cooldowns.insert(id, now + sprint.cooldown_ms);
        Ok(())
    }

    pub fn speed(&self, catalog: Option<&Catalog>, now: u64) -> u16 {
        if let Some(buff) = self.buffs.get(&22).filter(|b| now < b.until)
            && let Some(row) = catalog.and_then(|c| c.effects.get(&(4, buff.skill)))
        {
            return (45u32 * crate::combat::integer(row, 5).clamp(0, 100) as u32).div_ceil(100)
                as u16;
        }
        if let Some(buff) = self.buffs.get(&28).filter(|b| now < b.until)
            && let Some(row) = catalog.and_then(|c| c.effects.get(&(4, buff.skill)))
        {
            return (45u32 * crate::combat::integer(row, 5).clamp(1, 200) as u32).div_ceil(100)
                as u16;
        }
        if let Some(buff) = self.buffs.get(&6).filter(|buff| now < buff.until)
            && let Some(skill) = catalog.and_then(|c| c.speeds.get(&buff.skill))
        {
            return (45u32 * u32::from(skill.speed_percent)).div_ceil(100) as u16;
        }
        45
    }

    pub fn expire(&mut self, now: u64) -> Vec<u8> {
        let expired: Vec<_> = self
            .buffs
            .iter()
            .filter(|(_, buff)| now >= buff.until)
            .map(|(kind, _)| *kind)
            .collect();
        for kind in &expired {
            self.buffs.remove(kind);
        }
        expired
    }

    pub fn mana_packet(&self) -> Vec<u8> {
        let mut p = vec![0x18];
        p.extend(self.max_mana.to_le_bytes());
        p.extend(self.mana.to_le_bytes());
        p
    }

    pub fn hp_packet(&self, delta: i16) -> Vec<u8> {
        let mut p = vec![0x17];
        p.extend(self.max_hp.to_le_bytes());
        p.extend(self.hp.to_le_bytes());
        p.extend(delta.to_le_bytes());
        p
    }

    pub fn hp_packet_from(&self, attacker: u16) -> Vec<u8> {
        self.hp_packet(attacker as i16)
    }

    pub fn regenerate(&mut self, level: u8, class: u16, sitting: bool) -> (u16, u16) {
        if self.hp == 0 {
            return (0, 0);
        }
        let level = f64::from(level.max(1));
        let hp_delta = if sitting {
            (level * (1.0 + level / 30.0)) as u16 + 3
        } else {
            0
        };
        let mana_delta = if sitting {
            (u32::from(self.max_mana) * 5 / (level as u32 + 29) + 3) as u16
        } else {
            ((level * (1.0 + level / 60.0) + 1.0) * 0.2 + 3.0) as u16
        };
        let mana_delta = if matches!(class, 110 | 210)
            && u32::from(self.mana) * 100 < u32::from(self.max_mana) * 30
        {
            mana_delta * 120 / 100
        } else {
            mana_delta
        };
        let hp_before = self.hp;
        let mana_before = self.mana;
        self.hp = self.hp.saturating_add(hp_delta).min(self.max_hp);
        self.mana = self.mana.saturating_add(mana_delta).min(self.max_mana);
        (self.hp - hp_before, self.mana - mana_before)
    }
}

pub(crate) fn effect(id: u32, player: u16) -> Vec<u8> {
    let mut p = vec![0x31, 3];
    p.extend(id.to_le_bytes());
    p.extend(player.to_le_bytes());
    p.extend(player.to_le_bytes());
    p.extend(0i16.to_le_bytes());
    p.extend(1i16.to_le_bytes());
    p.extend([0; 8]);
    p
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS character_periodic_effects (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL, skill INTEGER NOT NULL,
        amount INTEGER NOT NULL CHECK(amount BETWEEN -32767 AND 32767), next_tick INTEGER NOT NULL, until_tick INTEGER NOT NULL,
        PRIMARY KEY(account,slot,skill), FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS character_skill_state (
      account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
      mana INTEGER NOT NULL CHECK(mana BETWEEN 0 AND 100), next_cast INTEGER NOT NULL,
      buff_until INTEGER NOT NULL, skill INTEGER NOT NULL,
      PRIMARY KEY(account,slot), FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
      CREATE TABLE IF NOT EXISTS character_skill_cooldowns (
      account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL, skill INTEGER NOT NULL,
      next_cast INTEGER NOT NULL, PRIMARY KEY(account,slot,skill),
      FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
      INSERT OR IGNORE INTO character_skill_cooldowns(account,slot,skill,next_cast)
      SELECT account,slot,skill,next_cast FROM character_skill_state WHERE skill != 0 AND next_cast != 0;")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS character_buffs (
    account TEXT NOT NULL COLLATE NOCASE,slot INTEGER NOT NULL,category INTEGER NOT NULL,skill INTEGER NOT NULL,buff_until INTEGER NOT NULL,
    PRIMARY KEY(account,slot,category),FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
    INSERT OR IGNORE INTO character_buffs(account,slot,category,skill,buff_until)
    SELECT account,slot,6,skill,buff_until FROM character_skill_state WHERE skill != 0 AND buff_until != 0;")?;
    let mut columns = db.prepare("PRAGMA table_info(character_skill_state)")?;
    let names = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !names.iter().any(|name| name == "hp") {
        db.execute_batch("ALTER TABLE character_skill_state ADD COLUMN hp INTEGER NOT NULL DEFAULT 100 CHECK(hp BETWEEN 0 AND 100)")?;
    }
    if !names.iter().any(|name| name == "max_hp") {
        db.execute_batch("BEGIN;
        CREATE TABLE character_skill_state_v2 (
        account TEXT NOT NULL COLLATE NOCASE,slot INTEGER NOT NULL,
        mana INTEGER NOT NULL CHECK(mana BETWEEN 0 AND 32767),next_cast INTEGER NOT NULL,
        buff_until INTEGER NOT NULL,skill INTEGER NOT NULL,hp INTEGER NOT NULL CHECK(hp BETWEEN 0 AND 32767),
        max_hp INTEGER NOT NULL CHECK(max_hp BETWEEN 1 AND 32767),max_mana INTEGER NOT NULL CHECK(max_mana BETWEEN 1 AND 32767),
        PRIMARY KEY(account,slot),FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
        INSERT INTO character_skill_state_v2 SELECT account,slot,mana,next_cast,buff_until,skill,hp,100,100 FROM character_skill_state;
        DROP TABLE character_skill_state;
        ALTER TABLE character_skill_state_v2 RENAME TO character_skill_state;
        COMMIT;")?;
    }
    Ok(())
}

pub(crate) fn load(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<State> {
    let mut state=db.query_row("SELECT mana,hp,max_hp,max_mana FROM character_skill_state WHERE account=?1 AND slot=?2", params![account,slot], |r| Ok(State {mana:r.get(0)?,hp:r.get(1)?,max_hp:r.get(2)?,max_mana:r.get(3)?,cooldowns:HashMap::new(),buffs:HashMap::new(),periodic:HashMap::new()})).optional()?.unwrap_or_default();
    let mut query=db.prepare("SELECT skill,amount,next_tick,until_tick FROM character_periodic_effects WHERE account=?1 AND slot=?2")?;
    state.periodic = query
        .query_map(params![account, slot], |r| {
            Ok((
                r.get(0)?,
                Periodic {
                    amount: r.get(1)?,
                    next: r.get(2)?,
                    until: r.get(3)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    let mut query = db.prepare(
        "SELECT skill,next_cast FROM character_skill_cooldowns WHERE account=?1 AND slot=?2",
    )?;
    state.cooldowns = query
        .query_map(params![account, slot], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    let mut query = db.prepare(
        "SELECT category,skill,buff_until FROM character_buffs WHERE account=?1 AND slot=?2",
    )?;
    state.buffs = query
        .query_map(params![account, slot], |r| {
            Ok((
                r.get(0)?,
                Buff {
                    skill: r.get(1)?,
                    until: r.get(2)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    Ok(state)
}

pub(crate) fn save(
    db: &Connection,
    account: &str,
    slot: u8,
    state: &State,
) -> rusqlite::Result<()> {
    let transaction = db.unchecked_transaction()?;
    save_in(&transaction, account, slot, state)?;
    transaction.commit()
}

pub(crate) fn save_in(
    db: &Connection,
    account: &str,
    slot: u8,
    state: &State,
) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM character_periodic_effects WHERE account=?1 AND slot=?2",
        params![account, slot],
    )?;
    for (skill, effect) in &state.periodic {
        db.execute(
            "INSERT INTO character_periodic_effects VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                account,
                slot,
                skill,
                effect.amount,
                effect.next,
                effect.until
            ],
        )?;
    }
    db.execute("INSERT INTO character_skill_state(account,slot,mana,next_cast,buff_until,skill,hp,max_hp,max_mana) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
    ON CONFLICT(account,slot) DO UPDATE SET mana=excluded.mana,next_cast=excluded.next_cast,buff_until=excluded.buff_until,skill=excluded.skill,hp=excluded.hp,max_hp=excluded.max_hp,max_mana=excluded.max_mana", params![account,slot,state.mana,0,0,0,state.hp,state.max_hp,state.max_mana])?;
    for (skill, next_cast) in &state.cooldowns {
        db.execute("INSERT INTO character_skill_cooldowns(account,slot,skill,next_cast) VALUES (?1,?2,?3,?4) ON CONFLICT(account,slot,skill) DO UPDATE SET next_cast=excluded.next_cast",params![account,slot,skill,next_cast])?;
    }
    db.execute(
        "DELETE FROM character_buffs WHERE account=?1 AND slot=?2",
        params![account, slot],
    )?;
    for (kind, buff) in &state.buffs {
        db.execute("INSERT INTO character_buffs(account,slot,category,skill,buff_until) VALUES (?1,?2,?3,?4,?5)",params![account,slot,kind,buff.skill,buff.until])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spent_mana_and_timers_survive_database_reopen() {
        let path =
            std::env::temp_dir().join(format!("reko-skill-{}-{}.db", std::process::id(), now_ms()));
        {
            let mut db = Connection::open(&path).unwrap();
            crate::characters::initialize(&db).unwrap();
            initialize(&db).unwrap();
            crate::characters::ensure_account(&db, "admin").unwrap();
            crate::characters::set_nation(&db, "admin", 1).unwrap();
            let character = crate::characters::Character {
                slot: 0,
                name: "SkillTest".into(),
                race: 2,
                class: 102,
                face: 0,
                hair: 0,
                stats: [60, 60, 80, 50, 50],
            };
            assert_eq!(
                crate::characters::create(&mut db, "admin", &character).unwrap(),
                0
            );
            let state = State {
                periodic: HashMap::new(),
                max_hp: 100,
                max_mana: 100,
                hp: 100,
                mana: 75,
                cooldowns: HashMap::from([(102001, 6000)]),
                buffs: HashMap::from([(
                    6,
                    Buff {
                        until: 10000,
                        skill: 102001,
                    },
                )]),
            };
            save(&db, "admin", 0, &state).unwrap();
        }
        {
            let db = Connection::open(&path).unwrap();
            let state = load(&db, "ADMIN", 0).unwrap();
            assert_eq!(
                (
                    state.mana,
                    state.cooldowns[&102001],
                    state.buffs[&6].until,
                    state.buffs[&6].skill
                ),
                (75, 6000, 10000, 102001)
            );
            assert_eq!(load(&db, "other", 0).unwrap().mana, 100);
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn sprint_cost_cooldown_expiry_and_insufficient_mana() {
        let sprint = SpeedSkill {
            mana: 5,
            duration_ms: 10000,
            cooldown_ms: 6000,
            speed_percent: 150,
            skill_group: 1020,
            required_points: 1,
        };
        let catalog = Catalog::fixture(HashMap::from([(102001, sprint)]));
        let mut s = State::default();
        s.cast(102001, sprint, 1000).unwrap();
        assert_eq!(s.mana, 95);
        assert_eq!(s.speed(Some(&catalog), 1001), 68);
        assert!(s.cast(102001, sprint, 6999).is_err());
        assert_eq!(s.mana, 95);
        s.cast(102001, sprint, 7000).unwrap();
        assert_eq!(s.mana, 90);
        assert_eq!(s.speed(Some(&catalog), 11000), 68);
        assert_eq!(s.speed(Some(&catalog), 17000), 45);
        assert_eq!(s.expire(17000), [6]);
        assert!(s.expire(17001).is_empty());
        s.mana = 4;
        assert!(s.cast(102001, sprint, 17000).is_err());
        assert_eq!(s.mana, 4);
        assert_eq!(s.mana_packet(), [0x18, 100, 0, 4, 0]);
    }
    #[test]
    fn original_sprint_is_not_modern_stroke() {
        let root = reko_common::game_dir();
        if !root.join("Data/Skill_Magic_4.tbl").exists() {
            return;
        }
        let c = Catalog::load(&root).unwrap();
        let sprint = c.speeds[&102001];
        assert_eq!(
            (sprint.mana, sprint.duration_ms, sprint.speed_percent),
            (5, 10000, 150)
        );
        assert_eq!(
            (
                c.speeds[&107010].duration_ms,
                c.speeds[&107010].speed_percent
            ),
            (600000, 150)
        );
        assert_eq!(
            (
                c.speeds[&107725].duration_ms,
                c.speeds[&107725].speed_percent
            ),
            (10000, 200)
        );
    }

    #[test]
    fn successful_buff_sets_the_client_success_field() {
        let packet = effect(102001, 7);
        assert_eq!(
            packet,
            [
                0x31, 3, 0x71, 0x8e, 1, 0, 7, 0, 7, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn cooldowns_do_not_block_unrelated_skills() {
        let sprint = SpeedSkill {
            mana: 5,
            duration_ms: 10000,
            cooldown_ms: 6000,
            speed_percent: 150,
            skill_group: 1020,
            required_points: 1,
        };
        let mut state = State::default();
        state.cast(102001, sprint, 1000).unwrap();
        state
            .cast(
                107725,
                SpeedSkill {
                    mana: 40,
                    speed_percent: 200,
                    ..sprint
                },
                2000,
            )
            .unwrap();
        assert!(state.cast(102001, sprint, 6999).is_err());
        state.cast(102001, sprint, 7000).unwrap();
        assert!(state.cast(107725, sprint, 7999).is_err());
    }

    #[test]
    fn recovery_uses_posture_and_stops_at_maximum_or_death() {
        let mut state = State {
            hp: 90,
            mana: 50,
            ..State::default()
        };
        assert_eq!(state.regenerate(1, 102, false), (0, 3));
        assert_eq!(state.regenerate(1, 102, true), (4, 19));
        assert_eq!(state.hp_packet(4), [0x17, 100, 0, 94, 0, 4, 0]);
        state.hp = 99;
        state.mana = 99;
        assert_eq!(state.regenerate(1, 102, true), (1, 1));
        assert_eq!(state.regenerate(1, 102, true), (0, 0));
        state.hp = 0;
        state.mana = 20;
        assert_eq!(state.regenerate(1, 102, true), (0, 0));
        state.hp = 100;
        state.mana = 20;
        assert_eq!(state.regenerate(60, 110, false), (0, 32));
    }
}
