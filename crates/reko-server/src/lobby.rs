//! Temporary login-to-game association until the v1453 account token is recovered.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;

struct PendingLogin {
    account: String,
    issued: Instant,
}

pub(crate) struct Lobby {
    pub advertised_ip: Option<std::net::Ipv4Addr>,
    pub social: Arc<Mutex<super::social::Hub>>,
    pub merchants: Vec<super::shops::Merchant>,
    pub db: Mutex<Connection>,
    pub rules: super::characters::CreationRules,
    pub bootstrap: super::world_entry::Bootstrap,
    pub chat: tokio::sync::broadcast::Sender<super::chat::Message>,
    pub starters: Option<super::starter::Catalog>,
    pub skills: Option<super::skills::Catalog>,
    pub combat: Option<super::combat::Catalog>,
    pub monsters: Option<std::sync::Arc<super::monsters::World>>,
    pub login_slots: Arc<tokio::sync::Semaphore>,
    pending: Mutex<HashMap<IpAddr, Vec<PendingLogin>>>,
}

impl Lobby {
    pub fn new(
        db: Connection,
        rules: super::characters::CreationRules,
        bootstrap: super::world_entry::Bootstrap,
    ) -> rusqlite::Result<Self> {
        super::database::initialize(&db)?;
        Ok(Self {
            advertised_ip: None,
            social: Arc::new(Mutex::new(super::social::Hub::default())),
            merchants: Vec::new(),
            db: Mutex::new(db),
            rules,
            bootstrap,
            chat: tokio::sync::broadcast::channel(64).0,
            starters: None,
            skills: None,
            combat: None,
            monsters: None,
            login_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn record_login(&self, ip: IpAddr, account: &str) -> rusqlite::Result<()> {
        if account.is_empty()
            || account.len() > 32
            || !account
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Ok(());
        }
        let account = account.to_ascii_lowercase();
        super::characters::ensure_account(&self.db.lock().unwrap(), &account)?;
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, entries| {
            entries.retain(|entry| entry.issued.elapsed() < Duration::from_secs(60));
            !entries.is_empty()
        });
        let entries = pending.entry(ip).or_default();
        entries.retain(|entry| entry.account != account);
        if entries.len() < super::MAX_PLAYERS {
            entries.push(PendingLogin {
                account,
                issued: Instant::now(),
            });
        }
        Ok(())
    }

    pub fn claim_login(&self, ip: IpAddr) -> Option<String> {
        let mut entries = self.pending.lock().unwrap().remove(&ip)?;
        entries.retain(|entry| entry.issued.elapsed() < Duration::from_secs(60));
        // TODO(re): Verify game-token binding for simultaneous logins behind one IP.
        if entries.len() == 1 {
            entries.pop().map(|entry| entry.account)
        } else {
            None
        }
    }

    pub fn clear_login(&self, ip: IpAddr) {
        self.pending.lock().unwrap().remove(&ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn association_is_single_use_and_rejects_ambiguity_and_expiry() {
        let lobby = Lobby::new(
            Connection::open_in_memory().unwrap(),
            super::super::characters::CreationRules::fixture(),
            super::super::world_entry::Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        let ip = "127.0.0.1".parse().unwrap();
        lobby.record_login(ip, "ADMIN").unwrap();
        assert_eq!(lobby.claim_login(ip).as_deref(), Some("admin"));
        assert_eq!(lobby.claim_login(ip), None);
        lobby.record_login(ip, "admin").unwrap();
        lobby.record_login(ip, "other").unwrap();
        assert_eq!(lobby.claim_login(ip), None);
        lobby.record_login(ip, "admin").unwrap();
        lobby.pending.lock().unwrap().get_mut(&ip).unwrap()[0].issued =
            Instant::now() - Duration::from_secs(61);
        assert_eq!(lobby.claim_login(ip), None);
    }
}
