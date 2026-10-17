//! Character-owned opaque native shortcut records; not learned skill state.
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS character_hotkeys (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        entries BLOB NOT NULL CHECK(length(entries)<=256 AND length(entries)%4=0),
        PRIMARY KEY(account,slot),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);",
    )
}

pub(crate) fn process(
    db: &Connection,
    account: &str,
    slot: u8,
    p: &[u8],
) -> anyhow::Result<Option<Vec<u8>>> {
    anyhow::ensure!(p.len() >= 2 && p[0] == 0x79, "invalid skillbar packet");
    let owned: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM lobby_characters WHERE account=?1 AND slot=?2)",
        params![account, slot],
        |r| r.get(0),
    )?;
    anyhow::ensure!(owned, "skillbar character not owned");
    match p[1] {
        1 => {
            anyhow::ensure!(p.len() >= 4, "short skillbar save");
            let count = usize::from(u16::from_le_bytes([p[2], p[3]]));
            anyhow::ensure!(
                count <= 64 && p.len() == 4 + count * 4,
                "invalid skillbar count"
            );
            db.execute("INSERT INTO character_hotkeys VALUES (?1,?2,?3) ON CONFLICT(account,slot) DO UPDATE SET entries=excluded.entries",params![account,slot,&p[4..]])?;
            tracing::info!(account, slot, count, "skillbar saved");
            Ok(None)
        }
        2 => {
            anyhow::ensure!(p.len() == 2, "invalid skillbar load");
            let entries: Option<Vec<u8>> = db
                .query_row(
                    "SELECT entries FROM character_hotkeys WHERE account=?1 AND slot=?2",
                    params![account, slot],
                    |r| r.get(0),
                )
                .optional()?;
            // Preserve the client's existing local layout until its first save.
            let Some(entries) = entries else {
                return Ok(None);
            };
            let mut packet = vec![0x79, 2];
            packet.extend((entries.len() as u16 / 4).to_le_bytes());
            packet.extend(entries);
            tracing::info!(
                account,
                slot,
                count = (packet.len() - 4) / 4,
                "skillbar loaded"
            );
            Ok(Some(packet))
        }
        _ => anyhow::bail!("unsupported skillbar operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_survive_reopen_are_character_scoped_and_reject_malformed_saves() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("reko-hotkeys-{}-{unique}.db", std::process::id()));
        let mut save = vec![0x79, 1, 64, 0];
        for bar in 0..8u32 {
            for (slot, skill) in [
                108670u32, 108655, 108635, 108640, 108645, 108725, 490014, 490020,
            ]
            .into_iter()
            .enumerate()
            {
                // Native encoding: decimal bar/slot digits followed by six-digit skill ID.
                save.extend(((bar * 10 + slot as u32) * 1_000_000 + skill).to_le_bytes());
            }
        }
        let mut expected = save.clone();
        expected[1] = 2;
        {
            let db = Connection::open(&path).unwrap();
            crate::characters::initialize(&db).unwrap();
            initialize(&db).unwrap();
            for account in ["one", "two"] {
                crate::characters::ensure_account(&db, account).unwrap();
            }
            for (account, slot, name) in [
                ("one", 0, "First"),
                ("one", 1, "Second"),
                ("two", 0, "Other"),
            ] {
                db.execute(
                    "INSERT INTO lobby_characters VALUES (?1,?2,?3,2,102,0,0,?4)",
                    params![account, slot, name, [60u8, 60, 80, 50, 50].as_slice()],
                )
                .unwrap();
            }
            assert_eq!(process(&db, "one", 0, &[0x79, 2]).unwrap(), None);
            assert!(process(&db, "one", 0, &save).unwrap().is_none());
            assert_eq!(
                process(&db, "ONE", 0, &[0x79, 2]).unwrap(),
                Some(expected.clone())
            );
            assert_eq!(process(&db, "one", 1, &[0x79, 2]).unwrap(), None);
            assert_eq!(process(&db, "two", 0, &[0x79, 2]).unwrap(), None);
            for end in 0..save.len() {
                assert!(process(&db, "one", 0, &save[..end]).is_err());
            }
            for p in [
                vec![0x79, 1, 65, 0],
                vec![0x79, 2, 0],
                vec![0x79, 3],
                [save.clone(), vec![0]].concat(),
            ] {
                assert!(process(&db, "one", 0, &p).is_err());
            }
            assert!(process(&db, "unknown", 0, &save).is_err());
            assert_eq!(
                process(&db, "one", 0, &[0x79, 2]).unwrap(),
                Some(expected.clone())
            );
        }
        {
            let db = Connection::open(&path).unwrap();
            initialize(&db).unwrap();
            assert_eq!(process(&db, "one", 0, &[0x79, 2]).unwrap(), Some(expected));
            assert!(process(&db, "one", 0, &[0x79, 1, 0, 0]).unwrap().is_none());
            assert_eq!(
                process(&db, "one", 0, &[0x79, 2]).unwrap(),
                Some(vec![0x79, 2, 0, 0])
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
