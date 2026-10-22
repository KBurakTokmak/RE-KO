//! Game server library. Hosted in-process (singleplayer) or by `reko-serverd` (LAN).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::Instrument;

mod abyss;
mod admin;
mod archery;
mod auth;
mod bifrost;
mod bosses;
mod chaotic;
mod characters;
mod chat;
mod clan_promotion;
mod clans;
mod combat;
#[cfg(test)]
mod combat_zone_tests;
pub mod crypto;
mod durability;
mod framing;
mod gm_spawn;
mod gm_teleport;
mod hemes;
mod hotkeys;
mod inventory;
#[cfg(test)]
mod inventory_service_tests;
mod lobby;
mod loot;
mod loot_generation;
mod mage;
mod mage_special;
#[cfg(test)]
mod mage_tests;
mod magic;
mod model_radius;
mod monsters;
mod movement;
mod npc_scripts;
mod party;
mod population;
mod priest;
#[cfg(test)]
mod priest_tests;
mod progress;
mod quest_journal;
mod quest_rewards;
mod quests;
mod resources;
#[cfg(test)]
mod rogue_tests;
mod shops;
mod skills;
mod social;
mod starter;
mod stat_points;
mod storage;
mod suppression;
mod town;
mod transformation;
mod travel;
mod upgrading;
mod utility;
mod wallet;
#[cfg(test)]
mod warrior_tests;
mod world_entry;
#[cfg(test)]
mod world_service_tests;

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

use crypto::{JvCrypt, KO_PUBLIC_KEY};

/// Login port the client dials.
pub const DEFAULT_LOGIN_PORT: u16 = 15100;

/// Game server port where the client connects after server selection.
pub const DEFAULT_GAME_PORT: u16 = 15001;

pub const MAX_PLAYERS: usize = 20;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub db_path: PathBuf,
    pub game_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], DEFAULT_LOGIN_PORT)),
            db_path: PathBuf::from("reko.db"),
            game_dir: reko_common::game_dir(),
        }
    }
}

/// Opens/creates the save DB.
pub fn open_db(cfg: &Config) -> rusqlite::Result<rusqlite::Connection> {
    let db = rusqlite::Connection::open(&cfg.db_path)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    Ok(db)
}

/// Formats a byte slice as a hex dump string.
pub fn format_hex(bytes: &[u8]) -> String {
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
    let ascii: String = bytes
        .iter()
        .map(|&b| {
            if (0x20..=0x7E).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect();
    format!("{} | {}", hex.join(" "), ascii)
}

// Wraps payload bytes in KO framing: 0xAA 0x55 [len: u16] [payload] 0x55 0xAA
pub fn frame_packet(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut out = Vec::with_capacity(payload.len() + 6);
    out.extend_from_slice(&[0xAA, 0x55]);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&[0x55, 0xAA]);
    out
}

// Encodes a length-prefixed string (u16 little-endian length + bytes).
fn put_short_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u16).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

// Builds a KO server list payload containing 1 server and 1 channel.
fn build_server_list_payload(opcode: u8) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(opcode);
    payload.push(1); // 1 server

    // Server IP
    put_short_str(&mut payload, "127.0.0.1");
    // Server name and channel (delimited by pipe)
    put_short_str(&mut payload, "RE-KO|Channel 1");
    // Stats: player count, free limit, premium limit
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&3000u16.to_le_bytes());
    payload.extend_from_slice(&3000u16.to_le_bytes());
    // King 1 (El Morad) and notice
    put_short_str(&mut payload, "");
    put_short_str(&mut payload, "");
    // King 2 (Karus) and notice
    put_short_str(&mut payload, "");
    put_short_str(&mut payload, "");

    payload
}

// Helper to send a response payload, encrypting if crypto is active.
async fn send_response(
    sock: &mut TcpStream,
    peer: SocketAddr,
    crypt: &Option<JvCrypt>,
    payload: &[u8],
    name: &str,
) {
    let body = if let Some(c) = crypt {
        c.encrypt_server_packet(payload)
    } else {
        payload.to_vec()
    };
    let packet = frame_packet(&body);
    tracing::info!(%peer, name, dump = %format_hex(&packet), "sending response packet");
    if let Err(e) = sock.write_all(&packet).await {
        tracing::error!(%peer, error = %e, "failed to send response packet");
    }
}

/// Handles an incoming login client connection, managing crypto state and protocol responses.
async fn handle_client(mut sock: TcpStream, peer: SocketAddr, lobby: Arc<lobby::Lobby>) {
    tracing::info!(%peer, "client connected to login server");
    let mut crypt: Option<JvCrypt> = None;
    let mut buf = [0u8; 4096];
    let mut decoder = framing::Decoder::default();
    loop {
        match sock.read(&mut buf).await {
            Ok(0) => {
                tracing::info!(%peer, "client disconnected (EOF)");
                break;
            }
            Ok(n) => {
                let data = &buf[..n];
                tracing::info!(%peer, bytes = n, "received login wire bytes");

                decoder.push(data);
                loop {
                    match decoder.next() {
                        Ok(Some(wire_payload)) => {
                            let mut dec_buf = wire_payload.to_vec();
                            let (dispatch_payload, is_decrypted) = if let Some(c) = &crypt {
                                if let Some(dec) = c.decrypt_client_packet(&mut dec_buf) {
                                    (dec.to_vec(), true)
                                } else {
                                    (wire_payload.clone(), false)
                                }
                            } else {
                                (wire_payload.clone(), false)
                            };

                            if is_decrypted && dispatch_payload.first() != Some(&0xF3) {
                                tracing::info!(%peer, dump = %format_hex(&dispatch_payload), "decrypted login packet");
                            }
                            handle_payload(&mut sock, peer, &mut crypt, &dispatch_payload, &lobby)
                                .await;
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::error!(%peer, error, pending_bytes = decoder.pending(), "invalid login frame; closing connection");
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(%peer, error = %e, "read error");
                break;
            }
        }
    }
}

// Builds a KO news payload (Title and Body strings).
fn build_news_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(0xF6); // opcode
    put_short_str(&mut payload, "Welcome to RE-KO");
    put_short_str(&mut payload, "Singleplayer and LAN server running.");
    payload
}

// Responds to known login server opcodes.
async fn handle_payload(
    sock: &mut TcpStream,
    peer: SocketAddr,
    crypt: &mut Option<JvCrypt>,
    payload: &[u8],
    lobby: &Arc<lobby::Lobby>,
) {
    if payload.is_empty() {
        return;
    }
    let opcode = payload[0];
    match opcode {
        // Opcode 0x01: LS_VERSION_CHECK
        0x01 => {
            let client_ver = if payload.len() >= 3 {
                u16::from_le_bytes([payload[1], payload[2]])
            } else {
                1453
            };
            tracing::info!(%peer, client_ver, "handling LS_VERSION_CHECK request");
            let response_payload = [0x01, (client_ver & 0xFF) as u8, (client_ver >> 8) as u8];
            send_response(sock, peer, &None, &response_payload, "LS_VERSION_CHECK").await;
        }
        // Opcode 0xF2: LS_CRYPTO / Key Exchange
        0xF2 => {
            tracing::info!(%peer, "handling LS_CRYPTO (0xF2) key exchange request");
            // Enable JvCryption for subsequent packets on this session
            *crypt = Some(JvCrypt::new(KO_PUBLIC_KEY));
            let mut response_payload = Vec::with_capacity(9);
            response_payload.push(0xF2);
            response_payload.extend_from_slice(&KO_PUBLIC_KEY.to_le_bytes());
            // Handshake response 0xF2 is sent unencrypted
            let packet = frame_packet(&response_payload);
            tracing::info!(%peer, dump = %format_hex(&packet), "sending LS_CRYPTO (0xF2) response");
            if let Err(e) = sock.write_all(&packet).await {
                tracing::error!(%peer, error = %e, "failed to send 0xF2 response");
            }
        }
        // Opcode 0xF3: LS_LOGIN
        0xF3 => {
            let result = auth::login(lobby.clone(), peer.ip(), payload).await;
            tracing::info!(%peer,result,"login authentication result");
            let response = if result == 1 {
                vec![0xF3, 1, 0xAD, 5, 1, 0]
            } else {
                vec![0xF3, result]
            };
            send_response(sock, peer, crypt, &response, "LS_LOGIN (0xF3)").await;
        }
        // Opcode 0xF4: LS_MGAME_LOGIN
        0xF4 => {
            tracing::info!(%peer, "handling LS_MGAME_LOGIN (0xF4) request");
            let response_payload = [0xF4, 0x01, 0xAD, 0x05, 0x01, 0x00];
            send_response(
                sock,
                peer,
                crypt,
                &response_payload,
                "LS_MGAME_LOGIN (0xF4)",
            )
            .await;
        }
        // Opcode 0x05 or 0xF5: LS_SERVERLIST
        0x05 | 0xF5 => {
            tracing::info!(%peer, opcode = %format!("0x{opcode:02X}"), "handling LS_SERVERLIST request");
            let response_payload = build_server_list_payload(opcode);
            send_response(sock, peer, crypt, &response_payload, "LS_SERVERLIST").await;
        }
        // Opcode 0xF6: LS_NEWS
        0xF6 => {
            tracing::info!(%peer, "handling LS_NEWS (0xF6) request");
            let response_payload = build_news_payload();
            send_response(sock, peer, crypt, &response_payload, "LS_NEWS (0xF6)").await;
        }
        _ => {
            tracing::warn!(%peer, opcode = %format!("0x{opcode:02X}"), dump = %format_hex(payload), "unhandled opcode");
        }
    }
}

/// Responds to incoming game server opcodes.
async fn handle_game_payload(
    sock: &mut TcpStream,
    peer: SocketAddr,
    payload: &[u8],
    lobby: &lobby::Lobby,
    account: &mut Option<String>,
    world: &mut Option<world_entry::Session>,
) {
    if let (Some(account), Some(s)) = (account.as_deref(), world.as_mut()) {
        clan_promotion::transfer(sock, peer, lobby, s).await;
        social::synchronize(lobby, account, s);
        social::flush(sock, peer, lobby, s).await;
    }
    handle_game_payload_inner(sock, peer, payload, lobby, account, world).await;
    if let (Some(account), Some(s)) = (account.as_deref(), world.as_mut()) {
        social::synchronize(lobby, account, s);
        social::flush(sock, peer, lobby, s).await;
    }
}
async fn handle_game_payload_inner(
    sock: &mut TcpStream,
    peer: SocketAddr,
    payload: &[u8],
    lobby: &lobby::Lobby,
    account: &mut Option<String>,
    world: &mut Option<world_entry::Session>,
) {
    if payload.is_empty() {
        return;
    }
    let opcode = payload[0];
    if let Some(s) = world.as_ref().filter(|s| s.entered) {
        let frozen = s
            .skills
            .buffs
            .get(&22)
            .is_some_and(|b| skills::now_ms() < b.until);
        let blinded = s
            .skills
            .buffs
            .get(&20)
            .is_some_and(|b| skills::now_ms() < b.until);
        if frozen && opcode == 6 {
            let mut correction = vec![0x1e];
            correction.extend(s.position.x.to_le_bytes());
            correction.extend(s.position.z.to_le_bytes());
            send_response(sock, peer, &None, &correction, "FROZEN_POSITION").await;
            return;
        }
        if blinded
            && opcode == 0x31
            && payload.len() == 22
            && payload[1] != 4
            && ![s.player_id, u16::MAX].contains(&u16::from_le_bytes([payload[8], payload[9]]))
        {
            magic::refused(
                sock,
                peer,
                s,
                u32::from_le_bytes(payload[2..6].try_into().unwrap()),
                "You cannot select an attack target while blinded.",
            )
            .await;
            return;
        }
    }
    match opcode {
        0x16 => {
            if let Some(s) = world.as_ref().filter(|s| s.entered)
                && payload.len() >= 3
            {
                let count = usize::from(u16::from_le_bytes([payload[1], payload[2]]));
                if count <= MAX_PLAYERS && payload.len() == 3 + 2 * count {
                    let mut h = lobby.social.lock().unwrap();
                    let mut seen = std::collections::HashSet::new();
                    let mut records = Vec::new();
                    for bytes in payload[3..].as_chunks::<2>().0 {
                        let id = u16::from_le_bytes(*bytes);
                        if id != s.player_id
                            && seen.insert(id)
                            && h.players.get(&id).is_some_and(|p| {
                                !p.s.zone_loading
                                    && population::same_region_view(p.s.position, s.position)
                            })
                        {
                            records.push(h.user_in(id)[2..].to_vec());
                        }
                    }
                    let mut p = vec![0x16];
                    p.extend((records.len() as u16).to_le_bytes());
                    for record in records {
                        p.extend(record);
                    }
                    h.send(s.player_id, p);
                }
            }
        }
        0x2f => {
            if let Some(s) = world.as_ref().filter(|s| s.entered) {
                party::process(&mut lobby.social.lock().unwrap(), s.player_id, payload);
            }
        }
        0x3c => {
            if let (Some(account), Some(s)) =
                (account.as_deref(), world.as_mut().filter(|s| s.entered))
            {
                clans::process(lobby, account, s, payload);
            }
        }
        // Opcode 0x2B: WIZ_VERSION_CHECK
        0x2B => {
            tracing::info!(%peer, dump = %format_hex(payload), "handling game server WIZ_VERSION_CHECK (0x2B)");
            // KnightOnline1453 (0x0075EE10):
            // byte 0: 0x2B opcode
            // byte 1..2: u16 version (1453 = 0x05AD)
            // byte 3..10: 8-byte public key (8 zeros -> crypto disabled for game session)
            // byte 11: u8 subcode / result (0 = success)
            // byte 12: u8 string length (0 = empty message)
            let mut response = Vec::with_capacity(13);
            response.push(0x2B);
            response.extend_from_slice(&1453u16.to_le_bytes());
            response.extend_from_slice(&[0u8; 8]);
            response.push(0x00);
            response.push(0x00);

            let packet = frame_packet(&response);
            tracing::info!(%peer, dump = %format_hex(&packet), "sending plaintext WIZ_VERSION_CHECK response");
            if let Err(e) = sock.write_all(&packet).await {
                tracing::error!(%peer, error = %e, "failed to send WIZ_VERSION_CHECK response");
            }
        }
        // Opcode 0x01: WIZ_LOGIN (Game Server Account Auth)
        0x01 => {
            tracing::info!(%peer, "handling game server WIZ_LOGIN (0x01)");
            // Nation zero opens selection; 0xff refuses an unauthenticated handoff.
            if account.is_none() {
                *account = lobby.claim_login(peer.ip());
            }
            let nation = account
                .as_deref()
                .and_then(|account| characters::nation(&lobby.db.lock().unwrap(), account).ok())
                .unwrap_or(0xff);
            tracing::info!(%peer, ?account, nation, "game lobby account association");
            let response = [0x01, nation];
            let packet = frame_packet(&response);
            tracing::info!(%peer, dump = %format_hex(&packet), "sending WIZ_LOGIN (0x01) response");
            if let Err(e) = sock.write_all(&packet).await {
                tracing::error!(%peer, error = %e, "failed to send WIZ_LOGIN response");
            }
        }
        // Creation success is zero; the client then requests the character list.
        0x02 => {
            let result = match (account.as_deref(), characters::Character::parse(payload)) {
                (Some(account), Ok(character)) => {
                    if let Err(status) = lobby.rules.validate(&character) {
                        send_response(sock, peer, &None, &[0x02, status], "WIZ_NEW_CHAR").await;
                        return;
                    }
                    let result =
                        characters::create(&mut lobby.db.lock().unwrap(), account, &character);
                    match result {
                        Ok(status) => {
                            if status == 0
                                && let Some(catalog) = &lobby.starters
                            {
                                let grant_result =
                                    starter::grant_missing(&mut lobby.db.lock().unwrap(), catalog);
                                if let Err(error) = grant_result {
                                    tracing::error!(%peer, %error, "starter grant failed");
                                    send_response(sock, peer, &None, &[0x02, 4], "WIZ_NEW_CHAR")
                                        .await;
                                    return;
                                }
                            }
                            tracing::info!(%peer, account, name = %character.name, slot = character.slot, race = character.race, class = character.class, ?character.stats, status, "character creation result");
                            status
                        }
                        Err(error) => {
                            tracing::error!(%peer, %error, "character creation database error");
                            4
                        }
                    }
                }
                (_, Err(status)) => status,
                (None, _) => {
                    tracing::warn!(%peer, "creation refused: no unambiguous recent login; reconnect through login server");
                    11
                }
            };
            send_response(sock, peer, &None, &[0x02, result], "WIZ_NEW_CHAR").await;
        }
        0x05 => {
            let result = if payload.len() == 2 {
                account.as_deref().map(|account| {
                    characters::set_nation(&lobby.db.lock().unwrap(), account, payload[1])
                })
            } else {
                None
            };
            let selected = match result {
                Some(Ok(true)) => payload[1],
                Some(Err(error)) => {
                    tracing::error!(%peer, %error, "nation selection database error");
                    0
                }
                _ => 0,
            };
            send_response(sock, peer, &None, &[0x05, selected], "WIZ_NATION_SELECT").await;
        }
        0x04 => {
            let character = match (account.as_deref(), world_entry::selected_name(payload)) {
                (Some(account), Some(name)) => {
                    match characters::find(&lobby.db.lock().unwrap(), account, name) {
                        Ok(character) => character,
                        Err(error) => {
                            tracing::error!(%peer, %error, "character selection database error");
                            None
                        }
                    }
                }
                _ => None,
            };
            let character = character.filter(|_| {
                !lobby.social.lock().unwrap().players.values().any(|p| {
                    account
                        .as_deref()
                        .is_some_and(|a| p.account.eq_ignore_ascii_case(a))
                        && world
                            .as_ref()
                            .is_none_or(|current| current.player_id != p.s.player_id)
                })
            });
            *world = character.and_then(world_entry::Session::new);
            if let (Some(session), Some(registry)) = (world.as_ref(), lobby.monsters.as_ref()) {
                registry.take_experience(&session.character.name);
            }
            if let (Some(session), Some(book)) = (
                world.as_ref(),
                lobby.monsters.as_ref().and_then(|w| w.loot.as_ref()),
            ) {
                book.register_owner(&session.character.name, session.character.nation());
            }
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                session.clan =
                    clans::load(&lobby.db.lock().unwrap(), account, session.character.slot)
                        .unwrap_or_default();
                session.is_admin =
                    admin::is_admin(&lobby.db.lock().unwrap(), account, session.character.slot)
                        .unwrap_or(false);
                let position =
                    movement::load(&lobby.db.lock().unwrap(), account, session.character.slot);
                match position {
                    Ok(position) => session.position = position,
                    Err(error) => {
                        tracing::error!(%peer, %error, "failed to load saved position");
                        *world = None;
                    }
                }
            }
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                let loaded = {
                    let db = lobby.db.lock().unwrap();
                    skills::load(&db, account, session.character.slot).and_then(|state| {
                        progress::load(&db, account, session.character.slot)
                            .map(|progress| (state, progress))
                    })
                };
                match loaded {
                    Ok((state, progress)) => {
                        session.stat_points = stat_points::load(
                            &lobby.db.lock().unwrap(),
                            account,
                            session.character.slot,
                        )
                        .unwrap_or(0);
                        session.skills = state;
                        session.progress = progress;
                        let db = lobby.db.lock().unwrap();
                        session.rewards = quest_rewards::load(&db, account, session.character.slot)
                            .unwrap_or_default();
                        session.max_exp = quest_rewards::maximum(session.progress.level)
                            .unwrap_or(lobby.bootstrap.next_level_exp);
                    }
                    Err(error) => {
                        tracing::error!(%peer, %error, "failed to load mana/buff state");
                        *world = None;
                    }
                }
            }
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                let items = {
                    let db = lobby.db.lock().unwrap();
                    starter::equipment(&db, account, session.character.slot).and_then(|equipment| {
                        starter::coins(&db, account, session.character.slot)
                            .map(|coins| (equipment, coins))
                    })
                };
                match items {
                    Ok((equipment, coins)) => {
                        session.equipment = equipment;
                        match starter::inventory(
                            &lobby.db.lock().unwrap(),
                            account,
                            session.character.slot,
                        ) {
                            Ok(items) => session.inventory = items,
                            Err(error) => {
                                tracing::error!(%peer,%error,"inventory load failed");
                                return;
                            }
                        }
                        session.coins = coins;
                    }
                    Err(error) => {
                        tracing::error!(%peer, %error, "failed to load character equipment/wallet");
                        *world = None;
                    }
                }
            }
            if let (Some(session), Some(account), Some(catalog)) =
                (world.as_mut(), account.as_deref(), lobby.combat.as_ref())
            {
                match catalog.calculate(&session.character, &session.progress, &session.equipment) {
                    Ok(mut stats) => {
                        stats.weight = stats
                            .weight
                            .saturating_add(catalog.bag_weight(&session.inventory));
                        tracing::info!(%peer,attack=stats.attack,defence=stats.defence,hp=stats.hp,mp=stats.mp,hit_rate=stats.hit_rate,evasion=stats.evasion,"combat attributes calculated");
                        session.combat = stats;
                        session.base_combat = stats;
                        session.skills.set_limits(stats.hp, stats.mp);
                        utility::refresh(session, lobby.skills.as_ref(), lobby.combat.as_ref());
                        session
                            .skills
                            .set_limits(session.combat.hp, session.combat.mp);
                        if let Err(error) = skills::save(
                            &lobby.db.lock().unwrap(),
                            account,
                            session.character.slot,
                            &session.skills,
                        ) {
                            tracing::error!(%peer,%error,"combat vitals save failed");
                            *world = None;
                        }
                    }
                    Err(error) => {
                        tracing::error!(%peer,%error,"combat attributes failed");
                        *world = None;
                    }
                }
            }
            let response = if let Some(session) = world {
                tracing::info!(%peer, name = %session.character.name, player_id = session.player_id, "owned character selected; waiting for client map load");
                session.select_response()
            } else {
                tracing::warn!(%peer, "character selection refused: malformed request or character not owned");
                vec![4, 0]
            };
            send_response(sock, peer, &None, &response, "WIZ_SEL_CHAR").await;
        }
        0x0D => {
            let Some(session) = world.as_mut() else {
                tracing::warn!(%peer, "world start refused: select an owned character first");
                return;
            };
            if !world_entry::start_matches(payload, session) {
                tracing::warn!(%peer, "world start refused: character name or stage mismatch");
                return;
            }
            if !session.snapshot_sent {
                let packet = session.my_info(&lobby.bootstrap);
                tracing::info!(%peer, name = %session.character.name, player_id = session.player_id, bytes = packet.len(), "sending initial character data after map load");
                send_response(sock, peer, &None, &packet, "WIZ_MYINFO").await;
                session.snapshot_sent = true;
            }
            send_response(sock, peer, &None, &[0x0D], "WIZ_GAMESTART").await;
            let first_entry = !session.entered;
            session.entered = true;
            if first_entry {
                session.next_regen = skills::now_ms() + 6000;
                if let Some(account) = account.as_deref() {
                    if let Some(packet) = session.zone_ability() {
                        send_response(sock, peer, &None, &packet, "ZONE_ABILITY").await;
                    }
                    let packet = quests::snapshot(
                        &lobby.db.lock().unwrap(),
                        account,
                        session.character.slot,
                    );
                    match packet {
                        Ok(packet) => {
                            send_response(sock, peer, &None, &packet, "QUEST_SNAPSHOT").await
                        }
                        Err(error) => tracing::error!(%peer,%error,"quest snapshot failed"),
                    }
                    let counts = quests::monster_snapshot(
                        &lobby.db.lock().unwrap(),
                        account,
                        session.character.slot,
                    );
                    if let Ok(counts) = counts {
                        send_response(sock, peer, &None, &counts, "QUEST_MONSTER_SNAPSHOT").await;
                    }
                }
            }
            let now = skills::now_ms();
            for buff in session
                .skills
                .buffs
                .values()
                .filter(|buff| now < buff.until)
            {
                let mut packet = skills::effect(buff.skill, session.player_id);
                let remaining = ((buff.until - now).div_ceil(1000) as u16).to_le_bytes();
                packet[16..18].copy_from_slice(&remaining);
                send_response(sock, peer, &None, &packet, "SPRINT_RESTORE").await;
            }
            if session
                .skills
                .buffs
                .get(&91)
                .is_some_and(|buff| now < buff.until)
            {
                magic::visibility(lobby, session, true);
            }
            if first_entry {
                population::refresh(sock, peer, lobby, session).await;
            }
            tracing::info!(%peer, name = %session.character.name, "world bootstrap acknowledged; awaiting client follow-up");
        }
        0x6B => {
            // A zero count skips the optional entries in the client parser.
            send_response(sock, peer, &None, &[0x6B, 0, 0], "WIZ_OPTIONAL_LIST").await;
        }
        0x06 => {
            let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) else {
                return;
            };
            if !session.entered {
                return;
            }
            let Some(position) = session.movement.validate(
                payload,
                session.position,
                std::time::Instant::now(),
                session
                    .skills
                    .speed(lobby.skills.as_ref(), skills::now_ms()),
            ) else {
                tracing::warn!(%peer, "movement refused: malformed packet, speed, or distance limit");
                return;
            };
            let saved = movement::save(
                &lobby.db.lock().unwrap(),
                account,
                session.character.slot,
                position,
            );
            match saved {
                Ok(()) => {
                    let previous = session.position;
                    session.position = position;
                    if travel::movement_event(sock, peer, lobby, account, session, previous).await {
                        return;
                    }
                    population::refresh(sock, peer, lobby, session).await;
                    if i16::from_le_bytes([payload[7], payload[8]]) != 0 {
                        session.sitting = false;
                        // Forecast movement can arrive after casting starts; phase4 cancels casts.
                        match magic::remove_stealth(lobby, account, session, true) {
                            Ok(true) => {
                                send_response(
                                    sock,
                                    peer,
                                    &None,
                                    &[0x31, 5, 91],
                                    "HIDE_BROKEN_ON_MOVE",
                                )
                                .await
                            }
                            Err(error) => tracing::error!(%peer,%error,"hide break failed"),
                            _ => {}
                        }
                    }
                    tracing::debug!(%peer, ?position, "movement position committed");
                }
                Err(error) => tracing::error!(%peer, %error, "failed to save movement position"),
            }
        }
        0x08 => {
            let Some(session) = world.as_mut().filter(|s| s.entered && s.skills.hp > 0) else {
                return;
            };
            if payload.len() != 9 || payload[1] != 1 {
                return;
            }
            let target = u16::from_le_bytes([payload[3], payload[4]]);
            let now = skills::now_ms();
            session.is_admin = account.as_deref().is_some_and(|account| {
                admin::is_admin(&lobby.db.lock().unwrap(), account, session.character.slot)
                    .unwrap_or(false)
            });
            let mut result = 0;
            if now >= session.next_attack
                && !session.skills.buffs.get(&20).is_some_and(|b| now < b.until)
                && (session.is_admin || !matches!(session.combat.weapon_kind / 10, 7 | 8 | 10))
                && let Some(monsters) = lobby.monsters.as_ref()
            {
                let attack = monsters::Attack {
                    position: session.position,
                    range: session.combat.range,
                    attack: if session.is_admin {
                        0
                    } else {
                        session.combat.attack
                    },
                    percent: 100,
                    extra: if session.is_admin { 10_000 } else { 0 },
                    skill: false,
                    drain_percent: 0,
                    arrow: false,
                    penetrate: false,
                    magic: None,
                };
                match monsters.hit_session_owned(target, attack, session, true, || Ok(())) {
                    Ok(hit) => {
                        result = if hit.dead { 2 } else { 1 };
                        session.next_attack = now + session.combat.delay_ms;
                        if let Some(account) = account.as_deref() {
                            durability::attack(
                                sock,
                                peer,
                                lobby,
                                account,
                                session,
                                hit.damage.min(32767) as u16,
                            )
                            .await;
                            match magic::remove_stealth(lobby, account, session, false) {
                                Ok(true) => {
                                    send_response(
                                        sock,
                                        peer,
                                        &None,
                                        &[0x31, 5, 91],
                                        "STEALTH_BROKEN_ON_ATTACK",
                                    )
                                    .await
                                }
                                Err(error) => tracing::error!(%peer,%error,"stealth break failed"),
                                _ => {}
                            }
                        }
                        tracing::info!(%peer,target,damage=hit.damage,"normal attack applied");
                    }
                    Err(reason) => tracing::info!(%peer,target,reason,"normal attack refused"),
                }
            }
            let mut response = vec![0x08, 1, result];
            response.extend(session.player_id.to_le_bytes());
            response.extend(target.to_le_bytes());
            send_response(sock, peer, &None, &response, "WIZ_ATTACK").await;
        }
        0x1d => {
            if let Some(session) = world.as_mut() {
                population::request(sock, peer, lobby, session, payload).await;
            }
        }
        0x45 => {
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                storage::handle(sock, peer, lobby, account, session, payload).await;
            }
        }
        0x33 | 0x5b => {
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                if payload.first() == Some(&0x33) && travel::is_gate(lobby, session, payload) {
                    travel::handle(sock, peer, lobby, account, session, payload).await;
                } else {
                    upgrading::handle(sock, peer, lobby, account, session, payload).await;
                }
            }
        }
        0x27 | 0x4b => {
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                travel::handle(sock, peer, lobby, account, session, payload).await;
            }
        }
        0x24 | 0x26 => {
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                loot::handle(sock, peer, lobby, account, session, payload).await;
            }
        }
        0x22 => {
            let Some(session) = world.as_ref().filter(|s| s.entered) else {
                return;
            };
            if payload.len() != 4 {
                return;
            }
            let target = u16::from_le_bytes([payload[1], payload[2]]);
            if let Some(packet) = lobby
                .monsters
                .as_ref()
                .and_then(|w| w.health(target, session.position.zone))
            {
                send_response(sock, peer, &None, &packet, "TARGET_HP").await;
            }
        }
        0x31 => {
            if magic::handle(
                sock,
                peer,
                payload,
                lobby,
                account.as_deref(),
                world.as_mut(),
            )
            .await
            {
                return;
            }
            let (Some(session), Some(account)) =
                (world.as_mut().filter(|s| s.entered), account.as_deref())
            else {
                return;
            };
            if payload.len() != 22 || payload[1] != 3 {
                tracing::warn!(%peer, "unsupported skill stage or malformed request");
                return;
            }
            let id = u32::from_le_bytes(payload[2..6].try_into().unwrap());
            let caster = u16::from_le_bytes(payload[6..8].try_into().unwrap());
            let target = u16::from_le_bytes(payload[8..10].try_into().unwrap());
            let definition = lobby
                .skills
                .as_ref()
                .and_then(|c| c.speeds.get(&id))
                .copied();
            let result = {
                let db = lobby.db.lock().unwrap();
                skills::load(&db, account, session.character.slot).and_then(|mut next| {
                    let cast = if caster != session.player_id
                        || target != session.player_id
                        || definition.is_some_and(|skill| {
                            !skill.learned_by(
                                session.character.class,
                                session.progress.level,
                                &session.progress.points,
                            )
                        }) {
                        Err("This skill's class or learning requirements are not met.")
                    } else if let Some(sprint) = definition {
                        next.cast(id, sprint, skills::now_ms())
                    } else {
                        Err("This skill is not implemented yet.")
                    };
                    match cast {
                        Err(reason) => Ok(Err(reason)),
                        Ok(()) => {
                            skills::save(&db, account, session.character.slot, &next)?;
                            Ok(Ok(next))
                        }
                    }
                })
            };
            let next = match result {
                Err(error) => {
                    tracing::error!(%peer, %error, "skill state commit failed");
                    return;
                }
                Ok(Ok(next)) => next,
                Ok(Err(reason)) => {
                    tracing::info!(%peer, id, reason, "skill refused");
                    let mut failure = skills::effect(id, session.player_id);
                    failure[1] = 4;
                    failure[10..].fill(0);
                    send_response(sock, peer, &None, &failure, "SKILL_FAILED").await;
                    let notice = chat::packet(
                        7,
                        session.character.nation(),
                        u16::MAX,
                        "",
                        reason.as_bytes(),
                    );
                    send_response(sock, peer, &None, &notice, "SKILL_NOTICE").await;
                    return;
                }
            };
            session.skills = next;
            send_response(
                sock,
                peer,
                &None,
                &session.skills.mana_packet(),
                "WIZ_MSP_CHANGE",
            )
            .await;
            send_response(
                sock,
                peer,
                &None,
                &skills::effect(id, session.player_id),
                "SPRINT_EFFECT",
            )
            .await;
            tracing::info!(%peer, id, mana=session.skills.mana, until=session.skills.buffs.get(&6).map(|buff|buff.until), "Sprint committed");
        }
        0x29 => {
            let Some(session) = world.as_mut().filter(|s| s.entered) else {
                return;
            };
            if payload.len() == 4 && payload[1] == 1 {
                match u16::from_le_bytes([payload[2], payload[3]]) {
                    1 => session.sitting = false,
                    2 => session.sitting = true,
                    _ => return,
                }
                tracing::info!(%peer, sitting=session.sitting, "recovery posture updated");
            }
        }
        0x20 | 0x21 | 0x3b => {
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                shops::handle(sock, peer, lobby, account, session, payload).await;
            }
        }
        0x28 => {
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                stat_points::handle(sock, peer, lobby, account, session, payload).await;
            }
        }
        0x32 => {
            let (Some(session), Some(account)) =
                (world.as_mut().filter(|s| s.entered), account.as_deref())
            else {
                return;
            };
            if payload.len() != 2 {
                return;
            }
            let branch = payload[1];
            let result = progress::allocate(
                &lobby.db.lock().unwrap(),
                account,
                session.character.slot,
                session.character.class,
                branch,
            );
            match result {
                Ok((progress, accepted)) => {
                    session.progress = progress;
                    if !accepted
                        && let Some(points) = session.progress.points.get(usize::from(branch))
                    {
                        send_response(
                            sock,
                            peer,
                            &None,
                            &[0x32, branch, *points],
                            "SKILL_POINTS_REFUSED",
                        )
                        .await;
                    }
                    tracing::info!(%peer,branch,accepted,free=session.progress.points[0],"skill points allocation");
                }
                Err(error) => tracing::error!(%peer,%error,"skill allocation save failed"),
            }
        }
        0x48 => {
            if matches!(payload, [0x48] | [0x48, 0])
                && let (Some(session), Some(account)) = (world.as_mut(), account.as_deref())
            {
                town::handle(sock, peer, lobby, account, session).await;
            }
        }
        0x10 => {
            let Some(session) = world.as_mut().filter(|session| session.entered) else {
                return;
            };
            let Some((channel, text)) = chat::parse(payload) else {
                tracing::warn!(%peer, "invalid or unsupported chat request");
                return;
            };
            if channel == 1
                && let Some(packet) = loot::command(session, text)
            {
                send_response(sock, peer, &None, &packet, "AUTOLOOT_COMMAND").await;
                return;
            }
            if channel == 1
                && let Some(account) = account.as_deref()
                && quest_journal::handle(sock, peer, lobby, account, session, text).await
            {
                return;
            }
            if channel == 1
                && let Some(account) = account.as_deref()
                && gm_teleport::handle(sock, peer, lobby, account, session, text).await
            {
                return;
            }
            if channel == 1
                && let Some(account) = account.as_deref()
                && gm_spawn::handle(sock, peer, lobby, account, session, text).await
            {
                return;
            }
            if channel == 1 && text.eq_ignore_ascii_case(b"/town") {
                if let Some(account) = account.as_deref() {
                    town::handle(sock, peer, lobby, account, session).await;
                }
                return;
            }
            if channel == 1
                && let Some(account) = account.as_deref()
                && utility::training_command(sock, peer, lobby, account, session, text).await
            {
                return;
            }
            if matches!(channel, 3 | 6)
                && lobby
                    .social
                    .lock()
                    .unwrap()
                    .chat(session.player_id, channel, text)
            {
                return;
            }
            if matches!(channel, 3 | 6 | 13) {
                let text = match channel {
                    3 => b"You are not in a party.".as_slice(),
                    6 => b"You are not in a clan.".as_slice(),
                    _ => b"You are not in an alliance.".as_slice(),
                };
                let packet = chat::packet(7, session.character.nation(), u16::MAX, "", text);
                send_response(sock, peer, &None, &packet, "CHAT_NOTICE").await;
            } else {
                if channel == 5 {
                    // TODO(re): Verify the original shout cost and eligibility rules.
                    let Some(account) = account.as_deref() else {
                        return;
                    };
                    let result = {
                        let db = lobby.db.lock().unwrap();
                        skills::load(&db, account, session.character.slot).and_then(|mut next| {
                            let cost = (next.max_mana / 5).max(1);
                            if next.mana < cost {
                                return Ok(None);
                            }
                            next.mana -= cost;
                            skills::save(&db, account, session.character.slot, &next)?;
                            Ok(Some(next))
                        })
                    };
                    match result {
                        Ok(Some(next)) => {
                            session.skills = next;
                            send_response(
                                sock,
                                peer,
                                &None,
                                &session.skills.mana_packet(),
                                "SHOUT_MANA",
                            )
                            .await;
                        }
                        Ok(None) => {
                            let notice = chat::packet(
                                7,
                                session.character.nation(),
                                u16::MAX,
                                "",
                                b"Not enough mana to shout.",
                            );
                            send_response(sock, peer, &None, &notice, "CHAT_NOTICE").await;
                            return;
                        }
                        Err(error) => {
                            tracing::error!(%peer, %error, "shout mana commit failed");
                            return;
                        }
                    }
                }
                let message = chat::Message {
                    channel,
                    nation: session.character.nation(),
                    position: session.position,
                    packet: chat::packet(
                        channel,
                        session.character.nation(),
                        session.player_id,
                        &session.character.name,
                        text,
                    ),
                };
                let _ = lobby.chat.send(message);
            }
        }
        // Opcode 0x0C: WIZ_ALLCHAR_INFO_REQ
        0x0C => {
            tracing::info!(%peer, dump = %format_hex(payload), "handling game server WIZ_ALLCHAR_INFO_REQ (0x0C)");
            let result = characters::list_payload(&lobby.db.lock().unwrap(), account.as_deref());
            let response = match result {
                Ok(response) => response,
                Err(error) => {
                    tracing::error!(%peer, %error, "character list database error");
                    vec![0x0C, 0]
                }
            };
            let packet = frame_packet(&response);
            tracing::info!(%peer, dump = %format_hex(&packet), "sending WIZ_ALLCHAR_INFO_REQ (0x0C) response");
            if let Err(e) = sock.write_all(&packet).await {
                tracing::error!(%peer, error = %e, "failed to send WIZ_ALLCHAR_INFO_REQ response");
            }
        }
        // Opcode 0x41: WIZ_SPEEDHACK_CHECK / Heartbeat
        0x55 | 0x64 => {
            if let (Some(account), Some(s)) = (account.as_deref(), world.as_mut()) {
                npc_scripts::handle(sock, peer, lobby, account, s, payload).await;
            }
        }
        0x12 => {
            if let (Some(account), Some(s)) = (account.as_deref(), world.as_mut()) {
                town::respawn(sock, peer, lobby, account, s, payload).await;
            }
        }
        0x1F => {
            let response = inventory::handle(lobby, account.as_deref(), world.as_mut(), payload);
            send_response(sock, peer, &None, &response, "ITEM_MOVE").await;
        }
        0x3F => {
            for response in inventory::remove(lobby, account.as_deref(), world.as_mut(), payload) {
                send_response(sock, peer, &None, &response, "ITEM_REMOVE").await;
            }
        }
        0x79 => {
            if let (Some(account), Some(s)) =
                (account.as_deref(), world.as_ref().filter(|s| s.entered))
            {
                let result = hotkeys::process(
                    &lobby.db.lock().unwrap(),
                    account,
                    s.character.slot,
                    payload,
                );
                match result {
                    Ok(Some(packet)) => {
                        send_response(sock, peer, &None, &packet, "SKILLBAR_LOAD").await
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(%peer,%error,"skillbar request refused"),
                }
            }
        }
        0x70 => {
            if let (Some(session), Some(account)) = (world.as_mut(), account.as_deref()) {
                clan_promotion::cape(sock, peer, lobby, account, session, payload).await;
            }
        }
        0x41 => {
            tracing::debug!(%peer, dump = %format_hex(payload), "handling game server WIZ_SPEEDHACK_CHECK (0x41)");
            // TODO(re): Recover the response semantics of this periodic timestamp packet.
        }
        _ => {
            tracing::warn!(%peer, opcode = %format!("0x{opcode:02X}"), dump = %format_hex(payload), "unhandled game server opcode; no response implemented; client may wait indefinitely");
        }
    }
}

async fn tick_character(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &lobby::Lobby,
    account: &str,
    session: &mut world_entry::Session,
    now: u64,
) {
    match suppression::tick(lobby, account, session, now) {
        Ok(packets) => {
            for p in packets {
                send_response(sock, peer, &None, &p, "SUPPRESSION_COMPLETION").await;
            }
        }
        Err(error) => tracing::error!(%error,"suppression return could not be saved"),
    }
    if !session.entered {
        return;
    }
    loot::auto_tick(sock, peer, lobby, account, session, now).await;
    transformation::refresh(sock, peer, lobby, account, session).await;
    utility::npc_strikes(sock, peer, lobby, account, session, now).await;
    if let Some(world) = &lobby.monsters {
        for credit in world.take_experience(&session.character.name) {
            quest_rewards::monster(sock, peer, lobby, account, session, &credit).await;
        }
        for prototype in world.take_quest_kills(&session.character.name) {
            let result = quests::kill(
                &lobby.db.lock().unwrap(),
                account,
                session.character.slot,
                prototype,
            );
            match result {
                Ok(packets) => {
                    for packet in packets {
                        send_response(sock, peer, &None, &packet, "QUEST_MONSTER_PROGRESS").await;
                    }
                }
                Err(error) => tracing::error!(%error,"quest kill progress could not be saved"),
            }
        }
    }
    let recovery_due = now >= session.next_regen;
    let expiry_due = session.skills.buffs.values().any(|buff| now >= buff.until);
    let periodic_due = session
        .skills
        .periodic
        .values()
        .any(|effect| now >= effect.next || now >= effect.until);
    if !recovery_due && !expiry_due && !periodic_due {
        return;
    }
    let result = {
        let db = lobby.db.lock().unwrap();
        skills::load(&db, account, session.character.slot).and_then(|mut next| {
            let before = next.hp;
            next.tick_periodic(now);
            let (_, mp) = if recovery_due {
                next.regenerate(
                    session.progress.level,
                    session.character.class,
                    session.sitting,
                )
            } else {
                (0, 0)
            };
            let expired = next.expire(now);
            if expired.iter().any(|c| matches!(c, 1 | 7)) {
                let stats = utility::effective(
                    session,
                    &next,
                    lobby.skills.as_ref(),
                    lobby.combat.as_ref(),
                );
                next.set_limits(stats.hp, stats.mp);
            }
            let hp = (i32::from(next.hp) - i32::from(before)) as i16;
            if hp != 0 || mp != 0 || !expired.is_empty() || periodic_due {
                skills::save(&db, account, session.character.slot, &next)?;
            }
            Ok((next, hp, mp, expired))
        })
    };
    if recovery_due {
        session.next_regen = now + 6000;
    }
    match result {
        Ok((next, hp, mp, expired)) => {
            session.skills = next;
            let previous = (
                session.combat.attack,
                session.combat.defence,
                session.combat.resistance,
            );
            utility::refresh(session, lobby.skills.as_ref(), lobby.combat.as_ref());
            if expired.iter().any(|c| matches!(c, 1 | 7)) {
                send_response(
                    sock,
                    peer,
                    &None,
                    &session.skills.hp_packet(0),
                    "STAT_BUFF_HEALTH_LIMIT",
                )
                .await;
                send_response(
                    sock,
                    peer,
                    &None,
                    &session.skills.mana_packet(),
                    "STAT_BUFF_MANA_LIMIT",
                )
                .await;
            }
            if previous
                != (
                    session.combat.attack,
                    session.combat.defence,
                    session.combat.resistance,
                )
            {
                send_response(
                    sock,
                    peer,
                    &None,
                    &inventory::attributes(&session.combat, 2),
                    "BUFF_ATTRIBUTES",
                )
                .await;
            }
            if hp != 0 {
                send_response(sock, peer, &None, &session.skills.hp_packet(hp), "HP_REGEN").await;
            }
            if mp != 0 {
                send_response(sock, peer, &None, &session.skills.mana_packet(), "MP_REGEN").await;
            }
            for category in expired {
                if category == transformation::CATEGORY {
                    continue;
                }
                if category == 91 {
                    magic::visibility(lobby, session, false);
                }
                send_response(sock, peer, &None, &[0x31, 5, category], "BUFF_EXPIRED").await;
            }
        }
        Err(error) => tracing::error!(%peer,%error,"recovery/buff state commit failed"),
    }
}

/// Handles incoming game client connection on port 15001.
async fn handle_game_client(mut sock: TcpStream, peer: SocketAddr, lobby: Arc<lobby::Lobby>) {
    tracing::info!(%peer, "game client connected to game server (15001)");
    let mut buf = [0u8; 4096];
    let mut decoder = framing::Decoder::default();
    let mut packet_count = 0u64;
    let mut last_opcode = None;
    let mut idle_reported = false;
    let mut account = None;
    let mut world: Option<world_entry::Session> = None;
    let mut social_guard = social::Guard {
        lobby: lobby.clone(),
        id: None,
    };
    let mut chat_receiver = lobby.chat.subscribe();
    let mut npc_receiver = lobby.monsters.as_ref().map(|w| w.events.subscribe());
    let mut skill_tick = tokio::time::interval(Duration::from_millis(250));
    let mut last_received = tokio::time::Instant::now();
    loop {
        let read = tokio::select! {
            event=async {npc_receiver.as_mut().unwrap().recv().await}, if npc_receiver.is_some()=> {
                match event {
                    Ok(event)=> {if let Some(packet)=world.as_mut().and_then(|s|population::event_packet(&event,&lobby,s)) {send_response(&mut sock,peer,&None,&packet,"COMBAT_WORLD").await;} }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count))=>tracing::warn!(%peer,count,"combat subscriber lagged"),
                    Err(tokio::sync::broadcast::error::RecvError::Closed)=>return,
                }
                continue;
            }
            _ = skill_tick.tick() => {
                if let (Some(session), Some(account)) = (world.as_mut().filter(|s| s.entered), account.as_deref()) {
                    clan_promotion::transfer(&mut sock,peer,&lobby,session).await;
                    if !session.entered { continue; }
                    tick_character(&mut sock,peer,&lobby,account,session,skills::now_ms()).await;
                    social::synchronize(&lobby,account,session);
                    social::flush(&mut sock,peer,&lobby,session).await;
                }
                continue;
            }
            event = chat_receiver.recv() => {
                match event {
                    Ok(message) => {
                        if world.as_ref().is_some_and(|session| message.visible_to(session)) {
                            send_response(&mut sock, peer, &None, &message.packet, "WIZ_CHAT").await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => tracing::warn!(%peer, count, "chat subscriber lagged"),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
                continue;
            }
            _ = tokio::time::sleep_until(last_received + Duration::from_secs(15)), if !idle_reported => {
                    tracing::warn!(%peer, packet_count, ?last_opcode, pending_bytes = decoder.pending(), "game connection idle for 15 seconds; watcher active, connection kept open");
                    idle_reported = true;
                    continue;
            },
            read = sock.read(&mut buf) => read,
        };
        match read {
            Ok(0) => {
                tracing::info!(%peer, packet_count, pending_bytes = decoder.pending(), "game client disconnected (EOF)");
                break;
            }
            Ok(n) => {
                let data = &buf[..n];
                idle_reported = false;
                last_received = tokio::time::Instant::now();
                tracing::info!(%peer, bytes = n, "received game server bytes");

                decoder.push(data);
                loop {
                    match decoder.next() {
                        Ok(Some(payload)) => {
                            if payload.first() != Some(&1) {
                                tracing::info!(%peer,dump=%format_hex(&payload),"received game request");
                            }
                            packet_count += 1;
                            last_opcode = payload.first().map(|opcode| format!("0x{opcode:02X}"));
                            let span = tracing::info_span!("request", packet = packet_count, opcode = %last_opcode.as_deref().unwrap_or("empty"));
                            handle_game_payload(
                                &mut sock,
                                peer,
                                &payload,
                                &lobby,
                                &mut account,
                                &mut world,
                            )
                            .instrument(span)
                            .await;
                            let current = world.as_ref().filter(|s| s.entered).map(|s| s.player_id);
                            if social_guard.id != current {
                                if let Some(old) = social_guard.id {
                                    lobby.social.lock().unwrap().disconnect(old);
                                }
                                social_guard.id = current;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::error!(%peer, error, pending_bytes = decoder.pending(), "invalid game frame; closing connection");
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(%peer, error = %e, "game client read error");
                break;
            }
        }
    }
    if let (Some(world), Some(registry)) = (world.as_ref(), lobby.monsters.as_ref()) {
        registry.take_experience(&world.character.name);
        registry.forget(world.player_id);
    }
}

/// Accept loops for Login Server (15100) and Game Server (15001).
pub async fn run(cfg: Config) -> anyhow::Result<()> {
    let rules = characters::CreationRules::load(&cfg.game_dir)?;
    let bootstrap = world_entry::Bootstrap::load(&cfg.game_dir)?;
    let catalog = starter::Catalog::load(&cfg.game_dir)?;
    let mut lobby = lobby::Lobby::new(open_db(&cfg)?, rules, bootstrap)?;
    quest_rewards::verify_thresholds(&lobby.db.lock().unwrap())?;
    starter::grant_missing(&mut lobby.db.lock().unwrap(), &catalog)?;
    lobby.starters = Some(catalog);
    lobby.skills = Some(skills::Catalog::load(&cfg.game_dir)?);
    lobby.merchants = shops::load(&lobby.db.lock().unwrap(), &cfg.game_dir)?;
    lobby.merchants.extend(shops::load_population(
        &lobby.db.lock().unwrap(),
        &cfg.game_dir,
    )?);
    lobby
        .merchants
        .extend(shops::load_objects(&lobby.db.lock().unwrap())?);
    let coefficients = PathBuf::from("logs/reference-coefficients.csv");
    if coefficients.exists() {
        lobby.combat = Some(combat::Catalog::load(&cfg.game_dir, &coefficients)?);
        lobby
            .combat
            .as_mut()
            .unwrap()
            .load_level_bounds(&lobby.db.lock().unwrap())?;
    }
    let mut target_world = monsters::World::load(&cfg.game_dir)?;
    target_world.social = Some(lobby.social.clone());
    if let Some(catalog) = lobby.combat.as_ref() {
        let mut book = loot::Book::load(&lobby.db.lock().unwrap(), catalog)?;
        book.social = Some(lobby.social.clone());
        target_world.loot = Some(Arc::new(book));
    }
    let targets = Arc::new(target_world);
    targets.load_population(&lobby.db.lock().unwrap(), &cfg.game_dir)?;
    targets.load_ai(&lobby.db.lock().unwrap())?;
    tokio::spawn(targets.clone().run());
    lobby.monsters = Some(targets);
    let lobby = Arc::new(lobby);
    let login_listener = TcpListener::bind(cfg.bind).await?;
    tracing::info!(addr = %cfg.bind, "listening for login client connections");

    let game_bind = SocketAddr::from(([0, 0, 0, 0], DEFAULT_GAME_PORT));
    let game_listener = TcpListener::bind(game_bind).await?;
    tracing::info!(addr = %game_bind, "listening for game server connections");

    let login_loop = async {
        loop {
            let (sock, peer) = login_listener.accept().await?;
            let connection = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(handle_client(sock, peer, lobby.clone()).instrument(
                tracing::info_span!("connection", connection, service = "login", %peer),
            ));
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };

    let game_loop = async {
        loop {
            let (sock, peer) = game_listener.accept().await?;
            let connection = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(handle_game_client(sock, peer, lobby.clone()).instrument(
                tracing::info_span!("connection", connection, service = "game", %peer),
            ));
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };

    tokio::try_join!(login_loop, game_loop)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admin_basic_hits_are_fixed_bounded_and_revocable_without_affecting_normal_attacks() {
        let Some((mut lobby, mut s)) = world_service_tests::setup() else {
            return;
        };
        let monsters = Arc::new(monsters::World::load(&reko_common::game_dir()).unwrap());
        lobby.monsters = Some(monsters.clone());
        lobby
            .db
            .lock()
            .unwrap()
            .execute("INSERT INTO character_privileges VALUES ('test',0,1)", [])
            .unwrap();
        s.position.x = 3250;
        s.position.z = 4060;
        s.combat.range = 2.0;
        s.combat.weapon_kind = 100;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut account = Some("test".to_string());
        let mut world = Some(s);
        let hp = |id| {
            let p = monsters.health(id, 21).unwrap();
            u32::from_le_bytes(p[8..12].try_into().unwrap())
        };
        let request = [8, 1, 0, 0x12, 0x27, 0, 0, 0, 0];
        handle_game_payload(
            &mut socket,
            peer,
            &request,
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert_eq!(hp(10002), 15000);
        handle_game_payload(
            &mut socket,
            peer,
            &request,
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert_eq!(hp(10002), 15000, "normal attack delay remains enforced");
        world.as_mut().unwrap().next_attack = 0;
        world.as_mut().unwrap().position.x = 5000;
        handle_game_payload(
            &mut socket,
            peer,
            &request,
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert_eq!(hp(10002), 15000, "remote admin targets remain out of range");
        world.as_mut().unwrap().position.x = 3250;
        handle_game_payload(
            &mut socket,
            peer,
            &request,
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert_eq!(hp(10002), 5000);
        world.as_mut().unwrap().next_attack = 0;
        handle_game_payload(
            &mut socket,
            peer,
            &request,
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert_eq!(hp(10002), 0, "last hit clamps to remaining HP");
        lobby
            .db
            .lock()
            .unwrap()
            .execute("UPDATE character_privileges SET is_admin=0", [])
            .unwrap();
        let s = world.as_mut().unwrap();
        s.next_attack = 0;
        s.position.z = 4000;
        s.combat.weapon_kind = 0;
        s.combat.attack = 1000;
        s.position.x = 3130;
        handle_game_payload(
            &mut socket,
            peer,
            &[8, 1, 0, 0x10, 0x27, 0, 0, 0, 0],
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert_eq!(
            hp(10000),
            25000,
            "ordinary model-based reach remains bounded"
        );
        world.as_mut().unwrap().position.x = 3250;
        handle_game_payload(
            &mut socket,
            peer,
            &[8, 1, 0, 0x10, 0x27, 0, 0, 0, 0],
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        assert!(!world.as_ref().unwrap().is_admin);
        assert!(
            (15001..25000).contains(&hp(10000)),
            "revocation restores ordinary damage immediately"
        );
    }

    async fn exchange(client: &mut TcpStream, payload: &[u8]) -> Vec<u8> {
        client.write_all(&frame_packet(payload)).await.unwrap();
        receive(client).await
    }

    async fn receive(client: &mut TcpStream) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut header = [0; 4];
            client.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[..2], &[0xAA, 0x55]);
            let len = u16::from_le_bytes([header[2], header[3]]) as usize;
            let mut body = vec![0; len + 2];
            client.read_exact(&mut body).await.unwrap();
            assert_eq!(&body[len..], &[0x55, 0xAA]);
            body.truncate(len);
            body
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn timer_recovery_obeys_posture_and_expiry_without_extra_requests() {
        let lobby = lobby::Lobby::new(
            rusqlite::Connection::open_in_memory().unwrap(),
            characters::CreationRules::fixture(),
            world_entry::Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        let character = characters::Character {
            slot: 0,
            name: "Recovery".into(),
            race: 2,
            class: 102,
            face: 0,
            hair: 0,
            stats: [60, 60, 80, 50, 50],
        };
        {
            let mut db = lobby.db.lock().unwrap();
            characters::ensure_account(&db, "recovery").unwrap();
            characters::set_nation(&db, "recovery", 1).unwrap();
            characters::create(&mut db, "recovery", &character).unwrap();
            let state = skills::State {
                periodic: std::collections::HashMap::new(),
                hp: 90,
                mana: 40,
                buffs: [(
                    6,
                    skills::Buff {
                        skill: 102001,
                        until: 6000,
                    },
                )]
                .into_iter()
                .collect(),
                ..skills::State::default()
            };
            skills::save(&db, "recovery", 0, &state).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut session = world_entry::Session::new(character).unwrap();
        session.entered = true;
        tick_character(&mut socket, peer, &lobby, "recovery", &mut session, 0).await;
        assert_eq!(receive(&mut client).await, [0x18, 100, 0, 43, 0]);
        let mut world = Some(session);
        let mut account = Some("recovery".into());
        handle_game_payload(
            &mut socket,
            peer,
            &[0x29, 1, 2, 0],
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        let session = world.as_mut().unwrap();
        assert!(session.sitting);
        tick_character(&mut socket, peer, &lobby, "recovery", session, 5999).await;
        assert_eq!(session.skills.mana, 43);
        tick_character(&mut socket, peer, &lobby, "recovery", session, 6000).await;
        assert_eq!(receive(&mut client).await, [0x17, 100, 0, 94, 0, 4, 0]);
        assert_eq!(receive(&mut client).await, [0x18, 100, 0, 62, 0]);
        assert_eq!(receive(&mut client).await, [0x31, 5, 6]);
        assert_eq!(
            skills::load(&lobby.db.lock().unwrap(), "recovery", 0)
                .unwrap()
                .mana,
            62
        );
        handle_game_payload(
            &mut socket,
            peer,
            &[0x29, 1, 1, 0],
            &lobby,
            &mut account,
            &mut world,
        )
        .await;
        let session = world.as_mut().unwrap();
        assert!(!session.sitting);
        tick_character(&mut socket, peer, &lobby, "recovery", session, 12000).await;
        assert_eq!(receive(&mut client).await, [0x18, 100, 0, 65, 0]);
        assert_eq!(session.skills.hp, 94);
    }

    #[tokio::test]
    async fn selected_character_data_precedes_world_acknowledgement() {
        let mut lobby = lobby::Lobby::new(
            rusqlite::Connection::open_in_memory().unwrap(),
            characters::CreationRules::fixture(),
            world_entry::Bootstrap { next_level_exp: 50 },
        )
        .unwrap();
        lobby.skills = Some(skills::Catalog::fixture(std::collections::HashMap::from([
            (
                102001,
                skills::SpeedSkill {
                    mana: 5,
                    duration_ms: 500,
                    cooldown_ms: 6000,
                    speed_percent: 150,
                    skill_group: 1020,
                    required_points: 1,
                },
            ),
        ])));
        let lobby = Arc::new(lobby);
        let character = characters::Character {
            slot: 0,
            name: "AdminSiken".into(),
            race: 2,
            class: 102,
            face: 0,
            hair: 0,
            stats: [60, 60, 80, 50, 50],
        };
        lobby
            .record_login("127.0.0.1".parse().unwrap(), "admin")
            .unwrap();
        characters::set_nation(&lobby.db.lock().unwrap(), "admin", 1).unwrap();
        assert_eq!(
            characters::create(&mut lobby.db.lock().unwrap(), "admin", &character).unwrap(),
            0
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, peer) = listener.accept().await.unwrap();
        let server = tokio::spawn(handle_game_client(socket, peer, lobby.clone()));
        assert_eq!(exchange(&mut client, &[1]).await, [1, 1]);
        let mut select = vec![4, 0, 0, 10, 0];
        select.extend_from_slice(b"OtherNameX");
        select.extend_from_slice(&[1, 21]);
        assert_eq!(exchange(&mut client, &select).await, [4, 0]);
        select[5..15].copy_from_slice(b"AdminSiken");
        assert_eq!(
            exchange(&mut client, &select).await,
            [4, 1, 21, 0x80, 0x0C, 0xA0, 0x0F, 0, 0, 0]
        );
        let mut start = vec![0x0D, 1, 10];
        start.extend_from_slice(b"AdminSiken");
        let snapshot = exchange(&mut client, &start).await;
        assert_eq!(snapshot[0], 0x0E);
        assert_eq!(&snapshot[4..14], b"AdminSiken");
        assert_eq!(&snapshot[20..24], &[1, 2, 102, 0]);
        assert!(snapshot.len() > 900);
        assert_eq!(receive(&mut client).await, [0x0D]);
        assert_eq!(receive(&mut client).await, [0x5e, 1, 1, 0, 1, 10, 0]);
        assert_eq!(receive(&mut client).await, [0x64, 1, 1, 0, 0xf4, 1, 2]);
        assert_eq!(receive(&mut client).await, [0x64, 9, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(exchange(&mut client, &start).await, [0x0D]);
        assert_eq!(exchange(&mut client, &[0x6B]).await, [0x6B, 0, 0]);
        let normal = exchange(&mut client, b"\x10\x01\x04\x00test").await;
        assert_eq!(&normal[..3], &[0x10, 1, 1]);
        assert_eq!(&normal[5..], b"\x0aAdminSiken\x04\x00test");
        assert_eq!(
            exchange(&mut client, b"\x10\x05\x04\x00test").await,
            [0x18, 100, 0, 80, 0]
        );
        let shout = receive(&mut client).await;
        assert_eq!(shout[1], 5);
        let party = exchange(&mut client, b"\x10\x03\x04\x00test").await;
        assert_eq!(party[1], 7);
        assert!(party.ends_with(b"You are not in a party."));
        for (channel, message) in [
            (6, b"You are not in a clan.".as_slice()),
            (13, b"You are not in an alliance.".as_slice()),
        ] {
            let notice = exchange(&mut client, &[0x10, channel, 1, 0, b'x']).await;
            assert_eq!(notice[1], 7);
            assert!(notice.ends_with(message));
        }
        let player_id = u16::from_le_bytes([snapshot[1], snapshot[2]]);
        let mut cast = skills::effect(102001, player_id);
        cast[10..].fill(0);
        assert_eq!(exchange(&mut client, &cast).await, [0x18, 100, 0, 75, 0]);
        assert_eq!(
            receive(&mut client).await,
            skills::effect(102001, player_id)
        );
        let failure = exchange(&mut client, &cast).await;
        assert_eq!(&failure[..2], &[0x31, 4]);
        assert!(
            receive(&mut client)
                .await
                .ends_with(b"This skill is still cooling down.")
        );
        assert_eq!(receive(&mut client).await, [0x31, 5, 6]);
        let skill_state = skills::load(&lobby.db.lock().unwrap(), "admin", 0).unwrap();
        assert_eq!(skill_state.mana, 75);
        assert!(skill_state.buffs.is_empty());
        let movement = [6, 0x82, 0x0C, 0x9E, 0x0F, 87, 0, 45, 0, 3];
        client.write_all(&frame_packet(&movement)).await.unwrap();
        assert_eq!(exchange(&mut client, &[0x6B]).await, [0x6B, 0, 0]);
        let saved = movement::load(&lobby.db.lock().unwrap(), "admin", 0).unwrap();
        assert_eq!(saved.x, 3202);
        assert_eq!(saved.z, 3998);
        assert_eq!(saved.y, 87);
        drop(client);
        server.await.unwrap();
        lobby.record_login(peer.ip(), "admin").unwrap();
        let mut reconnect = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, peer) = listener.accept().await.unwrap();
        let server = tokio::spawn(handle_game_client(socket, peer, lobby.clone()));
        assert_eq!(exchange(&mut reconnect, &[1]).await, [1, 1]);
        assert_eq!(
            exchange(&mut reconnect, &select).await,
            [4, 1, 21, 0x82, 0x0C, 0x9E, 0x0F, 87, 0, 0]
        );
        let restored = exchange(&mut reconnect, &start).await;
        assert_eq!(&restored[67..69], &75u16.to_le_bytes());
        assert_eq!(receive(&mut reconnect).await, [0x0D]);
        drop(reconnect);
        server.await.unwrap();
        assert!(
            characters::find(&lobby.db.lock().unwrap(), "other", "AdminSiken")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn encrypted_login_creation_reload_and_failure_replies() {
        let lobby = Arc::new(
            lobby::Lobby::new(
                rusqlite::Connection::open_in_memory().unwrap(),
                characters::CreationRules::fixture(),
                world_entry::Bootstrap { next_level_exp: 50 },
            )
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut login = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, peer) = listener.accept().await.unwrap();
        let login_task = tokio::spawn(handle_client(socket, peer, lobby.clone()));
        let key = exchange(&mut login, &[0xF2]).await;
        assert_eq!(
            key,
            [vec![0xF2], KO_PUBLIC_KEY.to_le_bytes().to_vec()].concat()
        );
        let crypt = JvCrypt::new(KO_PUBLIC_KEY);
        let mut credentials = vec![1, 0, 0, 0, 0xF3, 5, 0];
        credentials.extend_from_slice(b"admin");
        credentials.extend_from_slice(&[4, 0]);
        credentials.extend_from_slice(b"pass");
        credentials.extend_from_slice(&[0; 4]);
        crypt.transform(&mut credentials);
        assert_eq!(
            exchange(&mut login, &credentials).await,
            crypt.encrypt_server_packet(&[0xF3, 1, 0xAD, 5, 1, 0])
        );
        let mut wrong = vec![1, 0, 0, 0, 0xf3, 5, 0];
        wrong.extend_from_slice(b"admin");
        wrong.extend_from_slice(&[5, 0]);
        wrong.extend_from_slice(b"wrong");
        wrong.extend_from_slice(&[0; 4]);
        crypt.transform(&mut wrong);
        assert_eq!(
            exchange(&mut login, &wrong).await,
            crypt.encrypt_server_packet(&[0xf3, 3])
        );
        assert!(
            lobby
                .claim_login(listener.local_addr().unwrap().ip())
                .is_none()
        );
        assert_eq!(
            exchange(&mut login, &credentials).await,
            crypt.encrypt_server_packet(&[0xf3, 1, 0xad, 5, 1, 0])
        );
        drop(login);
        login_task.await.unwrap();
        let mut game = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, peer) = listener.accept().await.unwrap();
        let game_task = tokio::spawn(handle_game_client(socket, peer, lobby.clone()));
        assert_eq!(exchange(&mut game, &[0x01]).await, vec![1, 0]);
        assert_eq!(exchange(&mut game, &[0x05, 1]).await, vec![5, 1]);
        assert_eq!(exchange(&mut game, &[0x05, 2]).await, vec![5, 0]);
        assert_eq!(exchange(&mut game, &[0x0C]).await.len(), 173);
        let mut creation = vec![2, 0, 10, 0];
        creation.extend_from_slice(b"AdminSiken");
        creation.extend_from_slice(&[2, 102, 0, 0, 0, 60, 60, 80, 50, 50]);
        assert_eq!(exchange(&mut game, &creation).await, vec![2, 0]);
        let list = exchange(&mut game, &[0x0C]).await;
        assert_eq!(list.len(), 183);
        assert_eq!(&list[4..14], b"AdminSiken");
        assert_eq!(exchange(&mut game, &creation).await, vec![2, 1]);
        creation[1] = 1;
        assert_eq!(exchange(&mut game, &creation).await, vec![2, 3]);
        assert_eq!(exchange(&mut game, &[2]).await, vec![2, 11]);
        drop(game);
        game_task.await.unwrap();
        lobby.record_login(peer.ip(), "admin").unwrap();
        let mut reconnect = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, peer) = listener.accept().await.unwrap();
        let reconnect_task = tokio::spawn(handle_game_client(socket, peer, lobby));
        assert_eq!(exchange(&mut reconnect, &[1]).await, vec![1, 1]);
        assert_eq!(exchange(&mut reconnect, &[0x0C]).await, list);
        drop(reconnect);
        reconnect_task.await.unwrap();
    }

    #[tokio::test]
    async fn game_socket_handles_split_and_combined_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, peer) = listener.accept().await.unwrap();
        let lobby = Arc::new(
            lobby::Lobby::new(
                rusqlite::Connection::open_in_memory().unwrap(),
                characters::CreationRules::fixture(),
                world_entry::Bootstrap { next_level_exp: 50 },
            )
            .unwrap(),
        );
        let task = tokio::spawn(handle_game_client(server, peer, lobby));
        let version = frame_packet(&[0x2B, 0xAD, 0x05]);
        client.write_all(&version[..3]).await.unwrap();
        let mut probe = [0u8; 1];
        assert!(
            tokio::time::timeout(Duration::from_millis(50), client.read(&mut probe))
                .await
                .is_err()
        );
        let mut rest = version[3..].to_vec();
        rest.extend(frame_packet(&[0x01]));
        rest.extend(frame_packet(&[0x0C]));
        rest.extend(frame_packet(&[0x3f, 2, 13, 0x29, 0x11, 0x38, 0x0c]));
        client.write_all(&rest).await.unwrap();
        let mut expected = frame_packet(&[0x2B, 0xAD, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        expected.extend(frame_packet(&[0x01, 0xff]));
        let mut characters = vec![0x0C, 0x01];
        characters.extend([0u8; 171]);
        expected.extend(frame_packet(&characters));
        expected.extend(frame_packet(&[0x3f, 0]));
        let mut received = vec![0; expected.len()];
        tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, expected);
        drop(client);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
}
