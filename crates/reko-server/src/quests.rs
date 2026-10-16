//! Persisted quest states and native v1453 skill-unlock snapshot.

use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    let old: Option<String> = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='character_quests'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if old
        .as_ref()
        .is_some_and(|sql| sql.contains("state BETWEEN 0 AND 2"))
    {
        db.execute_batch(
            "BEGIN IMMEDIATE;
        CREATE TABLE character_quests_expanded (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        quest INTEGER NOT NULL CHECK(quest BETWEEN 1 AND 32767),
        state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 255),
        PRIMARY KEY(account,slot,quest),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
        INSERT INTO character_quests_expanded SELECT * FROM character_quests;
        DROP TABLE character_quests;
        ALTER TABLE character_quests_expanded RENAME TO character_quests;
        COMMIT;",
        )?;
    }
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS character_quests (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        quest INTEGER NOT NULL CHECK(quest BETWEEN 1 AND 32767),
        state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 255),
        PRIMARY KEY(account,slot,quest),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);",
    )?;
    let has_counters: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='character_quest_kills')",
        [], |r| r.get(0),
    )?;
    if !has_counters {
        db.execute_batch(
            "BEGIN IMMEDIATE;
            CREATE TABLE character_quest_kills (
                account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
                quest INTEGER NOT NULL, group_id INTEGER NOT NULL CHECK(group_id BETWEEN 1 AND 4),
                count INTEGER NOT NULL CHECK(count BETWEEN 0 AND 255),
                PRIMARY KEY(account,slot,quest,group_id),
                FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
            INSERT INTO character_quest_kills
            SELECT q.account,q.slot,q.quest,c.quest-32000,c.state
            FROM character_quests q
            JOIN world_reference_rules r ON r.table_name='QUEST_MONSTER' AND r.rule_id=q.quest
            JOIN character_quests c ON c.account=q.account AND c.slot=q.slot AND c.quest BETWEEN 32001 AND 32004
            WHERE q.state IN (1,3) AND q.quest=(
                SELECT MIN(q2.quest) FROM character_quests q2
                JOIN world_reference_rules r2 ON r2.table_name='QUEST_MONSTER' AND r2.rule_id=q2.quest
                WHERE q2.account=q.account AND q2.slot=q.slot AND q2.state IN (1,3));
            COMMIT;",
        )?;
    }
    Ok(())
}

pub(crate) const ACTIVE_LIMIT: usize = 40;
pub(crate) fn check_capacity(
    states: &std::collections::HashMap<u16, u8>,
    quest: u16,
) -> anyhow::Result<()> {
    if !states
        .get(&quest)
        .is_some_and(|state| matches!(state, 1 | 3))
    {
        anyhow::ensure!(
            states
                .iter()
                .filter(|(id, state)| **id < 32000 && matches!(state, 1 | 3))
                .count()
                < ACTIVE_LIMIT,
            "quest log is full (40 active quests)"
        );
    }
    Ok(())
}

pub(crate) fn counts(
    db: &Connection,
    account: &str,
    slot: u8,
    quest: u16,
) -> rusqlite::Result<[u8; 4]> {
    let mut counts = [0; 4];
    let mut query=db.prepare("SELECT group_id,count FROM character_quest_kills WHERE account=?1 AND slot=?2 AND quest=?3")?;
    for row in query.query_map(params![account, slot, quest], |r| {
        Ok((r.get::<_, usize>(0)?, r.get::<_, u8>(1)?))
    })? {
        let (group, count) = row?;
        counts[group - 1] = count;
    }
    Ok(counts)
}

pub(crate) fn reset_counts(
    db: &Connection,
    account: &str,
    slot: u8,
    quest: u16,
) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM character_quest_kills WHERE account=?1 AND slot=?2 AND quest=?3",
        params![account, slot, quest],
    )?;
    Ok(())
}

pub(crate) fn completed(
    db: &Connection,
    account: &str,
    slot: u8,
    quest: u16,
) -> rusqlite::Result<bool> {
    if quest == 0 {
        return Ok(true);
    }
    Ok(db
        .query_row(
            "SELECT state=2 FROM character_quests WHERE account=?1 AND slot=?2 AND quest=?3",
            params![account, slot, quest],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

pub(crate) fn fulfilled(
    db: &Connection,
    account: &str,
    slot: u8,
    states: &std::collections::HashMap<u16, u8>,
    quest: u16,
) -> anyhow::Result<bool> {
    if !states.get(&quest).is_some_and(|s| matches!(s, 1 | 3)) {
        return Ok(false);
    }
    let text: Option<String> = db.query_row("SELECT data FROM world_reference_rules WHERE table_name='QUEST_MONSTER' AND rule_id=?1",[quest],|r|r.get(0)).optional()?;
    let Some(text) = text else {
        return Ok(false);
    };
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let counts = counts(db, account, slot, quest)?;
    Ok((1..=4).all(|g| {
        let count = v[format!("sCount{g}")]
            .as_str()
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(0);
        counts[g - 1] >= count
    }))
}

pub(crate) fn monster_snapshot(
    db: &Connection,
    account: &str,
    slot: u8,
) -> rusqlite::Result<Vec<u8>> {
    monster_snapshot_for(db, account, slot, 0)
}

pub(crate) fn monster_snapshot_for(
    db: &Connection,
    account: &str,
    slot: u8,
    preferred: u16,
) -> rusqlite::Result<Vec<u8>> {
    let active:u16=db.query_row("SELECT COALESCE((SELECT q.quest FROM character_quests q JOIN world_reference_rules r ON r.table_name='QUEST_MONSTER' AND r.rule_id=q.quest WHERE q.account=?1 AND q.slot=?2 AND q.state IN (1,3) ORDER BY q.quest=?3 DESC,q.quest LIMIT 1),0)",params![account,slot,preferred],|r|r.get(0))?;
    let mut p = vec![0x64, 9, 1];
    p.extend(active.to_le_bytes());
    p.extend(counts(db, account, slot, active)?);
    Ok(p)
}

pub(crate) fn cancel(db: &Connection, account: &str, slot: u8, quest: u16) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    let state = tx
        .query_row(
            "SELECT state FROM character_quests WHERE account=?1 AND slot=?2 AND quest=?3",
            params![account, slot, quest],
            |r| r.get::<_, u8>(0),
        )
        .optional()?
        .unwrap_or(0);
    if state == 2 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    tx.execute("INSERT INTO character_quests VALUES (?1,?2,?3,4) ON CONFLICT(account,slot,quest) DO UPDATE SET state=4",params![account,slot,quest])?;
    let monster:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM world_reference_rules WHERE table_name='QUEST_MONSTER' AND rule_id=?1)",[quest],|r|r.get(0))?;
    if monster && matches!(state, 1 | 3) {
        reset_counts(&tx, account, slot, quest)?;
    }
    tx.commit()
}

pub(crate) fn kill(
    db: &Connection,
    account: &str,
    slot: u8,
    prototype: u16,
) -> anyhow::Result<Vec<Vec<u8>>> {
    let tx = db.unchecked_transaction()?;
    let mut q=tx.prepare("SELECT q.quest,r.data FROM character_quests q JOIN world_reference_rules r ON r.table_name='QUEST_MONSTER' AND r.rule_id=q.quest WHERE q.account=?1 AND q.slot=?2 AND q.state=1 ORDER BY q.quest")?;
    let active = q
        .query_map(params![account, slot], |r| {
            Ok((r.get::<_, u16>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(q);
    let mut packets = Vec::new();
    for (quest, text) in active {
        let v: serde_json::Value = serde_json::from_str(&text)?;
        let n = |key: String| {
            v[key]
                .as_str()
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(0)
        };
        for group in 1..=4 {
            if !(b'a'..=b'd').any(|part| n(format!("sNum{group}{}", part as char)) == prototype) {
                continue;
            }
            let cap = n(format!("sCount{group}")).min(255) as u8;
            let current = counts(&tx, account, slot, quest)?;
            let count = current[group - 1];
            if count < cap {
                // Select the native counter context before its ID-less increment.
                packets.push(monster_snapshot_for(&tx, account, slot, quest)?);
                tx.execute("INSERT INTO character_quest_kills VALUES (?1,?2,?3,?4,?5) ON CONFLICT(account,slot,quest,group_id) DO UPDATE SET count=excluded.count",params![account,slot,quest,group,count+1])?;
                packets.push(vec![0x64, 9, 2, group as u8, count + 1]);
            }
            break;
        }
    }
    tx.commit()?;
    Ok(packets)
}

pub(crate) fn snapshot(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<Vec<u8>> {
    db.execute(
        "INSERT INTO character_quests(account,slot,quest,state)
        SELECT account,slot,500,2 FROM lobby_characters WHERE account=?1 AND slot=?2
        ON CONFLICT(account,slot,quest) DO UPDATE SET state=2 WHERE state!=2",
        params![account, slot],
    )?;
    let mut statement = db.prepare(
        "SELECT quest,state FROM character_quests WHERE account=?1 AND slot=?2 ORDER BY quest",
    )?;
    let rows = statement
        .query_map(params![account, slot], |row| {
            Ok((row.get::<_, u16>(0)?, row.get::<_, u8>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut packet = vec![0x64, 1];
    packet.extend((rows.len() as u16).to_le_bytes());
    for (quest, state) in rows {
        packet.extend(quest.to_le_bytes());
        packet.push(state);
    }
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_kill_progress_migrates_once_and_is_never_shared_with_new_quests() {
        let Some((lobby, _)) = crate::world_service_tests::setup() else {
            return;
        };
        let db = lobby.db.lock().unwrap();
        db.execute_batch("DROP TABLE character_quest_kills;
            INSERT INTO character_quests VALUES('test',0,101,1),('test',0,32001,3);
            INSERT INTO world_reference_rules VALUES('QUEST_MONSTER',101,'{\"sNum1a\":\"700\",\"sCount1\":\"5\"}');").unwrap();
        initialize(&db).unwrap();
        assert_eq!(counts(&db, "test", 0, 101).unwrap(), [3, 0, 0, 0]);
        db.execute("INSERT INTO character_quests VALUES('test',0,102,1)", [])
            .unwrap();
        db.execute(
            "INSERT INTO world_reference_rules VALUES('QUEST_MONSTER',102,?1)",
            [r#"{"sNum1a":"701","sCount1":"5"}"#],
        )
        .unwrap();
        initialize(&db).unwrap();
        assert_eq!(counts(&db, "test", 0, 102).unwrap(), [0; 4]);
        assert_eq!(
            kill(&db, "test", 0, 700).unwrap().last().unwrap(),
            &[0x64, 9, 2, 1, 4]
        );
        assert_eq!(counts(&db, "test", 0, 101).unwrap(), [4, 0, 0, 0]);
        cancel(&db, "test", 0, 101).unwrap();
        initialize(&db).unwrap();
        assert_eq!(counts(&db, "test", 0, 101).unwrap(), [0; 4]);
        assert_eq!(counts(&db, "test", 0, 102).unwrap(), [0; 4]);
        let mut states = std::collections::HashMap::new();
        for id in 1000..1040 {
            states.insert(id, if id % 2 == 0 { 1 } else { 3 });
        }
        states.insert(500, 2);
        states.insert(32001, 1);
        assert!(check_capacity(&states, 1040).is_err());
        assert!(check_capacity(&states, 1000).is_ok());
        states.insert(1000, 4);
        assert!(check_capacity(&states, 1040).is_ok());
    }
    #[test]
    fn expanded_states_migrate_without_changing_completed_unlocks() {
        let Some((lobby, _)) = crate::world_service_tests::setup() else {
            return;
        };
        let db = lobby.db.lock().unwrap();
        db.execute_batch("DROP TABLE character_quests; CREATE TABLE character_quests(account TEXT,slot INTEGER,quest INTEGER,state INTEGER CHECK(state BETWEEN 0 AND 2),PRIMARY KEY(account,slot,quest)); INSERT INTO character_quests VALUES('test',0,500,2);").unwrap();
        initialize(&db).unwrap();
        initialize(&db).unwrap();
        cancel(&db, "test", 0, 101).unwrap();
        assert!(completed(&db, "test", 0, 500).unwrap());
        assert!(cancel(&db, "test", 0, 500).is_err());
        db.execute(
            "INSERT INTO character_quests VALUES('test',0,32001,100)",
            [],
        )
        .unwrap();
        assert_eq!(
            monster_snapshot(&db, "test", 0).unwrap(),
            [0x64, 9, 1, 0, 0, 0, 0, 0, 0]
        );
    }
}
