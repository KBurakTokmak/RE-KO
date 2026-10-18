use super::*;
use crate::{
    characters, clans,
    lobby::Lobby,
    party,
    world_entry::{Bootstrap, Session},
};
use rusqlite::Connection;
pub(crate) fn lobby() -> Lobby {
    Lobby::new(
        Connection::open_in_memory().unwrap(),
        characters::CreationRules::fixture(),
        Bootstrap { next_level_exp: 50 },
    )
    .unwrap()
}
pub(crate) fn player(lobby: &Lobby, account: &str, nation: u8, level: u8) -> Session {
    let c = characters::Character {
        slot: 0,
        name: account.into(),
        race: if nation == 1 { 2 } else { 12 },
        class: if nation == 1 { 108 } else { 208 },
        face: 0,
        hair: 0,
        stats: [60, 150, 255, 50, 50],
    };
    let mut s = Session::new(c.clone()).unwrap();
    s.entered = true;
    s.progress.level = level;
    s.skills.set_limits(500, 300);
    s.coins = 1000000;
    {
        let mut db = lobby.db.lock().unwrap();
        characters::ensure_account(&db, account).unwrap();
        characters::set_nation(&db, account, nation).unwrap();
        characters::create(&mut db, account, &c).unwrap();
        db.execute(
            "INSERT INTO character_wallets VALUES (?1,0,1000000)",
            [account],
        )
        .unwrap();
        crate::progress::save(&db, account, 0, &s.progress).unwrap();
    }
    synchronize(lobby, account, &mut s);
    s
}
fn command(sub: u8, text: &str) -> Vec<u8> {
    let mut p = vec![0x3c, sub];
    string(&mut p, text);
    p
}
fn invite(h: &mut Hub, a: u16, b: u16) {
    let name = h.players[&b].s.character.name.clone();
    let mut p = vec![0x2f, if h.party(a).is_some() { 3 } else { 1 }];
    string(&mut p, &name);
    party::process(h, a, &p);
}

#[test]
fn party_consent_capacity_leadership_chat_status_region_and_disconnect() {
    let lobby = lobby();
    let a = player(&lobby, "PartyLeader", 1, 80);
    let b = player(&lobby, "PartySecond", 2, 80);
    let outsider = player(&lobby, "Outsider", 1, 10);
    let mut h = lobby.social.lock().unwrap();
    h.queues.clear();
    invite(&mut h, a.player_id, b.player_id);
    assert_eq!(h.group(a.player_id), [a.player_id]);
    assert_eq!(&h.queues[&b.player_id][0][..2], &[0x2f, 2]);
    party::process(&mut h, outsider.player_id, &[0x2f, 2, 1]);
    assert!(h.party(outsider.player_id).is_none());
    party::process(&mut h, b.player_id, &[0x2f, 2, 1]);
    assert_eq!(h.group(a.player_id), [a.player_id, b.player_id]);
    invite(&mut h, a.player_id, outsider.player_id);
    assert_eq!(
        &h.queues[&a.player_id].last().unwrap()[..],
        &[0x2f, 3, 0xfe, 0xff]
    );
    assert!(h.chat(a.player_id, 3, b"party only"));
    assert!(
        !h.queues
            .get(&outsider.player_id)
            .is_some_and(|q| q.iter().any(|p| p.windows(10).any(|w| w == b"party only")))
    );
    let mut changed = b.clone();
    changed.skills.hp = 123;
    changed.skills.mana = 99;
    h.synchronize("PartySecond", &changed);
    assert!(h.queues[&a.player_id].iter().any(|p| p[..2] == [0x2f, 6]));
    drop(h);
    let mut others = Vec::new();
    for i in 0..7 {
        others.push(player(&lobby, &format!("Member{i}"), 1, 80));
    }
    let mut h = lobby.social.lock().unwrap();
    for p in others.iter().take(6) {
        invite(&mut h, a.player_id, p.player_id);
        party::process(&mut h, p.player_id, &[0x2f, 2, 1]);
    }
    assert_eq!(h.group(a.player_id).len(), 8);
    invite(&mut h, a.player_id, others[6].player_id);
    assert!(h.party(others[6].player_id).is_none());
    let mut promote = vec![0x2f, 0x1c];
    promote.extend(b.player_id.to_le_bytes());
    party::process(&mut h, a.player_id, &promote);
    assert_eq!(h.party(a.player_id), Some(b.player_id));
    party::leave(&mut h, a.player_id, others[0].player_id);
    assert_eq!(h.group(a.player_id).len(), 8, "non-leader cannot kick");
    h.disconnect(b.player_id);
    assert!(h.party(a.player_id).is_none());
    assert!(h.party_invites.is_empty());
    assert!(!h.players.contains_key(&b.player_id));
    let mut moved = a.clone();
    moved.position.x += 5000;
    h.synchronize("PartyLeader", &moved);
    assert!(!h.visible[&a.player_id].contains(&others[0].player_id));
}

#[test]
fn clan_creation_fee_rollback_nations_invites_roles_roster_relogin_and_disband() {
    let lobby = lobby();
    let mut a = player(&lobby, "ClanChief", 1, 80);
    let mut b = player(&lobby, "ClanJoiner", 1, 80);
    let mut enemy = player(&lobby, "EnemyChief", 2, 80);
    a.position.zone = 1;
    b.position.zone = 1;
    enemy.position.zone = 2;
    synchronize(&lobby, "ClanChief", &mut a);
    synchronize(&lobby, "ClanJoiner", &mut b);
    synchronize(&lobby, "EnemyChief", &mut enemy);
    let create = command(1, "TestClan");
    {
        let db = lobby.db.lock().unwrap();
        db.execute_batch("CREATE TRIGGER fail_clan_fee BEFORE UPDATE ON character_wallets BEGIN SELECT RAISE(ABORT,'test');END;").unwrap();
    }
    clans::process(&lobby, "ClanChief", &mut a, &create);
    assert_eq!(
        clans::load(&lobby.db.lock().unwrap(), "ClanChief", 0)
            .unwrap()
            .id,
        0
    );
    assert_eq!(a.coins, 1000000);
    lobby
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_clan_fee;")
        .unwrap();
    clans::process(&lobby, "ClanChief", &mut a, &create);
    assert_eq!(a.coins, 500000);
    assert_eq!(a.clan.fame, 1);
    assert!((1..15000).contains(&a.clan.id));
    synchronize(&lobby, "ClanChief", &mut a);
    clans::process(&lobby, "ClanJoiner", &mut b, &command(1, "testclan"));
    assert_eq!(b.clan.id, 0);
    assert_eq!(b.coins, 1000000);
    let mut p = vec![0x3c, 2];
    p.extend(enemy.player_id.to_le_bytes());
    clans::process(&lobby, "ClanChief", &mut a, &p);
    assert!(
        !lobby
            .social
            .lock()
            .unwrap()
            .clan_invites
            .contains_key(&enemy.player_id)
    );
    p[2..4].copy_from_slice(&b.player_id.to_le_bytes());
    clans::process(&lobby, "ClanChief", &mut a, &p);
    assert_eq!(b.clan.id, 0);
    let mut consent = vec![0x3c, 0x11, 1];
    consent.extend(a.player_id.to_le_bytes());
    consent.extend(a.clan.id.to_le_bytes());
    clans::process(&lobby, "ClanJoiner", &mut b, &consent);
    assert_eq!(b.clan.id, a.clan.id);
    assert_eq!(b.clan.fame, 4);
    synchronize(&lobby, "ClanJoiner", &mut b);
    clans::process(&lobby, "ClanJoiner", &mut b, &consent);
    assert_eq!(
        clans::load(&lobby.db.lock().unwrap(), "ClanJoiner", 0).unwrap(),
        b.clan
    );
    clans::process(&lobby, "ClanJoiner", &mut b, &command(10, "ClanChief"));
    assert_eq!(
        clans::load(&lobby.db.lock().unwrap(), "ClanChief", 0)
            .unwrap()
            .fame,
        1
    );
    clans::process(&lobby, "ClanChief", &mut a, &command(10, "ClanJoiner"));
    synchronize(&lobby, "ClanJoiner", &mut b);
    assert_eq!(b.clan.fame, 2);
    clans::process(&lobby, "ClanChief", &mut a, &[0x3c, 0x0d]);
    {
        let h = lobby.social.lock().unwrap();
        assert!(
            h.queues[&a.player_id]
                .iter()
                .any(|p| p.starts_with(&[0x3c, 0x0d, 1])
                    && p.windows(10).any(|w| w == b"ClanJoiner"))
        );
        assert!(
            h.players[&a.player_id]
                .s
                .my_info(&Bootstrap { next_level_exp: 50 })
                .windows(8)
                .any(|w| w == b"TestClan")
        );
    }
    clans::process(&lobby, "EnemyChief", &mut enemy, &command(1, "EnemyClan"));
    assert!(enemy.clan.id > 15000);
    {
        let db = lobby.db.lock().unwrap();
        assert!(
            db.execute(
                "INSERT INTO clan_members VALUES ('EnemyChief',0,?1,4)",
                [a.clan.id]
            )
            .is_err()
        );
    }
    clans::process(&lobby, "ClanJoiner", &mut b, &[0x3c, 5]);
    assert_ne!(
        clans::load(&lobby.db.lock().unwrap(), "ClanChief", 0)
            .unwrap()
            .id,
        0
    );
    clans::process(&lobby, "ClanChief", &mut a, &[0x3c, 5]);
    assert_eq!(
        clans::load(&lobby.db.lock().unwrap(), "ClanJoiner", 0)
            .unwrap()
            .id,
        0
    );
    assert_ne!(
        clans::load(&lobby.db.lock().unwrap(), "EnemyChief", 0)
            .unwrap()
            .id,
        0
    );
    assert_eq!(
        crate::starter::coins(&lobby.db.lock().unwrap(), "ClanChief", 0).unwrap(),
        500000
    );
    assert!(
        lobby
            .db
            .lock()
            .unwrap()
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}

#[test]
fn party_death_credits_group_contributions_once_and_share_nearby_kills() {
    let lobby = lobby();
    let a = player(&lobby, "DamageA", 1, 80);
    let b = player(&lobby, "DamageB", 1, 80);
    let c = player(&lobby, "Solo", 1, 80);
    let mut h = lobby.social.lock().unwrap();
    invite(&mut h, a.player_id, b.player_id);
    party::process(&mut h, b.player_id, &[0x2f, 2, 1]);
    let damage = HashMap::from([
        ("DamageA".into(), 70),
        ("DamageB".into(), 20),
        ("Solo".into(), 10),
    ]);
    let (credits, kills) = h.death(100, 6, a.position, &damage);
    assert_eq!(credits.len(), 3);
    assert_eq!(kills.len(), 2);
    assert!(
        credits
            .iter()
            .filter(|(_, c)| c.party.is_some())
            .all(|(_, c)| c.damage == 90 && c.total_damage == 100 && c.party == Some((2, 160)))
    );
    assert_eq!(
        credits
            .iter()
            .find(|(name, _)| name == "Solo")
            .unwrap()
            .1
            .damage,
        10
    );
    let mut far = b;
    far.position.x += 1000;
    h.synchronize("DamageB", &far);
    let (credits, kills) = h.death(100, 6, a.position, &damage);
    assert_eq!(kills, ["DamageA"]);
    assert_eq!(credits.len(), 2);
    assert_eq!(h.group(c.player_id), [c.player_id]);
}

#[tokio::test]
async fn native_two_client_party_invitation_roster_chat_and_clan_creation_use_the_game_dispatcher()
{
    use tokio::{
        io::AsyncReadExt,
        net::{TcpListener, TcpStream},
    };
    async fn pair() -> (TcpStream, TcpStream, std::net::SocketAddr) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        let (server, peer) = l.accept().await.unwrap();
        (client, server, peer)
    }
    async fn receive(client: &mut TcpStream, prefix: &[u8]) -> Vec<u8> {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let mut head = [0; 4];
                client.read_exact(&mut head).await.unwrap();
                let len = u16::from_le_bytes([head[2], head[3]]) as usize;
                let mut p = vec![0; len + 2];
                client.read_exact(&mut p).await.unwrap();
                if p.starts_with(prefix) {
                    return p[..len].to_vec();
                }
            }
        })
        .await
        .unwrap()
    }
    let lobby = lobby();
    let mut a = player(&lobby, "SocketLeader", 1, 80);
    let mut b = player(&lobby, "SocketMember", 1, 80);
    a.position.zone = 1;
    b.position.zone = 1;
    synchronize(&lobby, "SocketLeader", &mut a);
    synchronize(&lobby, "SocketMember", &mut b);
    lobby.social.lock().unwrap().queues.clear();
    let (mut ca, mut sa, pa) = pair().await;
    let (mut cb, mut sb, pb) = pair().await;
    let mut aa = Some("SocketLeader".to_owned());
    let mut ab = Some("SocketMember".to_owned());
    let mut wa = Some(a);
    let mut wb = Some(b);
    let mut invite = vec![0x2f, 1];
    string(&mut invite, "SocketMember");
    crate::handle_game_payload(&mut sa, pa, &invite, &lobby, &mut aa, &mut wa).await;
    flush(&mut sb, pb, &lobby, wb.as_ref().unwrap()).await;
    let p = receive(&mut cb, &[0x2f, 2]).await;
    assert_eq!(
        u16::from_le_bytes([p[2], p[3]]),
        wa.as_ref().unwrap().player_id
    );
    assert_eq!(&p[6..], b"SocketLeader");
    crate::handle_game_payload(&mut sb, pb, &[0x2f, 2, 1], &lobby, &mut ab, &mut wb).await;
    let member = receive(&mut cb, &[0x2f, 3]).await;
    assert_eq!(member[4], 1);
    assert_eq!(u16::from_le_bytes([member[5], member[6]]), 12);
    let mut chat = vec![0x10, 3];
    string(&mut chat, "Hello party");
    crate::handle_game_payload(&mut sa, pa, &chat, &lobby, &mut aa, &mut wa).await;
    flush(&mut sb, pb, &lobby, wb.as_ref().unwrap()).await;
    let p = receive(&mut cb, &[0x10, 3]).await;
    assert!(p.ends_with(b"Hello party"));
    crate::handle_game_payload(
        &mut sa,
        pa,
        &command(1, "SocketClan"),
        &lobby,
        &mut aa,
        &mut wa,
    )
    .await;
    let p = receive(&mut ca, &[0x3c, 1, 1]).await;
    assert_eq!(
        u32::from_le_bytes(p[p.len() - 4..].try_into().unwrap()),
        500000
    );
    assert_eq!(wa.as_ref().unwrap().clan.name, "SocketClan");
}
