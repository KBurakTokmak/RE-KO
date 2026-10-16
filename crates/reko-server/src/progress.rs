//! Persisted level and skill allocation; XP rewards are handled by quest_rewards.

use rusqlite::{Connection, OptionalExtension, params};

pub(crate) const LEVEL_CAP: u8 = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Progress {
    pub level: u8,
    pub points: [u8; 9],
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            level: 1,
            points: [0; 9],
        }
    }
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS character_progress (
    account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
    level INTEGER NOT NULL CHECK(level BETWEEN 1 AND 80), points BLOB NOT NULL CHECK(length(points)=9),
    PRIMARY KEY(account,slot), FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);")
}

pub(crate) fn load(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<Progress> {
    Ok(db
        .query_row(
            "SELECT level,points FROM character_progress WHERE account=?1 AND slot=?2",
            params![account, slot],
            |row| {
                let points: Vec<u8> = row.get(1)?;
                Ok(Progress {
                    level: row.get(0)?,
                    points: points
                        .try_into()
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

pub(crate) fn save(
    db: &Connection,
    account: &str,
    slot: u8,
    progress: &Progress,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO character_progress(account,slot,level,points) VALUES (?1,?2,?3,?4)
    ON CONFLICT(account,slot) DO UPDATE SET level=excluded.level,points=excluded.points",
        params![account, slot, progress.level, progress.points.as_slice()],
    )?;
    Ok(())
}

pub(crate) fn allocate(
    db: &Connection,
    account: &str,
    slot: u8,
    class: u16,
    branch: u8,
) -> rusqlite::Result<(Progress, bool)> {
    let tx = db.unchecked_transaction()?;
    let mut progress = load(&tx, account, slot)?;
    let mastered = matches!(class % 100, 6 | 8 | 10 | 12);
    let valid = (5..=8).contains(&branch) && class % 100 >= 5 && progress.points[0] > 0;
    let cap = if branch == 8 {
        progress.level.saturating_sub(60).min(LEVEL_CAP - 60)
    } else {
        progress.level
    };
    let valid = valid
        && (branch != 8 || mastered)
        && progress
            .points
            .get(usize::from(branch))
            .is_some_and(|points| *points < cap);
    if valid {
        progress.points[0] -= 1;
        progress.points[usize::from(branch)] += 1;
        save(&tx, account, slot, &progress)?;
    }
    tx.commit()?;
    Ok((progress, valid))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn master_points_are_limited_to_twenty_at_eighty() {
        let db = Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        initialize(&db).unwrap();
        db.execute("INSERT INTO lobby_accounts(name) VALUES ('admin')", [])
            .unwrap();
        db.execute("INSERT INTO lobby_characters(account,slot,name,race,class,face,hair,stats) VALUES ('admin',1,'Rogue80',2,108,0,0,zeroblob(5))",[]).unwrap();
        let mut points = [0; 9];
        points[0] = 160;
        save(&db, "admin", 1, &Progress { level: 80, points }).unwrap();
        for _ in 0..20 {
            assert!(allocate(&db, "admin", 1, 108, 8).unwrap().1);
        }
        let (progress, success) = allocate(&db, "ADMIN", 1, 108, 8).unwrap();
        assert!(!success);
        assert_eq!((progress.points[0], progress.points[8]), (140, 20));
        assert!(!allocate(&db, "admin", 1, 108, 0).unwrap().1);
        assert!(!allocate(&db, "admin", 1, 108, 255).unwrap().1);
        assert!(!allocate(&db, "admin", 1, 107, 8).unwrap().1);
    }
}
