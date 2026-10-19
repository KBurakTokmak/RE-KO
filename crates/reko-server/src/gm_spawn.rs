//! Admin monster discovery and one-life summons using imported1453 prototypes.
use crate::{admin, chat, lobby::Lobby, monsters::World, population::Spawn, world_entry::Session};
use rusqlite::Connection;
use std::{collections::HashSet, net::SocketAddr};
use tokio::net::TcpStream;

#[derive(serde::Deserialize)]
struct Template {
    prototype: u16,
    appearance: u16,
    name: String,
    kind: u8,
    group: u32,
    nation: u8,
    level: u8,
    size: u16,
    weapons: [u32; 2],
    hp: u32,
    armor: u16,
    resistance: [u16; 6],
}

fn execute(
    db: &Connection,
    world: &World,
    account: &str,
    s: &Session,
    command: &str,
    args: &str,
) -> anyhow::Result<Vec<String>> {
    if !admin::is_admin(db, account, s.character.slot)? {
        return Ok(vec!["Monster commands require an admin character.".into()]);
    }
    anyhow::ensure!(
        s.entered && !s.zone_loading && s.skills.hp > 0,
        "Enter the world alive before using monster commands."
    );
    if command == "/bossstatus" {
        let mut status = world.boss_status(s.position.zone, args.trim_matches('"'));
        if status.is_empty() {
            status.push("No matching natural boss pools in this zone.".into());
        }
        return Ok(status);
    }
    if args.is_empty() {
        return Ok(vec![
            "Usage: spawn <ID or name>; mobfind [page] <name>; bossstatus [name].".into(),
        ]);
    }
    let (page, query) = if command == "/mobfind" {
        match args.split_once(' ') {
            Some((n, rest)) if n.bytes().all(|b| b.is_ascii_digit()) => {
                let page = n.parse::<usize>()?;
                anyhow::ensure!(page > 0, "Page numbers start at 1.");
                (page, rest.trim())
            }
            _ => (1, args),
        }
    } else {
        (1, args)
    };
    let query = query.trim_matches('"');
    let mut stmt = db.prepare("SELECT rule_id,json_extract(data,'$.name'),data FROM world_reference_rules WHERE table_name='MONSTER_TEMPLATE' ORDER BY rule_id")?;
    let catalog = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, u16>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(!catalog.is_empty(), "Monster catalog is not imported.");
    let numeric = if command != "/mobfind" {
        query.parse::<u16>().ok()
    } else {
        None
    };
    let exact: Vec<_> = catalog
        .iter()
        .filter(|(id, name, _)| {
            numeric.map_or_else(|| name.eq_ignore_ascii_case(query), |n| *id == n)
        })
        .collect();
    let matches = if command != "/mobfind" && (!exact.is_empty() || numeric.is_some()) {
        exact
    } else {
        let lower = query.to_ascii_lowercase();
        catalog
            .iter()
            .filter(|(_, name, _)| name.to_ascii_lowercase().contains(&lower))
            .collect()
    };
    anyhow::ensure!(!matches.is_empty(), "No monster matches '{query}'.");
    if command == "/mobfind" || matches.len() != 1 {
        let pages = matches.len().div_ceil(12);
        anyhow::ensure!(page <= pages, "There are only {pages} result pages.");
        let mut messages = vec![format!(
            "{} matches (page {page}/{pages}). Choose spawn <ID>.",
            matches.len()
        )];
        messages.extend(
            matches
                .iter()
                .skip((page - 1) * 12)
                .take(12)
                .map(|(id, name, _)| format!("{id}: {name}")),
        );
        if page < pages {
            messages.push(if command == "/mobfind" {
                format!("Next: mobfind {} {query}", page + 1)
            } else {
                format!("More results: mobfind {query}")
            });
        }
        return Ok(messages);
    }
    let (id, _, data) = matches[0];
    let t: Template = serde_json::from_str(data)?;
    anyhow::ensure!(t.prototype == *id, "Monster catalog ID mismatch.");
    let ai: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM world_monster_ai WHERE prototype=?1)",
        [id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(ai, "This monster's AI data is not imported.");
    let mut reserved: HashSet<u16> = db
        .prepare("SELECT id FROM world_spawns UNION SELECT id FROM world_merchants")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    reserved.insert(s.player_id);
    let definition = Spawn {
        id: 0,
        monster: true,
        position: s.position,
        prototype: t.prototype,
        appearance: t.appearance,
        name: t.name.clone(),
        kind: t.kind,
        group: t.group,
        nation: t.nation,
        level: t.level,
        size: t.size,
        weapons: t.weapons,
        direction: 0,
        bounds: [s.position.x, s.position.z, s.position.x, s.position.z],
        hp: t.hp,
        armor: t.armor,
        resistance: t.resistance,
        respawn_ms: 1000,
        regen_type: 1,
        move_type: 1,
    };
    let entity = world.summon(definition, &reserved)?;
    tracing::info!(account, character=%s.character.name, prototype=id, entity,
        zone=s.position.zone, x=s.position.x, z=s.position.z, "admin monster summoned");
    Ok(vec![format!(
        "Spawned {} (ID {}, entity {entity}); no respawn.",
        t.name, t.prototype
    )])
}

fn parse(text: &str) -> Option<(&'static str, &str)> {
    let text = text.trim();
    let (token, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let token = token
        .strip_prefix('/')
        .or_else(|| token.strip_prefix('+'))
        .unwrap_or(token);
    let command = match token.to_ascii_lowercase().as_str() {
        "spawn" | "monsummon" => "/spawn",
        "mobfind" => "/mobfind",
        "bossstatus" => "/bossstatus",
        _ => return None,
    };
    Some((command, args.trim()))
}

pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &Session,
    text: &[u8],
) -> bool {
    let text: String = text.iter().map(|b| char::from(*b)).collect();
    let Some((command, args)) = parse(&text) else {
        return false;
    };
    let result = match lobby.monsters.as_ref() {
        Some(world) => execute(&lobby.db.lock().unwrap(), world, account, s, command, args),
        None => Err(anyhow::anyhow!("Monster world unavailable.")),
    };
    let messages = result.unwrap_or_else(|e| vec![format!("Monster command: {e}")]);
    for message in messages {
        let bytes: Vec<u8> = message.chars().take(240).map(|c| c as u8).collect();
        let packet = chat::packet(7, s.character.nation(), u16::MAX, "", &bytes);
        super::send_response(sock, peer, &None, &packet, "ADMIN_MONSTER_NOTICE").await;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{characters, combat, loot, monsters::Attack, population, skills};
    use rusqlite::params;

    #[test]
    fn captured_bare_commands_and_prefixed_aliases_parse_identically() {
        for text in ["spawn 803", "/spawn 803", "+monsummon 803", " SpAwN 803 "] {
            assert_eq!(parse(text), Some(("/spawn", "803")));
        }
        for text in ["mobfind worm", "/mobfind worm", "+mobfind worm"] {
            assert_eq!(parse(text), Some(("/mobfind", "worm")));
        }
        assert_eq!(
            parse("spawn Rabid Bandicoot"),
            Some(("/spawn", "Rabid Bandicoot"))
        );
        assert_eq!(parse("mobfind 2 harpy"), Some(("/mobfind", "2 harpy")));
        for text in ["bossstatus Talos", "/bossstatus Talos", "+bossstatus Talos"] {
            assert_eq!(parse(text), Some(("/bossstatus", "Talos")));
        }
        assert_eq!(parse("bossstatus"), Some(("/bossstatus", "")));
        for text in ["respawn 803", "spawned 803", "normal chat"] {
            assert_eq!(parse(text), None);
        }
    }

    #[tokio::test]
    async fn captured_chat_packet_routes_bare_spawn_to_world_and_private_notice() {
        use tokio::{io::AsyncReadExt, net::TcpListener};
        let Some((mut lobby, s)) = crate::world_service_tests::setup() else {
            return;
        };
        let world = std::sync::Arc::new(World::load(&reko_common::game_dir()).unwrap());
        lobby.monsters = Some(world.clone());
        let mut events = world.events.subscribe();
        {
            let db = lobby.db.lock().unwrap();
            db.execute("INSERT INTO character_privileges VALUES ('test',0,1)", [])
                .unwrap();
            let data = serde_json::json!({"prototype":803,"appearance":800,"name":"Rabid Bandicoot",
                "kind":0,"group":0,"nation":0,"level":9,"size":120,"weapons":[0,0],"hp":57,"armor":46,"resistance":[22,22,22,15,22,22]});
            db.execute(
                "INSERT INTO world_reference_rules VALUES ('MONSTER_TEMPLATE',803,?1)",
                [data.to_string()],
            )
            .unwrap();
            db.execute("INSERT INTO world_monster_ai VALUES (803,0,1,1,1000,1,2,2,10,20,100,0,0,0,0,0,1000)", []).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        let mut account = Some("test".to_string());
        let mut session = Some(s);
        // Exact payload captured from the owner's failed attempt.
        let p = b"\x10\x01\x09\x00spawn 803";
        crate::handle_game_payload(&mut socket, peer, p, &lobby, &mut account, &mut session).await;
        let spawn = events.try_recv().unwrap();
        assert_eq!(&spawn.packet[..2], &[0x0a, 1]);
        assert_eq!(u16::from_le_bytes([spawn.packet[4], spawn.packet[5]]), 803);
        let mut header = [0; 4];
        client.read_exact(&mut header).await.unwrap();
        let length = usize::from(u16::from_le_bytes([header[2], header[3]]));
        let mut packet = vec![0; length];
        client.read_exact(&mut packet).await.unwrap();
        assert_eq!(&packet[..2], &[0x10, 7]);
        assert!(
            packet
                .windows(b"Spawned Rabid Bandicoot".len())
                .any(|w| w == b"Spawned Rabid Bandicoot")
        );
    }

    #[test]
    fn names_ids_permissions_paging_and_one_life_rewards_use_the_normal_world() {
        let game = reko_common::game_dir();
        let rules = std::path::Path::new("../../logs/reference-coefficients.csv");
        if !game.join("Data/NPC_Looks.tbl").exists() || !rules.exists() {
            return;
        }
        let db = Connection::open_in_memory().unwrap();
        characters::initialize(&db).unwrap();
        admin::initialize(&db).unwrap();
        crate::shops::initialize(&db).unwrap();
        loot::initialize(&db).unwrap();
        db.execute_batch(include_str!("npc_ai.sql")).unwrap();
        population::tests::seed(&db);
        db.execute("UPDATE world_spawns SET id=30000", []).unwrap();
        characters::ensure_account(&db, "gm").unwrap();
        db.execute(
            "INSERT INTO lobby_characters VALUES ('gm',0,'Summoner',2,108,0,0,?1)",
            [[60u8; 5].as_slice()],
        )
        .unwrap();
        let mut s =
            Session::new(characters::find(&db, "gm", "Summoner").unwrap().unwrap()).unwrap();
        s.entered = true;
        for id in 100..114 {
            let name = if id == 113 {
                "Unique Beast"
            } else {
                "Duplicate Beast"
            };
            let data = serde_json::json!({"prototype":id,"appearance":100,"name":name,
                "kind":0,"group":0,"nation":0,"level":80,"size":150,
                "weapons":[0,0],"hp":100,"armor":500,"resistance":[10,20,30,40,50,60]});
            db.execute(
                "INSERT INTO world_reference_rules VALUES ('MONSTER_TEMPLATE',?1,?2)",
                params![id, data.to_string()],
            )
            .unwrap();
            db.execute("INSERT INTO world_monster_ai VALUES (?1,0,1,1,1000,1,2,2,10,20,100,0,0,0,0,0,1000)", [id]).unwrap();
        }
        db.execute("INSERT INTO world_monster_loot VALUES (100,1000,0)", [])
            .unwrap();
        let catalog = combat::Catalog::load(&game, rules).unwrap();
        let mut world = World::load(&game).unwrap();
        world.loot = Some(std::sync::Arc::new(
            loot::Book::load(&db, &catalog).unwrap(),
        ));
        let mut events = world.events.subscribe();
        let initial = world.spawns(s.position.zone).len();
        assert!(execute(&db, &world, "gm", &s, "/spawn", "100").unwrap()[0].contains("admin"));
        assert!(execute(&db, &world, "gm", &s, "/bossstatus", "").unwrap()[0].contains("admin"));
        assert_eq!(world.spawns(s.position.zone).len(), initial);
        db.execute("INSERT INTO character_privileges VALUES ('gm',0,1)", [])
            .unwrap();
        let choices = execute(&db, &world, "gm", &s, "/spawn", "Duplicate Beast").unwrap();
        assert!(choices[0].starts_with("13 matches"));
        assert!(choices.last().unwrap().contains("mobfind"));
        assert_eq!(world.spawns(s.position.zone).len(), initial);
        let page = execute(&db, &world, "gm", &s, "/mobfind", "2 duplicate").unwrap();
        assert_eq!(page.len(), 2);
        assert!(page[1].starts_with("112:"));
        assert!(execute(&db, &world, "gm", &s, "/mobfind", "999 duplicate").is_err());
        assert!(execute(&db, &world, "gm", &s, "/spawn", "65535").is_err());
        execute(&db, &world, "gm", &s, "/spawn", "100").unwrap();
        let spawn = events.try_recv().unwrap();
        let entity = u16::from_le_bytes([spawn.packet[2], spawn.packet[3]]);
        assert_eq!(
            entity, 30001,
            "reserved persistent NPC IDs cannot be reused"
        );
        assert_eq!(&spawn.packet[..2], &[0x0a, 1]);
        assert_eq!(u16::from_le_bytes([spawn.packet[4], spawn.packet[5]]), 100);
        assert_eq!(world.target_attributes(entity), (100, 500));
        let result = world
            .hit_owned(
                entity,
                Attack {
                    position: s.position,
                    range: 5.0,
                    attack: 0,
                    percent: 100,
                    extra: 1000,
                    skill: true,
                    penetrate: false,
                    drain_percent: 0,
                    magic: None,
                    arrow: false,
                },
                Some("Summoner"),
                || Ok(()),
            )
            .unwrap();
        assert!(result.dead);
        assert_eq!(world.take_experience("Summoner")[0].prototype, 100);
        assert_eq!(world.take_quest_kills("Summoner"), [100]);
        let mut loot_seen = false;
        while let Ok(event) = events.try_recv() {
            loot_seen |= event.packet[0] == 0x23;
        }
        assert!(
            loot_seen,
            "summoned monster uses the existing corpse-loot path"
        );
        world.tick(skills::now_ms() + 1_000_000);
        assert!(
            world.validate(entity, s.position, 5.0).is_err(),
            "summons must not respawn"
        );
        execute(&db, &world, "gm", &s, "+monsummon", "unique beast").unwrap();
        assert_eq!(world.spawns(s.position.zone).len(), initial + 1);
        db.execute("UPDATE world_reference_rules SET data=json_set(data,'$.appearance',65535) WHERE table_name='MONSTER_TEMPLATE' AND rule_id=113", []).unwrap();
        let error = execute(&db, &world, "gm", &s, "/spawn", "113").unwrap_err();
        assert!(error.to_string().contains("model"));
        assert_eq!(world.spawns(s.position.zone).len(), initial + 1);
        db.execute("UPDATE character_privileges SET is_admin=0", [])
            .unwrap();
        assert!(execute(&db, &world, "gm", &s, "/spawn", "113").unwrap()[0].contains("admin"));
        assert_eq!(world.spawns(s.position.zone).len(), initial + 1);
    }

    #[test]
    fn felankor_summons_with_the_original_complete_character_assets() {
        let Some((lobby, s)) = crate::world_service_tests::setup() else {
            return;
        };
        let game = reko_common::game_dir();
        let world = World::load(&game).unwrap();
        let mut events = world.events.subscribe();
        let db = lobby.db.lock().unwrap();
        let data = serde_json::json!({"prototype":7035,"appearance":6200,"name":"Felankor","kind":0,"group":0,"nation":0,"level":110,"size":120,"hp":1981241,"armor":0,"weapons":[0,0],"resistance":[500,500,500,500,500,500]});
        db.execute(
            "INSERT INTO world_reference_rules VALUES ('MONSTER_TEMPLATE',7035,?1)",
            [data.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO world_monster_ai VALUES (7035,0,1,1,1000,1,2,2,10,20,100,0,0,0,0,0,1000)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO character_privileges VALUES ('test',0,1)", [])
            .unwrap();
        let messages = execute(&db, &world, "test", &s, "/spawn", "Felankor").unwrap();
        assert!(messages[0].starts_with("Spawned Felankor"));
        let event = events.try_recv().unwrap();
        assert_eq!(&event.packet[..2], &[0x0a, 1]);
        assert_eq!(
            u16::from_le_bytes(event.packet[6..8].try_into().unwrap()),
            6200
        );
    }
}
