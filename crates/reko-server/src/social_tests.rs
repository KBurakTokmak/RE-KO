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

#[tokio::test]
async fn gm_visibility_filters_regions_details_and_revoked_commands() {
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let lobby = lobby();
    let mut gm = player(&lobby, "HiddenGM", 1, 80);
    let mut observer = player(&lobby, "ObserverGM", 1, 80);
    let normal = player(&lobby, "NormalUser", 1, 80);
    for s in [&mut gm, &mut observer] {
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO character_privileges VALUES(?1,0,1)",
                [&s.character.name],
            )
            .unwrap();
        s.is_admin = true;
        synchronize(&lobby, &s.character.name.clone(), s);
    }
    {
        let hub = lobby.social.lock().unwrap();
        assert!(hub.visible[&observer.player_id].contains(&gm.player_id));
        assert!(!hub.visible[&normal.player_id].contains(&gm.player_id));
        assert!(hub.visible[&gm.player_id].contains(&normal.player_id));
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut sock, peer) = listener.accept().await.unwrap();
    async fn read(c: &mut TcpStream) -> Vec<u8> {
        let mut h = [0; 4];
        c.read_exact(&mut h).await.unwrap();
        let n = u16::from_le_bytes([h[2], h[3]]) as usize;
        let mut p = vec![0; n + 2];
        c.read_exact(&mut p).await.unwrap();
        p.truncate(n);
        p
    }
    let mut account = Some("NormalUser".into());
    let mut world = Some(normal.clone());
    let mut details = vec![0x16, 1, 0];
    details.extend(gm.player_id.to_le_bytes());
    lobby.social.lock().unwrap().queues.clear();
    crate::handle_game_payload(&mut sock, peer, &details, &lobby, &mut account, &mut world).await;
    assert_eq!(read(&mut client).await, [0x16, 0, 0]);
    assert!(
        visibility_command(
            &mut sock,
            peer,
            &lobby,
            "NormalUser",
            world.as_mut().unwrap(),
            b"gmvisible on"
        )
        .await
    );
    assert!(
        read(&mut client)
            .await
            .ends_with(b"requires an admin character.")
    );
    assert!(
        !lobby
            .social
            .lock()
            .unwrap()
            .gm_public
            .contains(&normal.player_id)
    );
    for (command, visible) in [
        ("gmvisible on", true),
        ("gmvisible invalid", true),
        ("gmvisible off", false),
    ] {
        visibility_command(
            &mut sock,
            peer,
            &lobby,
            "HiddenGM",
            &mut gm,
            command.as_bytes(),
        )
        .await;
        assert_eq!(&read(&mut client).await[..2], &[0x10, 7]);
        let mut hub = lobby.social.lock().unwrap();
        assert_eq!(
            hub.visible[&normal.player_id].contains(&gm.player_id),
            visible
        );
        assert!(hub.visible[&observer.player_id].contains(&gm.player_id));
        if command == "gmvisible on" {
            assert!(
                hub.queues[&normal.player_id]
                    .iter()
                    .any(|p| p.starts_with(&[7, 1]))
            );
        }
        if command == "gmvisible off" {
            assert!(
                hub.queues[&normal.player_id]
                    .iter()
                    .any(|p| p.starts_with(&[7, 2]))
            );
        }
        hub.queues.clear();
    }
    crate::handle_game_payload(&mut sock, peer, &details, &lobby, &mut account, &mut world).await;
    assert_eq!(read(&mut client).await, [0x16, 0, 0]);
    visibility_command(
        &mut sock,
        peer,
        &lobby,
        "HiddenGM",
        &mut gm,
        b"gmvisible on",
    )
    .await;
    read(&mut client).await;
    lobby.social.lock().unwrap().queues.clear();
    crate::handle_game_payload(&mut sock, peer, &details, &lobby, &mut account, &mut world).await;
    assert_eq!(&read(&mut client).await[..3], &[0x16, 1, 0]);
    lobby
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE character_privileges SET is_admin=0 WHERE account='HiddenGM'",
            [],
        )
        .unwrap();
    visibility_command(
        &mut sock,
        peer,
        &lobby,
        "HiddenGM",
        &mut gm,
        b"gmvisible off",
    )
    .await;
    assert!(
        read(&mut client)
            .await
            .ends_with(b"requires an admin character.")
    );
    synchronize(&lobby, "HiddenGM", &mut gm);
    let mut hub = lobby.social.lock().unwrap();
    assert!(!hub.gm_public.contains(&gm.player_id));
    assert!(!gm.is_admin);
    hub.gm_public.insert(observer.player_id);
    hub.disconnect(observer.player_id);
    hub.synchronize("ObserverGM", &observer);
    assert!(!hub.gm_public.contains(&observer.player_id));
    assert!(!hub.visible[&normal.player_id].contains(&observer.player_id));
}

#[test]
fn administrators_keep_local_gm_privileges_but_have_visible_remote_authority() {
    let lobby = lobby();
    let mut a = player(&lobby, "VisibleAdmin", 1, 80);
    let mut b = player(&lobby, "OtherAdmin", 1, 80);
    for s in [&mut a, &mut b] {
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO character_privileges VALUES(?1,0,1)",
                [&s.character.name],
            )
            .unwrap();
        s.is_admin = true;
        synchronize(&lobby, &s.character.name.clone(), s);
    }
    let hub = lobby.social.lock().unwrap();
    for s in [&a, &b] {
        let packet = hub.user_in(s.player_id);
        let mut cursor = 4;
        cursor += 1 + usize::from(packet[cursor]);
        cursor += 1 + 2 + 1 + 2;
        cursor += 1 + usize::from(packet[cursor]);
        cursor += 1 + 1 + 2 + 2 + 1 + 1 + 2 + 6 + 3 + 4 + 1;
        assert_eq!(
            packet[cursor], 1,
            "Native remote GM authority0 hides an otherwise normal appearance"
        );
        let local = s.my_info(&lobby.bootstrap);
        let mut ordinary = s.clone();
        ordinary.is_admin = false;
        let normal = ordinary.my_info(&lobby.bootstrap);
        let differences: Vec<_> = local
            .iter()
            .zip(normal)
            .filter(|(a, b)| **a != *b)
            .map(|(a, b)| (*a, b))
            .collect();
        assert_eq!(
            differences,
            [(0, 1)],
            "Local GM authority/collision capability remains intact"
        );
        assert!(crate::admin::is_admin(&lobby.db.lock().unwrap(), &s.character.name, 0).unwrap());
        assert!(hub.players[&s.player_id].s.is_admin);
    }
}

#[test]
fn party_loot_rotation_follows_the_next_member_across_promotion_removal_and_disband() {
    let lobby = lobby();
    let a = player(&lobby, "RotationA", 1, 80);
    let b = player(&lobby, "RotationB", 1, 80);
    let c = player(&lobby, "RotationC", 1, 80);
    let mut h = lobby.social.lock().unwrap();
    h.parties
        .insert(a.player_id, vec![a.player_id, b.player_id, c.player_id]);
    h.loot_rotation.insert(a.player_id, 1);
    let mut promote = vec![0x2f, 0x1c];
    promote.extend(c.player_id.to_le_bytes());
    party::process(&mut h, a.player_id, &promote);
    assert!(!h.loot_rotation.contains_key(&a.player_id));
    assert_eq!(
        h.parties[&c.player_id][h.loot_rotation[&c.player_id]],
        b.player_id
    );
    party::leave(&mut h, c.player_id, b.player_id);
    assert_eq!(
        h.parties[&c.player_id][h.loot_rotation[&c.player_id]],
        a.player_id
    );
    party::leave(&mut h, c.player_id, c.player_id);
    assert!(!h.loot_rotation.contains_key(&c.player_id));
    invite(&mut h, c.player_id, a.player_id);
    assert_eq!(*h.loot_rotation.get(&c.player_id).unwrap_or(&0), 0);
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
async fn three_loaded_clients_receive_complete_regions_and_can_request_each_others_native_records()
{
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let lobby = lobby();
    let mut a = player(&lobby, "FirstClient", 1, 80);
    let b = player(&lobby, "SecondClient", 1, 80);
    let c = player(&lobby, "ThirdClient", 2, 80);
    let mut distant = player(&lobby, "DistantClient", 1, 80);
    distant.position.zone = 1;
    synchronize(&lobby, "DistantClient", &mut distant);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut server, peer) = listener.accept().await.unwrap();
    async fn read(client: &mut TcpStream) -> Vec<u8> {
        let mut head = [0; 4];
        client.read_exact(&mut head).await.unwrap();
        let length = u16::from_le_bytes([head[2], head[3]]) as usize;
        let mut data = vec![0; length + 2];
        client.read_exact(&mut data).await.unwrap();
        data.truncate(length);
        data
    }
    let mut start = vec![0x0d, 2, a.character.name.len() as u8];
    start.extend(a.character.name.as_bytes());
    let mut account = Some("FirstClient".into());
    let mut world = Some(a.clone());
    lobby.social.lock().unwrap().queues.clear();
    crate::handle_game_payload(&mut server, peer, &start, &lobby, &mut account, &mut world).await;
    assert!(
        lobby.social.lock().unwrap().queues.is_empty(),
        "Stage2 cannot complete before the snapshot"
    );
    a.snapshot_sent = true;
    world = Some(a.clone());
    crate::handle_game_payload(&mut server, peer, &start, &lobby, &mut account, &mut world).await;
    let region = read(&mut client).await;
    assert_eq!(&region[..3], &[0x15, 3, 0]);
    let ids: Vec<_> = region[3..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect();
    let mut expected = vec![a.player_id, b.player_id, c.player_id];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    {
        let hub = lobby.social.lock().unwrap();
        for id in [b.player_id, c.player_id] {
            assert!(hub.queues[&id].iter().any(|packet| packet == &region));
        }
        assert!(!hub.queues.contains_key(&distant.player_id));
    }
    let mut request = vec![0x16, 2, 0];
    request.extend(b.player_id.to_le_bytes());
    request.extend(c.player_id.to_le_bytes());
    crate::handle_game_payload(
        &mut server,
        peer,
        &request,
        &lobby,
        &mut account,
        &mut world,
    )
    .await;
    let records = read(&mut client).await;
    assert_eq!(&records[..3], &[0x16, 2, 0]);
    let mut cursor = 3;
    for other in [&b, &c] {
        assert_eq!(
            u16::from_le_bytes(records[cursor..cursor + 2].try_into().unwrap()),
            other.player_id
        );
        cursor += 2;
        let length = usize::from(records[cursor]);
        cursor += 1;
        assert_eq!(
            &records[cursor..cursor + length],
            other.character.name.as_bytes()
        );
        cursor += length;
        assert_eq!(records[cursor], other.character.nation());
        cursor += 1 + 2 + 1 + 2;
        let clan_length = usize::from(records[cursor]);
        cursor += 1 + clan_length;
        cursor += 1 + 1 + 2 + 2;
        assert_eq!(&records[cursor..cursor + 2], &[80, other.character.race]);
        cursor += 2 + 2 + 6 + 3 + 4 + 5 + 2 + 4 + 10 * 7;
        assert_eq!(records[cursor], other.position.zone);
        cursor += 1;
    }
    assert_eq!(
        cursor,
        records.len(),
        "Native1453 consumes all records without padding or shifted zone bytes"
    );
    let before = world.as_ref().unwrap().clone();
    crate::handle_game_payload(&mut server, peer, &start, &lobby, &mut account, &mut world).await;
    assert_eq!(
        read(&mut client).await,
        region,
        "Stage2 replay refreshes presence without replaying inventory/vitals"
    );
    assert_eq!(world.as_ref().unwrap().inventory, before.inventory);
    assert_eq!(world.as_ref().unwrap().coins, before.coins);
    let mut forged = vec![0x16, 1, 0];
    forged.extend(distant.player_id.to_le_bytes());
    crate::handle_game_payload(&mut server, peer, &forged, &lobby, &mut account, &mut world).await;
    assert_eq!(read(&mut client).await, [0x16, 0, 0]);
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
