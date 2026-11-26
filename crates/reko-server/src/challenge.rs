//! Native 1453 PvP and Clan-vs-Clan Arena challenges (WIZ_CHALLENGE, opcode 0x75).
//!
//! Protocol and rules follow supplied 1453 `GameServer/ArenaHandler.cpp`,
//! `Unit.cpp::isInSafetyArea`, and native client `0x006C89FB..0x006C9840`.

use crate::{
    clans,
    lobby::Lobby,
    movement::Position,
    skills,
    social::{Hub, Player},
    travel,
    world_entry::Session,
};

pub(crate) const ZONE_ARENA: u8 = 48;

const PVP_REQUEST: u8 = 1;
const PVP_CANCEL: u8 = 2;
const PVP_ACCEPT: u8 = 3;
const PVP_REJECT: u8 = 4;
const PVP_REQ_SENT: u8 = 5;
const CVC_REQUEST: u8 = 6;
const CVC_CANCEL: u8 = 7;
const CVC_ACCEPT: u8 = 8;
const CVC_REJECT: u8 = 9;
const CVC_REQ_SENT: u8 = 10;
const GENERIC_ERROR: u8 = 11;
const ZONE_ERROR: u8 = 12;
const CLAN_ERROR: u8 = 13;

const TIMEOUT_MS: u64 = 60_000;

const PVP_TARGET_POS: Position = Position {
    zone: ZONE_ARENA,
    x: 1350,
    z: 1150,
    y: 0,
};
const PVP_CHALLENGER_POS: Position = Position {
    zone: ZONE_ARENA,
    x: 1200,
    z: 1150,
    y: 0,
};
const CVC_TARGET_POS: Position = Position {
    zone: ZONE_ARENA,
    x: 1280,
    z: 1250,
    y: 0,
};
const CVC_CHALLENGER_POS: Position = Position {
    zone: ZONE_ARENA,
    x: 1350,
    z: 1200,
    y: 0,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Pending {
    pub challenger: u16,
    pub target: u16,
    pub clan: bool,
    pub until: u64,
}

/// Center lobby safety area inside Zone 48 (Arena) from 1453 `Unit.cpp::isInSafetyArea`.
pub(crate) fn in_arena_safety_area(pos: Position) -> bool {
    if pos.zone != ZONE_ARENA {
        return false;
    }
    let (x, z) = (pos.x, pos.z);
    (x > 1060 && x < 1480 && z > 500 && z < 1490)
        || (x > 860 && x < 1690 && z > 1000 && z < 1270)
        || (x > 1020 && x < 1500 && z > 820 && z < 1440)
        || (x > 990 && x < 1570 && z > 880 && z < 1390)
}

fn named_packet(subcode: u8, name: &str) -> Vec<u8> {
    let mut packet = Vec::with_capacity(3 + name.len());
    packet.push(0x75);
    packet.push(subcode);
    packet.push(name.len() as u8);
    packet.extend_from_slice(name.as_bytes());
    packet
}

fn parse_name(payload: &[u8]) -> Option<&str> {
    if payload.len() < 3 {
        return None;
    }
    let len = usize::from(payload[2]);
    if len == 0 || len > 20 || payload.len() != 3 + len {
        return None;
    }
    let text = std::str::from_utf8(&payload[3..]).ok()?;
    text.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        .then_some(text)
}

fn is_forbidden_zone(zone: u8) -> bool {
    matches!(zone, 30 | 31 | 61..=69 | 71..=75 | 82 | 83)
}

pub(crate) fn busy(hub: &Hub, id: u16) -> bool {
    hub.challenges
        .iter()
        .any(|c| c.challenger == id || c.target == id)
}

fn participant_ready(hub: &Hub, player: &Player) -> bool {
    let id = player.s.player_id;
    player.s.entered
        && !player.s.zone_loading
        && player.s.skills.hp > 0
        && player.s.position.zone != ZONE_ARENA
        && hub.party(id).is_none()
        && !crate::trade::busy(hub, id)
        && !crate::merchant::busy(hub, id)
}

pub(crate) fn disconnect(hub: &mut Hub, id: u16) {
    let mut removed = Vec::new();
    hub.challenges.retain(|c| {
        if c.challenger == id || c.target == id {
            removed.push(*c);
            false
        } else {
            true
        }
    });
    for c in removed {
        if c.challenger == id {
            let sub = if c.clan { CVC_CANCEL } else { PVP_CANCEL };
            hub.send(c.target, vec![0x75, sub]);
        } else {
            let sub = if c.clan { CVC_REJECT } else { PVP_REJECT };
            hub.send(c.challenger, vec![0x75, sub]);
        }
    }
}

pub(crate) fn maintain(hub: &mut Hub, id: u16) {
    let now = skills::now_ms();
    let challenges = std::mem::take(&mut hub.challenges);
    let mut kept = Vec::with_capacity(challenges.len());
    let mut expired = Vec::new();
    for c in challenges {
        if c.challenger != id && c.target != id {
            kept.push(c);
            continue;
        }
        let valid = now < c.until
            && hub
                .players
                .get(&c.challenger)
                .zip(hub.players.get(&c.target))
                .is_some_and(|(a, b)| {
                    participant_ready(hub, a)
                        && participant_ready(hub, b)
                        && a.s.position.zone == b.s.position.zone
                });
        if valid {
            kept.push(c);
        } else {
            expired.push(c);
        }
    }
    hub.challenges = kept;
    for c in expired {
        let cancel = if c.clan { CVC_CANCEL } else { PVP_CANCEL };
        let reject = if c.clan { CVC_REJECT } else { PVP_REJECT };
        hub.send(c.target, vec![0x75, cancel]);
        hub.send(c.challenger, vec![0x75, reject]);
    }
}

/// Processes a `WIZ_CHALLENGE` (0x75) request from `s`.
///
/// Returns `Some(destination)` when the accepting player should immediately begin
/// a zone transfer to `ZONE_ARENA` on their own socket.
pub(crate) fn process(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    payload: &[u8],
) -> Option<Position> {
    if !s.entered || s.zone_loading || s.skills.hp == 0 || payload.len() < 2 {
        return None;
    }
    let subcode = payload[1];
    match subcode {
        PVP_REQUEST => {
            request(lobby, s.player_id, payload, false);
            None
        }
        CVC_REQUEST => {
            request(lobby, s.player_id, payload, true);
            None
        }
        PVP_CANCEL | CVC_CANCEL => {
            if payload.len() == 2 {
                cancel(&mut lobby.social.lock().unwrap(), s.player_id, subcode);
            }
            None
        }
        PVP_REJECT | CVC_REJECT => {
            if payload.len() == 2 {
                reject(&mut lobby.social.lock().unwrap(), s.player_id, subcode);
            }
            None
        }
        PVP_ACCEPT => {
            if payload.len() == 2 {
                accept_pvp(lobby, account, s)
            } else {
                None
            }
        }
        CVC_ACCEPT => {
            if payload.len() == 2 {
                accept_cvc(lobby, account, s)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn request(lobby: &Lobby, challenger_id: u16, payload: &[u8], clan: bool) {
    let mut hub = lobby.social.lock().unwrap();
    maintain(&mut hub, challenger_id);
    let Some(challenger) = hub.players.get(&challenger_id).cloned() else {
        return;
    };
    if busy(&hub, challenger_id) || !participant_ready(&hub, &challenger) {
        hub.send(challenger_id, vec![0x75, GENERIC_ERROR]);
        return;
    }
    if clan && (challenger.s.clan.id == 0 || challenger.s.clan.fame != 1) {
        hub.send(challenger_id, vec![0x75, CLAN_ERROR]);
        return;
    }
    if is_forbidden_zone(challenger.s.position.zone) {
        hub.send(challenger_id, vec![0x75, ZONE_ERROR]);
        return;
    }
    let Some(target_name) = parse_name(payload) else {
        hub.send(challenger_id, vec![0x75, GENERIC_ERROR]);
        return;
    };
    let Some(target_id) = hub.named(target_name) else {
        hub.send(challenger_id, vec![0x75, GENERIC_ERROR]);
        return;
    };
    if target_id == challenger_id || !hub.can_see(challenger_id, target_id) {
        hub.send(challenger_id, vec![0x75, GENERIC_ERROR]);
        return;
    }
    maintain(&mut hub, target_id);
    let Some(target) = hub.players.get(&target_id).cloned() else {
        hub.send(challenger_id, vec![0x75, GENERIC_ERROR]);
        return;
    };
    if busy(&hub, target_id)
        || !participant_ready(&hub, &target)
        || target.s.position.zone != challenger.s.position.zone
    {
        hub.send(challenger_id, vec![0x75, GENERIC_ERROR]);
        return;
    }
    if clan
        && (target.s.clan.id == 0
            || target.s.clan.fame != 1
            || target.s.clan.id == challenger.s.clan.id)
    {
        hub.send(challenger_id, vec![0x75, CLAN_ERROR]);
        return;
    }
    hub.challenges.push(Pending {
        challenger: challenger_id,
        target: target_id,
        clan,
        until: skills::now_ms() + TIMEOUT_MS,
    });
    let (req_sub, sent_sub) = if clan {
        (CVC_REQUEST, CVC_REQ_SENT)
    } else {
        (PVP_REQUEST, PVP_REQ_SENT)
    };
    hub.send(
        target_id,
        named_packet(req_sub, &challenger.s.character.name),
    );
    hub.send(
        challenger_id,
        named_packet(sent_sub, &target.s.character.name),
    );
}

fn cancel(hub: &mut Hub, challenger_id: u16, subcode: u8) {
    let clan = subcode == CVC_CANCEL;
    let Some(pos) = hub
        .challenges
        .iter()
        .position(|c| c.challenger == challenger_id && c.clan == clan)
    else {
        return;
    };
    let pending = hub.challenges.remove(pos);
    if hub.players.contains_key(&pending.target) {
        hub.send(pending.target, vec![0x75, subcode]);
    } else {
        hub.send(challenger_id, vec![0x75, GENERIC_ERROR]);
    }
}

fn reject(hub: &mut Hub, target_id: u16, subcode: u8) {
    let clan = subcode == CVC_REJECT;
    let Some(pos) = hub
        .challenges
        .iter()
        .position(|c| c.target == target_id && c.clan == clan)
    else {
        return;
    };
    let pending = hub.challenges.remove(pos);
    if hub.players.contains_key(&pending.challenger) {
        hub.send(pending.challenger, vec![0x75, subcode]);
        hub.send(target_id, vec![0x75, subcode]);
    } else {
        hub.send(target_id, vec![0x75, GENERIC_ERROR]);
    }
}

fn arena_configured(db: &rusqlite::Connection) -> bool {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM world_town_starts WHERE zone=?1)",
        [ZONE_ARENA],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

fn accept_pvp(lobby: &Lobby, account: &str, s: &Session) -> Option<Position> {
    // Always lock `db` before `social` to match trade/merchant/clan lock ordering.
    let mut db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let pos = hub
        .challenges
        .iter()
        .position(|c| c.target == s.player_id && !c.clan)?;
    let pending = hub.challenges.remove(pos);
    let challenger = hub.players.get(&pending.challenger).cloned();
    let target = hub.players.get(&s.player_id).cloned();
    let Some((challenger, target)) = challenger.zip(target) else {
        hub.send(s.player_id, vec![0x75, GENERIC_ERROR]);
        return None;
    };
    if skills::now_ms() >= pending.until
        || !participant_ready(&hub, &challenger)
        || !participant_ready(&hub, &target)
        || challenger.s.position.zone != target.s.position.zone
    {
        hub.send(s.player_id, vec![0x75, GENERIC_ERROR]);
        hub.send(challenger.s.player_id, vec![0x75, GENERIC_ERROR]);
        return None;
    }
    let saved = (|| -> anyhow::Result<()> {
        anyhow::ensure!(arena_configured(&db), "Arena zone 48 is not configured.");
        let tx = db.transaction()?;
        travel::save_destination(
            &tx,
            account,
            s.character.slot,
            s.position.zone,
            PVP_TARGET_POS,
            lobby.skills.as_ref(),
        )?;
        travel::save_destination(
            &tx,
            &challenger.account,
            challenger.s.character.slot,
            challenger.s.position.zone,
            PVP_CHALLENGER_POS,
            lobby.skills.as_ref(),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(error) = saved {
        tracing::error!(%error, "PvP arena transfer save failed");
        hub.send(s.player_id, vec![0x75, GENERIC_ERROR]);
        hub.send(challenger.s.player_id, vec![0x75, GENERIC_ERROR]);
        return None;
    }
    hub.queue_arena_transfer(challenger.s.player_id, PVP_CHALLENGER_POS, s.player_id);
    Some(PVP_TARGET_POS)
}

fn accept_cvc(lobby: &Lobby, account: &str, s: &Session) -> Option<Position> {
    // Always lock `db` before `social` to match trade/merchant/clan lock ordering.
    let mut db = lobby.db.lock().unwrap();
    let mut hub = lobby.social.lock().unwrap();
    let pos = hub
        .challenges
        .iter()
        .position(|c| c.target == s.player_id && c.clan)?;
    let pending = hub.challenges.remove(pos);
    let challenger = hub.players.get(&pending.challenger).cloned();
    let target = hub.players.get(&s.player_id).cloned();
    let Some((challenger, target)) = challenger.zip(target) else {
        hub.send(s.player_id, vec![0x75, GENERIC_ERROR]);
        return None;
    };
    if skills::now_ms() >= pending.until
        || !participant_ready(&hub, &challenger)
        || !participant_ready(&hub, &target)
        || challenger.s.position.zone != target.s.position.zone
    {
        hub.send(s.player_id, vec![0x75, GENERIC_ERROR]);
        hub.send(challenger.s.player_id, vec![0x75, GENERIC_ERROR]);
        return None;
    }
    let zone = s.position.zone;
    let candidates: Vec<Player> = hub
        .players
        .values()
        .filter(|p| {
            p.s.position.zone == zone && participant_ready(&hub, p) && !busy(&hub, p.s.player_id)
        })
        .cloned()
        .collect();
    let result = (|| -> anyhow::Result<Vec<(u16, Position)>> {
        anyhow::ensure!(arena_configured(&db), "Arena zone 48 is not configured.");
        let tx = db.transaction()?;
        let target_clan = clans::load(&tx, account, s.character.slot)?;
        let challenger_clan = clans::load(&tx, &challenger.account, challenger.s.character.slot)?;
        anyhow::ensure!(
            target_clan.id != 0
                && target_clan.fame == 1
                && challenger_clan.id != 0
                && challenger_clan.fame == 1
                && target_clan.id != challenger_clan.id,
            "Both participants must be leaders of different clans."
        );
        let mut transfers = Vec::new();
        for player in &candidates {
            let info = clans::load(&tx, &player.account, player.s.character.slot)?;
            let destination = if info.id == target_clan.id {
                Some(CVC_TARGET_POS)
            } else if info.id == challenger_clan.id {
                Some(CVC_CHALLENGER_POS)
            } else {
                None
            };
            if let Some(dest) = destination {
                travel::save_destination(
                    &tx,
                    &player.account,
                    player.s.character.slot,
                    player.s.position.zone,
                    dest,
                    lobby.skills.as_ref(),
                )?;
                if player.s.player_id != s.player_id {
                    transfers.push((player.s.player_id, dest));
                }
            }
        }
        tx.commit()?;
        Ok(transfers)
    })();
    match result {
        Ok(transfers) => {
            for (id, dest) in transfers {
                hub.queue_transfer(id, dest);
            }
            Some(CVC_TARGET_POS)
        }
        Err(error) => {
            tracing::error!(%error, "CvC arena transfer save failed");
            hub.send(s.player_id, vec![0x75, GENERIC_ERROR]);
            hub.send(challenger.s.player_id, vec![0x75, GENERIC_ERROR]);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        characters::{Character, CreationRules},
        lobby::Lobby,
        social,
        world_entry::Bootstrap,
    };
    use rusqlite::Connection;
    use std::sync::Arc;

    fn test_lobby() -> Arc<Lobby> {
        let db = Connection::open_in_memory().unwrap();
        let lobby = Arc::new(
            Lobby::new(
                db,
                CreationRules::fixture(),
                Bootstrap { next_level_exp: 50 },
            )
            .unwrap(),
        );
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO world_town_starts VALUES (48, 128, 120, 128, 120, 0, 0, 'test')",
                [],
            )
            .unwrap();
        lobby
    }

    fn spawn_player(lobby: &Lobby, account: &str, name: &str, nation: u8, zone: u8) -> Session {
        let (race, class) = if nation == 1 { (1, 106) } else { (11, 206) };
        let ch = Character {
            slot: 0,
            name: name.into(),
            race,
            class,
            face: 0,
            hair: 0,
            stats: [60; 5],
        };
        {
            let mut db = lobby.db.lock().unwrap();
            crate::characters::ensure_account(&db, account).unwrap();
            crate::characters::set_nation(&db, account, nation).unwrap();
            assert_eq!(crate::characters::create(&mut db, account, &ch).unwrap(), 0);
        }
        let mut s = Session::new(ch).unwrap();
        s.entered = true;
        s.position = Position {
            zone,
            x: 3200,
            z: 4000,
            y: 0,
        };
        s.skills.hp = 500;
        s.skills.max_hp = 500;
        social::synchronize(lobby, account, &mut s);
        s
    }

    fn drain(lobby: &Lobby, ids: &[u16]) {
        let mut hub = lobby.social.lock().unwrap();
        for &id in ids {
            hub.take_queue(id);
        }
    }

    #[test]
    fn pvp_request_accept_cancel_reject_and_expiry_follow_native_1453_layout() {
        let lobby = test_lobby();
        let a = spawn_player(&lobby, "karuspvp", "KarusOne", 1, 21);
        let b = spawn_player(&lobby, "elmoradpvp", "ElmoOne", 2, 21);
        drain(&lobby, &[a.player_id, b.player_id]);

        // Request from KarusOne to ElmoOne
        let req = named_packet(PVP_REQUEST, "ElmoOne");
        assert!(process(&lobby, "karuspvp", &a, &req).is_none());
        {
            let mut hub = lobby.social.lock().unwrap();
            assert_eq!(
                hub.take_queue(b.player_id),
                vec![named_packet(PVP_REQUEST, "KarusOne")]
            );
            assert_eq!(
                hub.take_queue(a.player_id),
                vec![named_packet(PVP_REQ_SENT, "ElmoOne")]
            );
        }

        // Challenger cancel notifies target and clears pending
        assert!(process(&lobby, "karuspvp", &a, &[0x75, PVP_CANCEL]).is_none());
        {
            let mut hub = lobby.social.lock().unwrap();
            assert_eq!(hub.take_queue(b.player_id), vec![vec![0x75, PVP_CANCEL]]);
            assert!(!busy(&hub, a.player_id));
            assert!(!busy(&hub, b.player_id));
        }

        // Request again and reject -> notifies both
        assert!(process(&lobby, "karuspvp", &a, &req).is_none());
        drain(&lobby, &[a.player_id, b.player_id]);
        assert!(process(&lobby, "elmoradpvp", &b, &[0x75, PVP_REJECT]).is_none());
        {
            let mut hub = lobby.social.lock().unwrap();
            assert_eq!(hub.take_queue(a.player_id), vec![vec![0x75, PVP_REJECT]]);
            assert_eq!(hub.take_queue(b.player_id), vec![vec![0x75, PVP_REJECT]]);
            assert!(!busy(&hub, a.player_id));
            assert!(!busy(&hub, b.player_id));
        }

        // Expired pending challenge is pruned automatically on a new request
        assert!(process(&lobby, "karuspvp", &a, &req).is_none());
        drain(&lobby, &[a.player_id, b.player_id]);
        lobby.social.lock().unwrap().challenges[0].until = 0;
        assert!(process(&lobby, "karuspvp", &a, &req).is_none());
        {
            let mut hub = lobby.social.lock().unwrap();
            let bq = hub.take_queue(b.player_id);
            assert_eq!(bq.last(), Some(&named_packet(PVP_REQUEST, "KarusOne")));
            let aq = hub.take_queue(a.player_id);
            assert_eq!(aq.last(), Some(&named_packet(PVP_REQ_SENT, "ElmoOne")));
        }

        // Accept -> both teleport to Zone 48 at 1453 Arena coordinates
        let target_dest = process(&lobby, "elmoradpvp", &b, &[0x75, PVP_ACCEPT]);
        assert_eq!(target_dest, Some(PVP_TARGET_POS));
        {
            let mut hub = lobby.social.lock().unwrap();
            assert_eq!(
                hub.take_transfer(a.player_id),
                (Some(PVP_CHALLENGER_POS), Some(b.player_id))
            );
            assert!(hub.arena_entry_pending(b.player_id, ZONE_ARENA));
            let first = &mut hub.players.get_mut(&b.player_id).unwrap().s;
            first.position = PVP_TARGET_POS;
            first.zone_loading = true;
            assert!(hub.arena_entry_pending(b.player_id, ZONE_ARENA));
            hub.players.get_mut(&b.player_id).unwrap().s.zone_loading = false;
            assert!(!hub.arena_entry_pending(b.player_id, ZONE_ARENA));
            assert!(!busy(&hub, a.player_id));
            assert!(!busy(&hub, b.player_id));
        }
        let db = lobby.db.lock().unwrap();
        assert_eq!(
            crate::movement::load(&db, "karuspvp", 0).unwrap(),
            PVP_CHALLENGER_POS
        );
        assert_eq!(
            crate::movement::load(&db, "elmoradpvp", 0).unwrap(),
            PVP_TARGET_POS
        );
    }

    #[test]
    fn forbidden_zones_party_and_disconnect_enforce_challenge_rules() {
        let lobby = test_lobby();
        let cz_a = spawn_player(&lobby, "cza", "CzKarus", 1, 71);
        let cz_b = spawn_player(&lobby, "czb", "CzElmo", 2, 71);
        drain(&lobby, &[cz_a.player_id, cz_b.player_id]);
        assert!(process(&lobby, "cza", &cz_a, &named_packet(PVP_REQUEST, "CzElmo")).is_none());
        assert_eq!(
            lobby.social.lock().unwrap().take_queue(cz_a.player_id),
            vec![vec![0x75, ZONE_ERROR]]
        );

        // Party members cannot initiate or receive PvP challenges
        let p_a = spawn_player(&lobby, "pa", "PartyA", 1, 21);
        let p_b = spawn_player(&lobby, "pb", "PartyB", 1, 21);
        let solo = spawn_player(&lobby, "solo", "SoloC", 2, 21);
        drain(&lobby, &[p_a.player_id, p_b.player_id, solo.player_id]);
        lobby
            .social
            .lock()
            .unwrap()
            .parties
            .insert(p_a.player_id, vec![p_a.player_id, p_b.player_id]);
        assert!(process(&lobby, "pa", &p_a, &named_packet(PVP_REQUEST, "SoloC")).is_none());
        assert_eq!(
            lobby.social.lock().unwrap().take_queue(p_a.player_id),
            vec![vec![0x75, GENERIC_ERROR]]
        );
        assert!(process(&lobby, "solo", &solo, &named_packet(PVP_REQUEST, "PartyB")).is_none());
        assert_eq!(
            lobby.social.lock().unwrap().take_queue(solo.player_id),
            vec![vec![0x75, GENERIC_ERROR]]
        );

        // Disconnect cancels pending challenge and notifies peer
        let d_a = spawn_player(&lobby, "da", "DiscA", 1, 21);
        let d_b = spawn_player(&lobby, "db", "DiscB", 2, 21);
        drain(&lobby, &[d_a.player_id, d_b.player_id]);
        assert!(process(&lobby, "da", &d_a, &named_packet(PVP_REQUEST, "DiscB")).is_none());
        drain(&lobby, &[d_a.player_id, d_b.player_id]);
        lobby.social.lock().unwrap().disconnect(d_a.player_id);
        let bq = lobby.social.lock().unwrap().take_queue(d_b.player_id);
        assert!(bq.contains(&vec![0x75, PVP_CANCEL]));
    }

    #[test]
    fn cvc_requires_clan_leaders_and_transfers_same_zone_members_only() {
        let lobby = test_lobby();
        let mut l1 = spawn_player(&lobby, "c1lead", "ClanOneLead", 1, 21);
        let mut m1 = spawn_player(&lobby, "c1mem", "ClanOneMem", 1, 21);
        let mut far = spawn_player(&lobby, "c1far", "ClanOneFar", 1, 1);
        let mut l2 = spawn_player(&lobby, "c2lead", "ClanTwoLead", 2, 21);
        drain(
            &lobby,
            &[l1.player_id, m1.player_id, far.player_id, l2.player_id],
        );

        // Non-leader gets CLAN_ERROR
        assert!(
            process(
                &lobby,
                "c1lead",
                &l1,
                &named_packet(CVC_REQUEST, "ClanTwoLead")
            )
            .is_none()
        );
        assert_eq!(
            lobby.social.lock().unwrap().take_queue(l1.player_id),
            vec![vec![0x75, CLAN_ERROR]]
        );

        // Create two clans in SQLite and sync sessions
        {
            let db = lobby.db.lock().unwrap();
            db.execute(
                "INSERT INTO clans (id, nation, name) VALUES (1, 1, 'Alpha')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO clans (id, nation, name) VALUES (15001, 2, 'Beta')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO clan_members (clan_id, account, slot, fame) VALUES (1, 'c1lead', 0, 1), (1, 'c1mem', 0, 5), (1, 'c1far', 0, 5), (15001, 'c2lead', 0, 1)",
                [],
            )
            .unwrap();
        }
        social::synchronize(&lobby, "c1lead", &mut l1);
        social::synchronize(&lobby, "c1mem", &mut m1);
        social::synchronize(&lobby, "c1far", &mut far);
        social::synchronize(&lobby, "c2lead", &mut l2);
        drain(
            &lobby,
            &[l1.player_id, m1.player_id, far.player_id, l2.player_id],
        );

        assert!(
            process(
                &lobby,
                "c1lead",
                &l1,
                &named_packet(CVC_REQUEST, "ClanTwoLead")
            )
            .is_none()
        );
        drain(&lobby, &[l1.player_id, l2.player_id]);
        let dest = process(&lobby, "c2lead", &l2, &[0x75, CVC_ACCEPT]);
        assert_eq!(dest, Some(CVC_TARGET_POS));
        let hub = lobby.social.lock().unwrap();
        assert_eq!(hub.transfers.get(&l1.player_id), Some(&CVC_CHALLENGER_POS));
        assert_eq!(hub.transfers.get(&m1.player_id), Some(&CVC_CHALLENGER_POS));
        assert!(!hub.transfers.contains_key(&far.player_id));
    }

    #[test]
    fn arena_safety_area_matches_1453_lobby_bounds() {
        // Spawn coordinates inside center lobby around Judge (125, 116)
        assert!(in_arena_safety_area(PVP_TARGET_POS));
        assert!(in_arena_safety_area(PVP_CHALLENGER_POS));
        // Outside center lobby in fighting pits
        assert!(!in_arena_safety_area(Position {
            zone: ZONE_ARENA,
            x: 1900,
            z: 1900,
            y: 0,
        }));
        assert!(!in_arena_safety_area(Position {
            zone: 21,
            x: 1350,
            z: 1150,
            y: 0,
        }));
    }
}
