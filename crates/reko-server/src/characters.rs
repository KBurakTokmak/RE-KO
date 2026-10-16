//! Character creation and selection storage for the v1453 lobby.

use reko_assets::{CellValue, Table};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;

pub(crate) struct CreationRules {
    entries: HashMap<u32, ([u8; 5], u16)>,
}

impl CreationRules {
    pub fn load(game_dir: &Path) -> anyhow::Result<Self> {
        let table =
            Table::from_encrypted_bytes(&std::fs::read(game_dir.join("Data/NewChrValue.tbl"))?)?;
        Self::from_table(&table)
    }

    fn from_table(table: &Table) -> anyhow::Result<Self> {
        let mut entries = HashMap::new();
        for row in &table.rows {
            let number = |index: usize| -> anyhow::Result<u32> {
                match row.get(index) {
                    Some(CellValue::U32(value)) => Ok(*value),
                    Some(CellValue::I32(value)) if *value >= 0 => Ok(*value as u32),
                    _ => anyhow::bail!("invalid NewChrValue numeric column {index}"),
                }
            };
            let mut base = [0; 5];
            for (index, stat) in base.iter_mut().enumerate() {
                *stat = number(index + 2)?.try_into()?;
            }
            entries.insert(number(0)?, (base, number(7)?.try_into()?));
        }
        anyhow::ensure!(!entries.is_empty(), "NewChrValue has no creation rules");
        Ok(Self { entries })
    }

    pub fn validate(&self, character: &Character) -> Result<(), u8> {
        let key = u32::from(character.race) * 10_000 + u32::from(character.class);
        let (base, bonus) = self.entries.get(&key).ok_or(9u8)?;
        let mut allocated = 0u16;
        for (&stat, &minimum) in character.stats.iter().zip(base) {
            allocated += u16::from(stat.checked_sub(minimum).ok_or(10u8)?);
        }
        if allocated != *bonus {
            return Err(10);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn fixture() -> Self {
        Self {
            entries: HashMap::from([
                (20102, ([60, 60, 70, 50, 50], 10)),
                (10101, ([65, 65, 60, 50, 50], 10)),
            ]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Character {
    pub slot: u8,
    pub name: String,
    pub race: u8,
    pub class: u16,
    pub face: u8,
    pub hair: u8,
    pub stats: [u8; 5],
}

impl Character {
    pub fn parse(payload: &[u8]) -> Result<Self, u8> {
        if payload.len() < 4 || payload[0] != 0x02 {
            return Err(11);
        }
        let slot = payload[1];
        if slot > 2 {
            return Err(1);
        }
        let name_len = u16::from_le_bytes([payload[2], payload[3]]) as usize;
        if name_len == 0 {
            return Err(6);
        }
        if name_len > 20 {
            return Err(5);
        }
        if payload.len() != 4 + name_len + 10 {
            return Err(11);
        }
        let name_bytes = &payload[4..4 + name_len];
        if !name_bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            return Err(5);
        }
        let fields = &payload[4 + name_len..];
        let character = Self {
            slot,
            name: String::from_utf8(name_bytes.to_vec()).map_err(|_| 5u8)?,
            race: fields[0],
            class: u16::from_le_bytes([fields[1], fields[2]]),
            face: fields[3],
            hair: fields[4],
            stats: fields[5..10].try_into().map_err(|_| 11u8)?,
        };
        if !matches!(character.race, 1..=4 | 11..=13) {
            return Err(7);
        }
        let class_nation = character.class / 100;
        if !(1..=4).contains(&(character.class % 100)) {
            return Err(9);
        }
        if class_nation != character.nation() as u16 {
            return Err(2);
        }
        Ok(character)
    }

    pub fn nation(&self) -> u8 {
        if self.race < 10 { 1 } else { 2 }
    }

    pub fn put_slot(&self, payload: &mut Vec<u8>, items: &[crate::starter::Item; 14], level: u8) {
        payload.extend_from_slice(&(self.name.len() as u16).to_le_bytes());
        payload.extend_from_slice(self.name.as_bytes());
        payload.push(self.race);
        payload.extend_from_slice(&self.class.to_le_bytes());
        payload.extend_from_slice(&[level, self.face, self.hair, 21]);
        for index in [5, 4, 1, 6, 8, 10, 12, 13] {
            payload.extend_from_slice(&items[index].id.to_le_bytes());
            payload.extend_from_slice(&items[index].durability.to_le_bytes());
        }
    }
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS lobby_accounts (
            name TEXT PRIMARY KEY COLLATE NOCASE,
            nation INTEGER NOT NULL DEFAULT 1 CHECK(nation IN (1,2))
        );
        CREATE TABLE IF NOT EXISTS lobby_characters (
            account TEXT NOT NULL COLLATE NOCASE REFERENCES lobby_accounts(name),
            slot INTEGER NOT NULL CHECK(slot BETWEEN 0 AND 2),
            name TEXT NOT NULL COLLATE NOCASE UNIQUE,
            race INTEGER NOT NULL,
            class INTEGER NOT NULL,
            face INTEGER NOT NULL,
            hair INTEGER NOT NULL,
            stats BLOB NOT NULL CHECK(length(stats) = 5),
            PRIMARY KEY(account, slot)
        );",
    )?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS account_server_nations (
        account TEXT NOT NULL COLLATE NOCASE REFERENCES lobby_accounts(name),
        server_id INTEGER NOT NULL, nation INTEGER NOT NULL CHECK(nation IN (1,2)),
        PRIMARY KEY(account,server_id));
        INSERT OR IGNORE INTO account_server_nations
        SELECT c.account,1,MIN(CASE WHEN c.race<10 THEN 1 ELSE 2 END)
        FROM lobby_characters c GROUP BY c.account;",
    )?;
    crate::starter::initialize(db)?;
    crate::progress::initialize(db)
}

pub(crate) fn ensure_account(db: &Connection, account: &str) -> rusqlite::Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO lobby_accounts(name) VALUES (?1)",
        [account],
    )?;
    Ok(())
}

pub(crate) fn nation(db: &Connection, account: &str) -> rusqlite::Result<u8> {
    db.query_row(
        "SELECT COALESCE((SELECT nation FROM account_server_nations WHERE account=?1 AND server_id=1),0)",
        [account],
        |row| row.get(0),
    )
}

pub(crate) fn set_nation(db: &Connection, account: &str, selected: u8) -> rusqlite::Result<bool> {
    if !matches!(selected, 1 | 2) {
        return Ok(false);
    }
    let tx = db.unchecked_transaction()?;
    let current = nation(&tx, account)?;
    if current != 0 {
        return Ok(current == selected);
    }
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM lobby_accounts WHERE name=?1)",
        [account],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO account_server_nations VALUES (?1,1,?2)",
        params![account, selected],
    )?;
    tx.execute(
        "UPDATE lobby_accounts SET nation=?2 WHERE name=?1",
        params![account, selected],
    )?;
    tx.commit()?;
    Ok(true)
}

pub(crate) fn create(
    db: &mut Connection,
    account: &str,
    character: &Character,
) -> rusqlite::Result<u8> {
    let transaction = db.transaction()?;
    let occupied: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM lobby_characters WHERE account = ?1 AND slot = ?2)",
        params![account, character.slot],
        |row| row.get(0),
    )?;
    if occupied {
        return Ok(1);
    }
    let duplicate: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM lobby_characters WHERE name = ?1)",
        [&character.name],
        |row| row.get(0),
    )?;
    if duplicate {
        return Ok(3);
    }
    if nation(&transaction, account)? != character.nation() {
        return Ok(2);
    }
    transaction.execute("INSERT INTO lobby_characters(account,slot,name,race,class,face,hair,stats) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![account, character.slot, character.name, character.race, character.class, character.face, character.hair, character.stats.as_slice()])?;
    transaction.commit()?;
    Ok(0)
}

pub(crate) fn list_payload(db: &Connection, account: Option<&str>) -> rusqlite::Result<Vec<u8>> {
    let mut payload = vec![0x0C, 1];
    for slot in 0u8..3 {
        let character = if let Some(account) = account {
            db.query_row("SELECT name,race,class,face,hair,stats FROM lobby_characters WHERE account = ?1 AND slot = ?2", params![account, slot], |row| {
                let stats: Vec<u8> = row.get(5)?;
                let stats = stats.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(Character { slot, name: row.get(0)?, race: row.get(1)?, class: row.get(2)?, face: row.get(3)?, hair: row.get(4)?, stats })
            }).optional()?
        } else {
            None
        };
        if let Some(character) = character {
            let items = crate::starter::equipment(db, account.unwrap(), slot)?;
            let progress = crate::progress::load(db, account.unwrap(), slot)?;
            character.put_slot(&mut payload, &items, progress.level);
        } else {
            payload.extend_from_slice(&[0; 57]);
        }
    }
    Ok(payload)
}

pub(crate) fn find(
    db: &Connection,
    account: &str,
    name: &str,
) -> rusqlite::Result<Option<Character>> {
    db.query_row("SELECT slot,name,race,class,face,hair,stats FROM lobby_characters WHERE account = ?1 AND name = ?2", params![account, name], |row| {
        let stats: Vec<u8> = row.get(6)?;
        let stats = stats.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Character { slot: row.get(0)?, name: row.get(1)?, race: row.get(2)?, class: row.get(3)?, face: row.get(4)?, hair: row.get(5)?, stats })
    }).optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nation_is_required_locked_before_creation_and_survives_empty_roster() {
        let mut db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        ensure_account(&db, "newuser").unwrap();
        assert_eq!(nation(&db, "newuser").unwrap(), 0);
        let karus = Character::parse(&captured()).unwrap();
        assert_eq!(create(&mut db, "newuser", &karus).unwrap(), 2);
        assert!(!set_nation(&db, "newuser", 0).unwrap());
        assert!(set_nation(&db, "NEWUSER", 2).unwrap());
        assert!(!set_nation(&db, "newuser", 1).unwrap());
        assert_eq!(create(&mut db, "newuser", &karus).unwrap(), 2);
        let mut elmorad = karus.clone();
        elmorad.race = 12;
        elmorad.class = 202;
        assert_eq!(create(&mut db, "newuser", &elmorad).unwrap(), 0);
        db.execute("DELETE FROM lobby_characters WHERE account='newuser'", [])
            .unwrap();
        initialize(&db).unwrap();
        assert_eq!(nation(&db, "newuser").unwrap(), 2);
        assert!(!set_nation(&db, "newuser", 1).unwrap());
        db.execute(
            "INSERT INTO account_server_nations VALUES ('newuser',2,1)",
            [],
        )
        .unwrap();
        assert_eq!(nation(&db, "newuser").unwrap(), 2);
        assert!(!set_nation(&db, "unknown", 1).unwrap());
    }

    #[test]
    fn legacy_accounts_keep_character_faction_while_empty_accounts_choose() {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        ensure_account(&db, "legacy").unwrap();
        ensure_account(&db, "empty").unwrap();
        db.execute(
            "INSERT INTO lobby_characters VALUES ('legacy',0,'OldElmorad',12,202,0,0,?1)",
            [[60u8, 60, 80, 50, 50].as_slice()],
        )
        .unwrap();
        initialize(&db).unwrap();
        assert_eq!(nation(&db, "legacy").unwrap(), 2);
        assert_eq!(nation(&db, "empty").unwrap(), 0);
        assert!(!set_nation(&db, "legacy", 1).unwrap());
    }

    fn captured() -> Vec<u8> {
        let mut packet = vec![2, 0, 10, 0];
        packet.extend_from_slice(b"AdminSiken");
        packet.extend_from_slice(&[2, 102, 0, 0, 0, 60, 60, 80, 50, 50]);
        packet
    }

    #[test]
    fn captured_creation_roundtrip_and_account_isolation() {
        let mut db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        ensure_account(&db, "admin").unwrap();
        set_nation(&db, "admin", 1).unwrap();
        ensure_account(&db, "other").unwrap();
        set_nation(&db, "other", 1).unwrap();
        let character = Character::parse(&captured()).unwrap();
        assert_eq!(character.class, 102);
        assert_eq!(create(&mut db, "admin", &character).unwrap(), 0);
        assert_eq!(create(&mut db, "admin", &character).unwrap(), 1);
        assert_eq!(create(&mut db, "other", &character).unwrap(), 3);
        let payload = list_payload(&db, Some("ADMIN")).unwrap();
        assert_eq!(&payload[..14], b"\x0c\x01\x0a\x00AdminSiken");
        assert_eq!(&payload[14..21], &[2, 102, 0, 1, 0, 0, 21]);
        assert_eq!(payload.len(), 183);
        assert_eq!(
            list_payload(&db, Some("other")).unwrap(),
            [vec![0x0C, 1], vec![0; 171]].concat()
        );
        assert!(!set_nation(&db, "admin", 2).unwrap());
    }

    #[test]
    fn selection_weapons_use_client_attachment_fields() {
        let character = Character::parse(&captured()).unwrap();
        let mut items = [crate::starter::Item::default(); 14];
        items[6] = crate::starter::Item {
            id: 110110001,
            durability: 4000,
            count: 1,
        };
        items[10] = crate::starter::Item {
            id: 206002001,
            durability: 2000,
            count: 1,
        };
        items[11] = crate::starter::Item {
            id: 330110002,
            durability: 1,
            count: 1,
        };
        items[8] = crate::starter::Item {
            id: 170150001,
            durability: 5000,
            count: 1,
        };
        let mut payload = vec![0x0C, 1];
        character.put_slot(&mut payload, &items, 1);
        // Client attachment fields receive the fourth and fifth item pairs.
        assert_eq!(&payload[39..45], &[0x31, 0x25, 0x90, 0x06, 0xA0, 0x0F]);
        assert_eq!(&payload[45..49], &170150001u32.to_le_bytes());
        assert_eq!(&payload[51..55], &206002001u32.to_le_bytes());
        assert!(payload[57..69].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn malformed_and_invalid_creation_is_rejected() {
        let packet = captured();
        for end in 0..packet.len() {
            assert!(Character::parse(&packet[..end]).is_err());
        }
        let mut extra = packet.clone();
        extra.push(0);
        assert!(Character::parse(&extra).is_err());
        for (offset, value, status) in [(1, 3, 1), (4, b' ', 5), (14, 99, 7), (15, 202, 2)] {
            let mut invalid = packet.clone();
            invalid[offset] = value;
            assert_eq!(Character::parse(&invalid), Err(status));
        }
    }

    #[test]
    fn creation_stats_and_combinations_follow_the_table() {
        let rules = CreationRules::fixture();
        let mut character = Character::parse(&captured()).unwrap();
        assert_eq!(rules.validate(&character), Ok(()));
        character.stats[2] = 79;
        assert_eq!(rules.validate(&character), Err(10));
        character.stats[2] = 81;
        assert_eq!(rules.validate(&character), Err(10));
        character.class = 101;
        assert_eq!(rules.validate(&character), Err(9));
    }

    #[test]
    fn rules_match_original_table_when_available() {
        let game_dir = reko_common::game_dir();
        if !game_dir.join("Data/NewChrValue.tbl").exists() {
            return;
        }
        let rules = CreationRules::load(&game_dir).unwrap();
        assert_eq!(rules.entries.len(), 15);
        let character = Character::parse(&captured()).unwrap();
        assert_eq!(rules.validate(&character), Ok(()));
        assert_eq!(rules.entries.get(&20102), Some(&([60, 60, 70, 50, 50], 10)));
    }

    #[test]
    fn character_survives_database_reopen() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("reko-character-{}-{unique}.db", std::process::id()));
        let expected;
        {
            let mut db = Connection::open(&path).unwrap();
            initialize(&db).unwrap();
            crate::movement::initialize(&db).unwrap();
            ensure_account(&db, "admin").unwrap();
            set_nation(&db, "admin", 1).unwrap();
            let character = Character::parse(&captured()).unwrap();
            assert_eq!(create(&mut db, "admin", &character).unwrap(), 0);
            crate::movement::save(
                &db,
                "admin",
                0,
                crate::movement::Position {
                    zone: 21,
                    x: 2800,
                    z: 3500,
                    y: 87,
                },
            )
            .unwrap();
            expected = list_payload(&db, Some("admin")).unwrap();
        }
        {
            let db = Connection::open(&path).unwrap();
            initialize(&db).unwrap();
            crate::movement::initialize(&db).unwrap();
            assert_eq!(list_payload(&db, Some("admin")).unwrap(), expected);
            assert_eq!(
                crate::movement::load(&db, "admin", 0).unwrap(),
                crate::movement::Position {
                    zone: 21,
                    x: 2800,
                    z: 3500,
                    y: 87
                }
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
