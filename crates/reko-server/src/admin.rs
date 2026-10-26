//! Shared character-level administrative privilege, independent of account names.
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) const BASIC_ATTACK_DAMAGE: u16 = 30_000;

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS character_privileges (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        is_admin INTEGER NOT NULL DEFAULT 0 CHECK(is_admin IN (0,1)),
        PRIMARY KEY(account,slot),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);",
    )?;
    let legacy: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='character_test_permissions')", [], |r| r.get(0))?;
    if legacy {
        db.execute_batch(
            "INSERT INTO character_privileges(account,slot,is_admin)
            SELECT account,slot,1 FROM character_test_permissions WHERE self_damage=1
            ON CONFLICT(account,slot) DO NOTHING;
            DROP TABLE character_test_permissions;",
        )?;
    }
    Ok(())
}

pub(crate) fn is_admin(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<bool> {
    Ok(db
        .query_row(
            "SELECT p.is_admin=1 FROM character_privileges p
        JOIN lobby_characters c ON c.account=p.account AND c.slot=p.slot
        WHERE p.account=?1 AND p.slot=?2",
            params![account, slot],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_permission_migrates_once_and_admin_can_be_revoked() {
        let db = Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "test").unwrap();
        crate::characters::set_nation(&db, "test", 1).unwrap();
        db.execute(
            "INSERT INTO lobby_characters VALUES ('test',0,'RoleTest',2,108,0,0,?1)",
            [[60u8, 60, 70, 50, 50].as_slice()],
        )
        .unwrap();
        db.execute_batch("CREATE TABLE character_test_permissions(account TEXT,slot INTEGER,self_damage INTEGER); INSERT INTO character_test_permissions VALUES ('test',0,1);").unwrap();
        initialize(&db).unwrap();
        assert!(is_admin(&db, "TEST", 0).unwrap());
        assert!(!is_admin(&db, "test", 1).unwrap());
        db.execute("UPDATE character_privileges SET is_admin=0", [])
            .unwrap();
        initialize(&db).unwrap();
        assert!(!is_admin(&db, "test", 0).unwrap());
        assert!(
            db.prepare("SELECT * FROM character_test_permissions")
                .is_err()
        );
    }
}
