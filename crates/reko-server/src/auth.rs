//! First-login registration and persisted password verification.
use crate::lobby::Lobby;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{net::IpAddr, sync::Arc};

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS account_passwords (
        account TEXT PRIMARY KEY COLLATE NOCASE REFERENCES lobby_accounts(name),
        password_hash TEXT NOT NULL);",
    )
}

fn credentials(payload: &[u8]) -> Option<(String, Vec<u8>)> {
    if payload.first() != Some(&0xf3) || payload.len() < 5 {
        return None;
    }
    let size = usize::from(u16::from_le_bytes(payload[1..3].try_into().ok()?));
    if !(1..=32).contains(&size) {
        return None;
    }
    let name = payload.get(3..3 + size)?;
    if !name.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_') {
        return None;
    }
    let at = 3 + size;
    let length = usize::from(u16::from_le_bytes(
        payload.get(at..at + 2)?.try_into().ok()?,
    ));
    if !(1..=28).contains(&length) || payload.len() != at + 2 + length {
        return None;
    }
    let password = payload.get(at + 2..)?;
    if password.contains(&0) {
        return None;
    }
    Some((
        String::from_utf8(name.to_ascii_lowercase()).ok()?,
        password.to_vec(),
    ))
}

fn stored(db: &Connection, account: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT password_hash FROM account_passwords WHERE account=?1",
        [account],
        |r| r.get(0),
    )
    .optional()
}
fn verify(hash: &str, password: &[u8]) -> anyhow::Result<bool> {
    let parsed = PasswordHash::new(hash)?;
    match Argon2::default().verify_password(password, &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn authenticate(lobby: &Lobby, account: &str, password: &[u8]) -> anyhow::Result<bool> {
    let existing = stored(&lobby.db.lock().unwrap(), account)?;
    if let Some(hash) = existing {
        return verify(&hash, password);
    }
    let salt = SaltString::generate(&mut OsRng);
    let candidate = Argon2::default()
        .hash_password(password, &salt)?
        .to_string();
    let actual = {
        let db = lobby.db.lock().unwrap();
        let tx = db.unchecked_transaction()?;
        crate::characters::ensure_account(&tx, account)?;
        tx.execute(
            "INSERT OR IGNORE INTO account_passwords VALUES (?1,?2)",
            params![account, candidate],
        )?;
        let actual = stored(&tx, account)?
            .ok_or_else(|| anyhow::anyhow!("password registration missing"))?;
        tx.commit()?;
        actual
    };
    // A concurrent first login may have registered a different password.
    if actual == candidate {
        Ok(true)
    } else {
        verify(&actual, password)
    }
}

pub(crate) async fn login(lobby: Arc<Lobby>, ip: IpAddr, payload: &[u8]) -> u8 {
    lobby.clear_login(ip);
    let Some((account, password)) = credentials(payload) else {
        return 3;
    };
    let Ok(permit) = lobby.login_slots.clone().try_acquire_owned() else {
        return 6;
    };
    let work = lobby.clone();
    let name = account.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        authenticate(&work, &name, &password)
    })
    .await;
    match result {
        Ok(Ok(true)) => match lobby.record_login(ip, &account) {
            Ok(()) => 1,
            Err(error) => {
                tracing::error!(%error,"authenticated login association failed");
                6
            }
        },
        Ok(Ok(false)) => 3,
        error => {
            tracing::error!(?error, "password authentication unavailable");
            6
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lobby(db: Connection) -> Arc<Lobby> {
        Arc::new(
            Lobby::new(
                db,
                crate::characters::CreationRules::fixture(),
                crate::world_entry::Bootstrap { next_level_exp: 50 },
            )
            .unwrap(),
        )
    }
    fn packet(account: &str, password: &[u8]) -> Vec<u8> {
        let mut p = vec![0xf3];
        p.extend((account.len() as u16).to_le_bytes());
        p.extend(account.as_bytes());
        p.extend((password.len() as u16).to_le_bytes());
        p.extend(password);
        p
    }
    #[tokio::test]
    async fn first_password_is_persisted_case_sensitive_and_failed_login_cannot_claim() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("reko-auth-{}-{unique}.db", std::process::id()));
        let ip = "127.0.0.1".parse().unwrap();
        {
            let l = lobby(Connection::open(&path).unwrap());
            crate::characters::ensure_account(&l.db.lock().unwrap(), "Legacy").unwrap();
            assert_eq!(
                login(l.clone(), ip, &packet("LEGACY", b"FirstPassword")).await,
                1
            );
            assert_eq!(l.claim_login(ip).as_deref(), Some("legacy"));
            let hash = stored(&l.db.lock().unwrap(), "legacy").unwrap().unwrap();
            assert!(hash.starts_with("$argon2id$"));
            assert!(!hash.contains("FirstPassword"));
            assert_eq!(
                login(l.clone(), ip, &packet("legacy", b"FirstPassword")).await,
                1
            );
            assert_eq!(
                login(l.clone(), ip, &packet("legacy", b"firstpassword")).await,
                3
            );
            assert!(l.claim_login(ip).is_none());
            assert_eq!(
                stored(&l.db.lock().unwrap(), "legacy").unwrap().as_deref(),
                Some(hash.as_str())
            );
        }
        {
            let l = lobby(Connection::open(&path).unwrap());
            assert_eq!(
                login(l.clone(), ip, &packet("Legacy", b"FirstPassword")).await,
                1
            );
            assert_eq!(
                login(l.clone(), ip, &packet("Legacy", b"OtherPassword")).await,
                3
            );
            assert!(l.claim_login(ip).is_none());
            assert_eq!(
                login(l.clone(), ip, &packet("Other", b"FirstPassword")).await,
                1
            );
            let db = l.db.lock().unwrap();
            assert_ne!(
                stored(&db, "legacy").unwrap(),
                stored(&db, "other").unwrap()
            );
        }
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn invalid_packets_register_nothing_and_concurrent_registration_never_overwrites() {
        let l = lobby(Connection::open_in_memory().unwrap());
        let ip = "127.0.0.1".parse().unwrap();
        let valid = packet("NewUser", b"password");
        for end in 0..valid.len() {
            assert_eq!(login(l.clone(), ip, &valid[..end]).await, 3);
        }
        for invalid in [
            packet("", b"pass"),
            packet("bad'name", b"pass"),
            packet("NewUser", b""),
            packet("NewUser", &[b'a'; 29]),
            packet("NewUser", b"a\0b"),
            [valid.clone(), vec![0]].concat(),
        ] {
            assert_eq!(login(l.clone(), ip, &invalid).await, 3);
        }
        assert_eq!(
            l.db.lock()
                .unwrap()
                .query_row("SELECT count(*) FROM lobby_accounts", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
        let a = packet("NewUser", b"One");
        let b = packet("NEWUSER", b"Two");
        let (first, second) = tokio::join!(login(l.clone(), ip, &a), login(l.clone(), ip, &b));
        assert!(matches!((first, second), (1, 3) | (3, 1)));
        let hash = stored(&l.db.lock().unwrap(), "newuser").unwrap().unwrap();
        assert_eq!(verify(&hash, b"One").unwrap(), first == 1);
        assert_eq!(verify(&hash, b"Two").unwrap(), second == 1);
    }
}
