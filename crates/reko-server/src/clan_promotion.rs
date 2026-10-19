//! Original clerk trial, promoted clan persistence and cape selection.
use crate::{
    clans, combat, lobby::Lobby, movement::Position, npc_scripts::Context, world_entry::Session,
};
use rusqlite::{Connection, params};
use std::net::SocketAddr;
use tokio::net::TcpStream;

const SCRIPT: &str = "reko_clan_trial.lua";
const KEY: u32 = 910045000;
const FEE: u32 = 10000000;

fn notice(s: &Session, text: &str) -> Vec<u8> {
    crate::chat::packet(7, s.character.nation(), u16::MAX, "", text.as_bytes())
}
fn menu(s: &mut Session, npc: u16, prototype: u16, talk: i32, buttons: &[(i32, i32)]) -> Vec<u8> {
    let mut context = Context {
        npc,
        file: SCRIPT.into(),
        choices: [-1; 10],
        ..Context::default()
    };
    let mut p = vec![0x55];
    p.extend(prototype.to_le_bytes());
    p.push(if buttons.len() > 2 { 3 } else { 2 });
    p.extend((-1i32).to_le_bytes());
    p.extend(talk.to_le_bytes());
    for i in 0..12 {
        let (button, event) = buttons.get(i).copied().unwrap_or((-1, -1));
        p.extend(button.to_le_bytes());
        if i < 10 {
            context.choices[i] = event;
        }
    }
    p.push(SCRIPT.len() as u8);
    p.extend(SCRIPT.as_bytes());
    s.npc_dialog = Some(context);
    p
}
fn chief(db: &Connection, account: &str, s: &Session) -> anyhow::Result<clans::Info> {
    let info = clans::load(db, account, s.character.slot)?;
    anyhow::ensure!(
        info.id != 0 && info.fame == 1,
        "Only the clan leader can undertake this mission."
    );
    anyhow::ensure!(
        info.rank == 1,
        "Your clan has already earned knight privileges."
    );
    anyhow::ensure!(
        info.grade <= 3,
        "Your clan must reach Grade 3 (144000 total national points) first."
    );
    Ok(info)
}
fn update(info: &clans::Info) -> Vec<u8> {
    let mut p = vec![0x3c, 0x24];
    p.extend(info.id.to_le_bytes());
    p.push(info.rank);
    p.extend(info.cape.to_le_bytes());
    p
}
fn publish(lobby: &Lobby, info: &clans::Info) {
    let mut hub = lobby.social.lock().unwrap();
    let ids = hub
        .players
        .iter()
        .filter(|(_, p)| p.s.clan.id == info.id)
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    hub.send_many(&ids, &update(info));
}
fn promote(db: &Connection, account: &str, s: &Session) -> anyhow::Result<clans::Info> {
    let tx = db.unchecked_transaction()?;
    let info = chief(&tx, account, s)?;
    let key: (u8, u16) = tx.query_row("SELECT inventory_slot,count FROM character_inventory WHERE account=?1 AND slot=?2 AND item_id=?3 AND count>0 ORDER BY inventory_slot LIMIT 1", params![account,s.character.slot,KEY], |r| Ok((r.get(0)?,r.get(1)?)))
        .map_err(|_| anyhow::anyhow!("You need the Key of Honor dropped by Caitharos."))?;
    anyhow::ensure!(tx.execute("UPDATE character_wallets SET coins=coins-?3 WHERE account=?1 AND slot=?2 AND coins>=?3", params![account,s.character.slot,FEE])? == 1, "You need 10000000 Noah to promote your clan.");
    if key.1 == 1 {
        tx.execute(
            "DELETE FROM character_inventory WHERE account=?1 AND slot=?2 AND inventory_slot=?3",
            params![account, s.character.slot, key.0],
        )?;
    } else {
        tx.execute("UPDATE character_inventory SET count=count-1 WHERE account=?1 AND slot=?2 AND inventory_slot=?3", params![account,s.character.slot,key.0])?;
    }
    tx.execute("INSERT INTO clan_progress VALUES (?1,2,0) ON CONFLICT(clan_id) DO UPDATE SET rank=2,cape=0", [info.id])?;
    let info = clans::load(&tx, account, s.character.slot)?;
    tx.commit()?;
    Ok(info)
}
fn start(lobby: &Lobby, account: &str, s: &mut Session) -> anyhow::Result<Vec<u8>> {
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let info = chief(&tx, account, s)?;
    let configured: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM world_town_starts WHERE zone=54) AND EXISTS(SELECT 1 FROM world_spawns WHERE zone=54 AND prototype=2690 AND monster=1)", [], |r| r.get(0))?;
    anyhow::ensure!(configured, "The clan trial zone is unavailable.");
    let destination = Position {
        zone: 54,
        x: 1500,
        z: 1500,
        y: 0,
    };
    let mut hub = lobby.social.lock().unwrap();
    let mut targets = Vec::new();
    for (&id, player) in &hub.players {
        if id == s.player_id
            || !player.s.entered
            || player.s.skills.hp == 0
            || player.s.position.zone >= 30
        {
            continue;
        }
        let membership = clans::load(&tx, &player.account, player.s.character.slot)?;
        if membership.id == info.id {
            crate::movement::save(&tx, &player.account, player.s.character.slot, destination)?;
            targets.push(id);
        }
    }
    crate::movement::save(&tx, account, s.character.slot, destination)?;
    tx.commit()?;
    for id in targets {
        hub.transfers.insert(id, destination);
    }
    drop(hub);
    drop(db);
    Ok(crate::travel::begin(lobby, s, destination))
}
pub(crate) async fn transfer(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    s: &mut Session,
) {
    let destination = lobby.social.lock().unwrap().transfers.remove(&s.player_id);
    if let Some(destination) = destination {
        let p = crate::travel::begin(lobby, s, destination);
        crate::send_response(sock, peer, &None, &p, "CLAN_TRIAL_TRANSFER").await;
    }
}
fn home(lobby: &Lobby, account: &str, s: &mut Session) -> anyhow::Result<Vec<u8>> {
    let nation = s.character.nation();
    let db = lobby.db.lock().unwrap();
    let (kx, kz, ex, ez): (u16, u16, u16, u16) = db.query_row(
        "SELECT karus_x,karus_z,elmo_x,elmo_z FROM world_town_starts WHERE zone=?1",
        [nation],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let (x, z) = if nation == 1 { (kx, kz) } else { (ex, ez) };
    let destination = Position {
        zone: nation,
        x: x.checked_mul(10)
            .ok_or_else(|| anyhow::anyhow!("Invalid home coordinates."))?,
        z: z.checked_mul(10)
            .ok_or_else(|| anyhow::anyhow!("Invalid home coordinates."))?,
        y: 0,
    };
    crate::movement::save(&db, account, s.character.slot, destination)?;
    drop(db);
    Ok(crate::travel::begin(lobby, s, destination))
}
fn process(
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) -> anyhow::Result<Vec<Vec<u8>>> {
    anyhow::ensure!(
        s.entered && !s.zone_loading && s.skills.hp > 0,
        "Your character is unavailable."
    );
    let (npc, event) = if p.first() == Some(&0x20) && matches!(p.len(), 3 | 7) {
        (u16::from_le_bytes([p[1], p[2]]), 0)
    } else {
        let context = s
            .npc_dialog
            .take()
            .ok_or_else(|| anyhow::anyhow!("No active clan dialog."))?;
        anyhow::ensure!(
            p.len() >= 2 && p[0] == 0x55 && p[1] < 10,
            "Invalid clan menu choice."
        );
        if p.len() > 2 {
            let n = usize::from(p[2]);
            anyhow::ensure!(
                (p.len() == 3 + n || p.len() == 4 + n)
                    && (n == 0 || p.get(3..3 + n) == Some(SCRIPT.as_bytes())),
                "Invalid clan dialog filename."
            );
        }
        let event = context.choices[usize::from(p[1])];
        anyhow::ensure!(event >= 0, "Invalid clan menu choice.");
        (context.npc, event)
    };
    let merchant = lobby
        .merchants
        .iter()
        .find(|m| m.id == npc && m.nearby(s))
        .ok_or_else(|| anyhow::anyhow!("The knight clerk is out of range."))?;
    if merchant.prototype == 13010 && s.position.zone == 54 {
        return if event == 0 {
            Ok(vec![menu(
                s,
                npc,
                merchant.prototype,
                4072,
                &[(4071, 10), (4072, 11)],
            )])
        } else if event == 10 {
            Ok(vec![home(lobby, account, s)?])
        } else {
            Ok(Vec::new())
        };
    }
    anyhow::ensure!(
        merchant.prototype
            == if s.character.nation() == 1 {
                21610
            } else {
                11610
            },
        "Visit your nation's knight clerk."
    );
    let packet = match event {
        0 => menu(s, npc, merchant.prototype, 4150, &[(4150, 1)]),
        1 => {
            chief(&lobby.db.lock().unwrap(), account, s)?;
            menu(
                s,
                npc,
                merchant.prototype,
                if s.character.nation() == 1 {
                    4165
                } else {
                    4265
                },
                &[(4157, 2), (4158, 3), (4159, 4)],
            )
        }
        2 => menu(
            s,
            npc,
            merchant.prototype,
            if s.character.nation() == 1 {
                4168
            } else {
                4268
            },
            &[(4160, 1)],
        ),
        3 => {
            chief(&lobby.db.lock().unwrap(), account, s)?;
            menu(
                s,
                npc,
                merchant.prototype,
                if s.character.nation() == 1 {
                    4172
                } else {
                    4272
                },
                &[(4161, 5), (4162, 6)],
            )
        }
        4 => return Ok(vec![start(lobby, account, s)?]),
        5 => {
            let old = s.coins;
            let before = s.inventory;
            {
                let db = lobby.db.lock().unwrap();
                s.clan = promote(&db, account, s)?;
                s.coins = crate::starter::coins(&db, account, s.character.slot)?;
                s.inventory = crate::starter::inventory(&db, account, s.character.slot)?;
            }
            let changed = s
                .inventory
                .iter()
                .copied()
                .enumerate()
                .filter(|(i, item)| before[*i] != *item)
                .collect::<Vec<_>>();
            let mut packets = vec![
                crate::wallet::change(old, s.coins),
                crate::inventory::count_packet(&changed),
                notice(
                    s,
                    "Your clan has earned knight privileges. Visit the cape merchant to choose a cape.",
                ),
            ];
            if let Some(catalog) = &lobby.combat {
                crate::durability::refresh(s, catalog, lobby.skills.as_ref());
                let mut weight = vec![0x54];
                weight.extend(s.combat.weight.to_le_bytes());
                packets.push(weight);
            }
            publish(lobby, &s.clan);
            return Ok(packets);
        }
        _ => return Ok(Vec::new()),
    };
    Ok(vec![packet])
}
pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) -> bool {
    let original_mission = p.first() == Some(&0x55)
        && p.get(1).is_some_and(|i| *i < 10)
        && s.npc_dialog.as_ref().is_some_and(|c| {
            c.file != SCRIPT
                && c.choices[usize::from(p[1])] == 100
                && lobby
                    .merchants
                    .iter()
                    .any(|m| m.id == c.npc && matches!(m.prototype, 11610 | 21610))
        });
    let relevant = original_mission
        || match p.first() {
            Some(0x20) if matches!(p.len(), 3 | 7) => {
                let id = u16::from_le_bytes([p[1], p[2]]);
                lobby
                    .merchants
                    .iter()
                    .any(|m| m.id == id && m.prototype == 13010 && s.position.zone == 54)
            }
            Some(0x55) => s.npc_dialog.as_ref().is_some_and(|c| c.file == SCRIPT),
            _ => false,
        };
    if !relevant {
        return false;
    }
    if original_mission {
        let context = s.npc_dialog.as_ref().unwrap();
        if p.len() > 2 {
            let n = usize::from(p[2]);
            if !(p.len() == 3 + n || p.len() == 4 + n)
                || (n != 0 && p.get(3..3 + n) != Some(context.file.as_bytes()))
            {
                s.npc_dialog = None;
                return true;
            }
        }
        let context = s.npc_dialog.as_mut().unwrap();
        context.file = SCRIPT.into();
        context.choices = [-1; 10];
        context.choices[0] = 1;
    }
    let request = if original_mission { &[0x55, 0][..] } else { p };
    let packets = match process(lobby, account, s, request) {
        Ok(p) => p,
        Err(e) => {
            s.npc_dialog = None;
            vec![notice(s, &e.to_string())]
        }
    };
    for p in packets {
        crate::send_response(sock, peer, &None, &p, "CLAN_PROMOTION").await;
    }
    true
}

fn purchase(lobby: &Lobby, account: &str, s: &mut Session, p: &[u8]) -> Result<Vec<Vec<u8>>, i16> {
    if p.len() != 3
        || p[0] != 0x70
        || !s.entered
        || s.skills.hp == 0
        || !lobby
            .merchants
            .iter()
            .any(|m| Some(m.id) == s.merchant && m.kind == 25 && m.nearby(s))
    {
        return Err(-1);
    }
    let cape = u16::from_le_bytes([p[1], p[2]]);
    let table = reko_assets::Table::from_encrypted_bytes(
        &std::fs::read(reko_common::game_dir().join("Data/Cloak.tbl")).map_err(|_| -5i16)?,
    )
    .map_err(|_| -5i16)?;
    let row = table
        .rows
        .iter()
        .find(|r| combat::integer(r, 0) == i32::from(cape) && cape != 99)
        .ok_or(-5i16)?;
    let cost = u32::try_from(combat::integer(row, 2)).map_err(|_| -5i16)?;
    // The original table's fourth column is the clan point cost; these capes require none.
    if combat::integer(row, 3) != 0 {
        return Err(-7);
    }
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction().map_err(|_| -2i16)?;
    let info = clans::load(&tx, account, s.character.slot).map_err(|_| -2i16)?;
    if info.id == 0 || info.fame != 1 || info.rank != 2 {
        return Err(-1);
    }
    let required = combat::integer(row, 4);
    if required > 0 && i32::from(info.grade) > required {
        return Err(-6);
    }
    let old = crate::starter::coins(&tx, account, s.character.slot).map_err(|_| -7i16)?;
    let cost = if info.cape == cape { 0 } else { cost };
    if tx.execute("UPDATE character_wallets SET coins=coins-?3 WHERE account=?1 AND slot=?2 AND coins>=?3",params![account,s.character.slot,cost]).map_err(|_|-7i16)?!=1 {return Err(-7);}
    tx.execute(
        "UPDATE clan_progress SET cape=?2 WHERE clan_id=?1",
        params![info.id, cape],
    )
    .map_err(|_| -2i16)?;
    tx.commit().map_err(|_| -2i16)?;
    s.coins = old - cost;
    s.clan = clans::Info { cape, ..info };
    drop(db);
    publish(lobby, &s.clan);
    let mut response = vec![0x70];
    for n in [1, 0, s.clan.id, cape] {
        response.extend(n.to_le_bytes());
    }
    let mut packets = vec![response];
    if cost != 0 {
        packets.push(crate::wallet::change(old, s.coins));
    }
    Ok(packets)
}
pub(crate) async fn cape(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    p: &[u8],
) {
    let packets = purchase(lobby, account, s, p).unwrap_or_else(|code| {
        let mut p = vec![0x70];
        p.extend(code.to_le_bytes());
        vec![
            p,
            notice(
                s,
                match code {
                    -5 => "This cape is unavailable.",
                    -6 => "Your clan grade is too low for this cape.",
                    -7 => "You do not have enough Noah for this cape.",
                    _ => "Only a promoted clan's leader can select a cape at the cape merchant.",
                },
            ),
        ]
    });
    for p in packets {
        crate::send_response(sock, peer, &None, &p, "CAPE_CHANGE").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn existing_clerk_menu_enters_safe_original_trial_and_claims_over_socket() {
        let (mut lobby, mut s, _) = fixture();
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let script = game
            .parent()
            .unwrap()
            .join("kn_2086_patch_private_server/lCAFILES/Quests/21610_delaga.lua");
        if !script.exists()
            || !std::path::Path::new("../../logs/reference-coefficients.csv").exists()
        {
            return;
        }
        {
            let db = lobby.db.lock().unwrap();
            db.execute("UPDATE world_spawns SET monster=0,prototype=21610,appearance=22000,kind=26,zone=?1,x0=?2,x1=?2,z0=?3,z1=?3",params![s.position.zone,s.position.x,s.position.z]).unwrap();
            db.execute("INSERT INTO world_reference_rules VALUES ('QUEST_HELPER',4150,?1)",[r#"{"sNpcId":"21610","nEventTriggerIndex":"240","strLuaFilename":"21610_delaga.lua"}"#]).unwrap();
            lobby.merchants = crate::shops::load_population(&db, &game).unwrap();
        }
        lobby.combat = Some(
            combat::Catalog::load(
                &game,
                std::path::Path::new("../../logs/reference-coefficients.csv"),
            )
            .unwrap(),
        );
        s.combat = lobby
            .combat
            .as_ref()
            .unwrap()
            .calculate(&s.character, &s.progress, &s.equipment)
            .unwrap();
        s.base_combat = s.combat;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut open = vec![0x20];
        open.extend(12000u16.to_le_bytes());
        crate::npc_scripts::handle(&mut socket, peer, &lobby, "Chief", &mut s, &open).await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 100);
        crate::npc_scripts::handle(&mut socket, peer, &lobby, "Chief", &mut s, &[0x55, 0]).await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().file, SCRIPT);
        assert_eq!(&s.npc_dialog.as_ref().unwrap().choices[..3], &[2, 3, 4]);
        crate::npc_scripts::handle(&mut socket, peer, &lobby, "Chief", &mut s, &[0x55, 1]).await;
        assert_eq!(s.npc_dialog.as_ref().unwrap().choices[0], 5);
        crate::npc_scripts::handle(&mut socket, peer, &lobby, "Chief", &mut s, &[0x55, 0]).await;
        assert_eq!(s.clan.rank, 2);
        assert_eq!(s.coins, 10000000);
        crate::npc_scripts::handle(&mut socket, peer, &lobby, "Chief", &mut s, &[0x55, 0]).await;
        assert_eq!(s.coins, 10000000);
    }
    fn fixture() -> (Lobby, Session, Session) {
        let lobby = crate::social::tests::lobby();
        let mut leader = crate::social::tests::player(&lobby, "Chief", 1, 80);
        let mut member = crate::social::tests::player(&lobby, "Member", 1, 80);
        let db = lobby.db.lock().unwrap();
        db.execute_batch(
            "INSERT INTO clans VALUES (1,'TrialClan',1);
            INSERT INTO clan_members VALUES ('Chief',0,1,1),('Member',0,1,4);
            UPDATE character_quest_rewards SET loyalty=72000;
            UPDATE character_wallets SET coins=20000000;
            INSERT INTO character_inventory VALUES ('Chief',0,0,910045000,1,1);
            INSERT INTO world_town_starts VALUES (54,150,150,150,150,0,0,'test');
            INSERT INTO world_town_starts VALUES (1,368,1688,0,0,0,0,'test');",
        )
        .unwrap();
        crate::population::tests::seed(&db);
        db.execute("UPDATE world_spawns SET zone=54,prototype=2690", [])
            .unwrap();
        drop(db);
        leader.position.zone = 1;
        member.position.zone = 1;
        crate::social::synchronize(&lobby, "Chief", &mut leader);
        crate::social::synchronize(&lobby, "Member", &mut member);
        (lobby, leader, member)
    }
    #[test]
    fn trial_checks_grade_and_live_membership_and_commits_clan_transfers() {
        let (lobby, mut leader, mut member) = fixture();
        assert_eq!(leader.clan.grade, 3);
        assert!(start(&lobby, "Member", &mut member).is_err());
        {
            let db = lobby.db.lock().unwrap();
            db.execute("UPDATE character_quest_rewards SET loyalty=71999", [])
                .unwrap();
            assert_eq!(clans::load(&db, "Chief", 0).unwrap().grade, 4);
        }
        assert!(start(&lobby, "Chief", &mut leader).is_err());
        lobby
            .db
            .lock()
            .unwrap()
            .execute("UPDATE character_quest_rewards SET loyalty=72000", [])
            .unwrap();
        let packet = start(&lobby, "Chief", &mut leader).unwrap();
        assert_eq!(&packet[..4], &[0x27, 3, 54, 0]);
        assert_eq!((leader.position.x, leader.position.z), (1500, 1500));
        let destination = lobby.social.lock().unwrap().transfers[&member.player_id];
        assert_eq!(destination.zone, 54);
        let db = lobby.db.lock().unwrap();
        for account in ["Chief", "Member"] {
            assert_eq!(crate::movement::load(&db, account, 0).unwrap().zone, 54);
        }
        assert_eq!(crate::starter::coins(&db, "Chief", 0).unwrap(), 20000000);
    }
    #[test]
    fn promotion_consumes_key_and_fee_once_and_rolls_back_everything_on_failure() {
        let (lobby, leader, member) = fixture();
        let db = lobby.db.lock().unwrap();
        assert!(promote(&db, "Member", &member).is_err());
        db.execute(
            "UPDATE character_wallets SET coins=9999999 WHERE account='Chief'",
            [],
        )
        .unwrap();
        assert!(promote(&db, "Chief", &leader).is_err());
        assert_eq!(
            crate::starter::inventory(&db, "Chief", 0).unwrap()[0].id,
            KEY
        );
        db.execute(
            "UPDATE character_wallets SET coins=20000000 WHERE account='Chief'",
            [],
        )
        .unwrap();
        db.execute_batch("CREATE TRIGGER fail_trial BEFORE INSERT ON clan_progress BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(promote(&db, "Chief", &leader).is_err());
        assert_eq!(crate::starter::coins(&db, "Chief", 0).unwrap(), 20000000);
        assert_eq!(
            crate::starter::inventory(&db, "Chief", 0).unwrap()[0].id,
            KEY
        );
        db.execute_batch("DROP TRIGGER fail_trial").unwrap();
        let info = promote(&db, "Chief", &leader).unwrap();
        assert_eq!((info.rank, info.cape, info.grade), (2, 0, 3));
        assert_eq!(crate::starter::coins(&db, "Chief", 0).unwrap(), 10000000);
        assert_eq!(crate::starter::inventory(&db, "Chief", 0).unwrap()[0].id, 0);
        assert!(promote(&db, "Chief", &leader).is_err());
        assert_eq!(clans::load(&db, "Member", 0).unwrap().cape, 0);
        assert_eq!(crate::starter::coins(&db, "Chief", 0).unwrap(), 10000000);
    }
    #[test]
    fn original_menu_ids_cape_purchase_and_stale_dialog_checks() {
        let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
            return;
        };
        {
            let db = lobby.db.lock().unwrap();
            db.execute_batch(
                "INSERT INTO clans VALUES (1,'CapeClan',1);
                INSERT INTO clan_members VALUES ('test',0,1,1);
                UPDATE character_quest_rewards SET loyalty=144000;
                UPDATE character_wallets SET coins=100000000;
                INSERT INTO clan_progress VALUES (1,2,0);",
            )
            .unwrap();
            crate::population::tests::seed(&db);
            db.execute("UPDATE world_spawns SET monster=0,prototype=26031,appearance=22000,kind=25,zone=?1,x0=?2,x1=?2,z0=?3,z1=?3",params![s.position.zone,s.position.x,s.position.z]).unwrap();
            lobby.merchants = crate::shops::load_population(&db, &reko_common::game_dir()).unwrap();
        }
        s.merchant = Some(12000);
        assert_eq!(
            purchase(&lobby, "test", &mut s, &[0x70, 3, 0]).unwrap_err(),
            -6
        );
        assert_eq!(
            purchase(&lobby, "test", &mut s, &[0x70, 99, 0]).unwrap_err(),
            -5
        );
        let packets = purchase(&lobby, "test", &mut s, &[0x70, 1, 0]).unwrap();
        assert_eq!(packets[0], [0x70, 1, 0, 0, 0, 1, 0, 1, 0]);
        assert_eq!(s.coins, 80000000);
        purchase(&lobby, "test", &mut s, &[0x70, 1, 0]).unwrap();
        assert_eq!(s.coins, 80000000);
        assert_eq!(
            clans::load(&lobby.db.lock().unwrap(), "test", 0)
                .unwrap()
                .cape,
            1
        );
        s.position.x += 1000;
        assert_eq!(
            purchase(&lobby, "test", &mut s, &[0x70, 4, 0]).unwrap_err(),
            -1
        );
        let game = reko_common::game_dir();
        let talks = reko_assets::Table::from_encrypted_bytes(
            &std::fs::read(game.join("Data/Quest_Talk_us.tbl")).unwrap(),
        )
        .unwrap();
        let buttons = reko_assets::Table::from_encrypted_bytes(
            &std::fs::read(game.join("Data/Quest_Menu_us.tbl")).unwrap(),
        )
        .unwrap();
        for id in [4072, 4150, 4165, 4168, 4172, 4265, 4268, 4272] {
            assert!(talks.rows.iter().any(|r| combat::integer(r, 0) == id));
        }
        for id in [4071, 4072, 4150, 4157, 4158, 4159, 4160, 4161, 4162] {
            assert!(buttons.rows.iter().any(|r| combat::integer(r, 0) == id));
        }
        let p = menu(
            &mut s,
            12000,
            21610,
            4165,
            &[(4157, 2), (4158, 3), (4159, 4)],
        );
        assert_eq!(i32::from_le_bytes(p[8..12].try_into().unwrap()), 4165);
        assert_eq!(p[60] as usize, SCRIPT.len());
        assert!(process(&lobby, "test", &mut s, &[0x55, 1]).is_err());
    }
}
