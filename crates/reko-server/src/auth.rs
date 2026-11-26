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

fn parse_credentials(
    payload: &[u8],
    opcode: u8,
    allow_game_tail: bool,
) -> Option<(String, Vec<u8>)> {
    if payload.first() != Some(&opcode) || payload.len() < 5 {
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
    let end = at + 2 + length;
    if !(1..=28).contains(&length) || payload.len() < end {
        return None;
    }
    if payload.len() != end && (!allow_game_tail || payload.len() > 256) {
        return None;
    }
    let password = payload.get(at + 2..end)?;
    if password.contains(&0) {
        return None;
    }
    Some((
        String::from_utf8(name.to_ascii_lowercase()).ok()?,
        password.to_vec(),
    ))
}

fn credentials(payload: &[u8]) -> Option<(String, Vec<u8>)> {
    parse_credentials(payload, 0xf3, false)
}

fn verify_existing(lobby: &Lobby, account: &str, password: &[u8]) -> anyhow::Result<bool> {
    let Some(hash) = stored(&lobby.db.lock().unwrap(), account)? else {
        return Ok(false);
    };
    verify(&hash, password)
}

pub(crate) async fn game_login(lobby: Arc<Lobby>, payload: &[u8]) -> Option<String> {
    let Some((account, password)) = parse_credentials(payload, 0x01, true) else {
        let first_len = payload
            .get(1..3)
            .and_then(|b| b.try_into().ok())
            .map(u16::from_le_bytes);
        let second_len = first_len.and_then(|n| {
            let at = 3 + usize::from(n);
            payload
                .get(at..at + 2)
                .and_then(|b| b.try_into().ok())
                .map(u16::from_le_bytes)
        });
        tracing::warn!(
            payload_len = payload.len(),
            ?first_len,
            ?second_len,
            "WIZ_LOGIN credential parser rejected payload shape"
        );
        return None;
    };
    let permit = lobby.login_slots.clone().try_acquire_owned().ok()?;
    let work = lobby.clone();
    let name = account.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        verify_existing(&work, &name, &password)
    })
    .await;
    match result {
        Ok(Ok(true)) => Some(account),
        Ok(Ok(false)) => {
            tracing::warn!("WIZ_LOGIN password verification failed");
            None
        }
        error => {
            tracing::error!(?error, "game login password authentication unavailable");
            None
        }
    }
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

pub(crate) async fn login_account(
    lobby: Arc<Lobby>,
    ip: IpAddr,
    payload: &[u8],
) -> (u8, Option<String>) {
    lobby.clear_login(ip);
    let Some((account, password)) = credentials(payload) else {
        return (3, None);
    };
    let Ok(permit) = lobby.login_slots.clone().try_acquire_owned() else {
        return (6, None);
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
            Ok(()) => (1, Some(account)),
            Err(error) => {
                tracing::error!(%error,"authenticated login association failed");
                (6, None)
            }
        },
        Ok(Ok(false)) => (3, None),
        error => {
            tracing::error!(?error, "password authentication unavailable");
            (6, None)
        }
    }
}

#[cfg(test)]
pub(crate) async fn login(lobby: Arc<Lobby>, ip: IpAddr, payload: &[u8]) -> u8 {
    login_account(lobby, ip, payload).await.0
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

    #[tokio::test]
    async fn game_login_verifies_existing_accounts_with_native_tail_and_never_registers() {
        let l = lobby(Connection::open_in_memory().unwrap());
        let ip = "127.0.0.1".parse().unwrap();
        assert_eq!(login(l.clone(), ip, &packet("Admin", b"secret")).await, 1);

        let mut wiz = vec![0x01];
        wiz.extend((5u16).to_le_bytes());
        wiz.extend(b"ADMIN");
        wiz.extend((6u16).to_le_bytes());
        wiz.extend(b"secret");
        // Native 1453 WIZ_LOGIN tail: u16 + u8 length + token bytes.
        wiz.extend((0u16).to_le_bytes());
        wiz.push(4);
        wiz.extend(b"tok1");

        assert_eq!(game_login(l.clone(), &wiz).await.as_deref(), Some("admin"));
        // Without native tail is also accepted.
        assert_eq!(
            game_login(l.clone(), &wiz[..wiz.len() - 7])
                .await
                .as_deref(),
            Some("admin")
        );
        // Oversized trailing tail beyond 256 bytes is rejected.
        let mut bad_tail = wiz.clone();
        bad_tail.resize(257, 0);
        assert_eq!(game_login(l.clone(), &bad_tail).await, None);
        // Wrong password is rejected.
        let mut wrong = vec![0x01];
        wrong.extend((5u16).to_le_bytes());
        wrong.extend(b"admin");
        wrong.extend((5u16).to_le_bytes());
        wrong.extend(b"wrong");
        assert_eq!(game_login(l.clone(), &wrong).await, None);
        // Unregistered account is rejected and never created on game port.
        let mut fresh = vec![0x01];
        fresh.extend((5u16).to_le_bytes());
        fresh.extend(b"ghost");
        fresh.extend((6u16).to_le_bytes());
        fresh.extend(b"secret");
        assert_eq!(game_login(l.clone(), &fresh).await, None);
        assert_eq!(stored(&l.db.lock().unwrap(), "ghost").unwrap(), None);
    }
}
