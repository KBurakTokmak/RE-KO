//! Shared schema initialization for runtime and offline database preparation.
use rusqlite::Connection;

pub const SCHEMA_VERSION: u32 = 1;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "database schema {version} is newer than this server ({SCHEMA_VERSION}); update the checkout"
        )));
    }
    crate::characters::initialize(db)?;
    crate::costumes::initialize(db)?;
    crate::auth::initialize(db)?;
    crate::hotkeys::initialize(db)?;
    crate::admin::initialize(db)?;
    crate::movement::initialize(db)?;
    crate::skills::initialize(db)?;
    crate::quest_rewards::initialize(db)?;
    crate::stat_points::initialize(db)?;
    crate::shops::initialize(db)?;
    crate::population::initialize(db)?;
    crate::bosses::initialize(db)?;
    crate::clans::initialize(db)?;
    crate::loot::initialize(db)?;
    db.execute_batch(include_str!("npc_ai.sql"))?;
    crate::quests::initialize(db)?;
    db.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_fresh_and_legacy_saves_without_resetting_accounts() {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "preserve").unwrap();
        db.pragma_update(None, "user_version", 0).unwrap();
        initialize(&db).unwrap();
        initialize(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT name FROM lobby_accounts", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "preserve"
        );
        db.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(initialize(&db).is_err());
    }
}
