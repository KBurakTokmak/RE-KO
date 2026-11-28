//! Applies the scheduled-event controller to the live world: notices, zone
//! eviction, temple waves, war NPCs, the siege artifact, and persisted rewards.
//!
//! Rewards are written to `character_event_rewards` in one transaction and then
//! delivered by each player's own session tick. A full bag or weight limit keeps
//! the reward queued (also across relogs and restarts) until space is free.
use crate::{
    events::{self, Action, Controller, Exit, Gates},
    lobby::Lobby,
    monsters::{EventTag, Restriction},
    movement::Position,
    starter,
    world_entry::Session,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Keepers and warders whose deaths decide a nation war in zones 61-63.
const WAR_NPCS: [u16; 6] = [11041, 21041, 11042, 21042, 11047, 21045];
/// Nereid monuments of zone 64.
const MONUMENTS: [u16; 3] = [24527, 24528, 24529];
const ARTIFACT: u16 = 541;
const REWARD_RETRY: Duration = Duration::from_secs(5);
const FULL_RETRY: Duration = Duration::from_secs(60);

/// Event state shared through the social hub (no extra lock order).
#[derive(Default)]
pub(crate) struct Shared {
    pub gates: Gates,
    /// Player deaths in the active war zone (Karus, El Morad).
    pub deaths: [u32; 2],
    /// Lowercase character names with queued rewards.
    pub pending: HashSet<String>,
    next_attempt: HashMap<String, Instant>,
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS character_event_rewards (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        item_id INTEGER NOT NULL, count INTEGER NOT NULL CHECK(count > 0),
        source TEXT NOT NULL, created TEXT NOT NULL,
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE);
        CREATE INDEX IF NOT EXISTS character_event_rewards_owner ON character_event_rewards(account,slot);
        CREATE TABLE IF NOT EXISTS castle_siege_owner (
        castle INTEGER PRIMARY KEY CHECK(castle = 1),
        clan_id INTEGER NOT NULL, updated TEXT NOT NULL);",
    )
}

/// Loads schedule, waves and the Delos owner. Running events never resume.
pub(crate) fn load(db: &Connection) -> anyhow::Result<Controller> {
    let mut controller = Controller::default();
    let mut q = db.prepare(
        "SELECT rule_id,data FROM world_reference_rules WHERE table_name='EVENT_SCHEDULE' ORDER BY rule_id",
    )?;
    for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? {
        let (id, data) = row?;
        match events::parse_entry(id, &data) {
            Ok(Some(entry)) => controller.schedule.push(entry),
            Ok(None) => {}
            Err(error) => tracing::error!(id, %error, "invalid event schedule row ignored"),
        }
    }
    let mut q = db.prepare(
        "SELECT rule_id,data FROM world_reference_rules WHERE table_name='MONSTER_CHALLENGE_SUMMON_LIST'",
    )?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    controller.waves = events::waves_from_rows(
        rows.into_iter()
            .filter_map(|(id, data)| serde_json::from_str(&data).ok().map(|v| (id, v))),
    );
    controller.castle_owner = castle_owner(db)?;
    tracing::info!(
        scheduled = controller.schedule.len(),
        temple_brackets = controller.waves.len(),
        castle_owner = ?controller.castle_owner,
        "event controller loaded"
    );
    Ok(controller)
}

pub(crate) fn castle_owner(db: &Connection) -> rusqlite::Result<Option<(u16, String)>> {
    db.query_row(
        "SELECT c.id,c.name FROM castle_siege_owner o JOIN clans c ON c.id=o.clan_id WHERE o.castle=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

pub(crate) fn pending_names(db: &Connection) -> rusqlite::Result<HashSet<String>> {
    db.prepare(
        "SELECT DISTINCT lower(c.name) FROM character_event_rewards r
         JOIN lobby_characters c ON c.account=r.account AND c.slot=r.slot",
    )?
    .query_map([], |r| r.get(0))?
    .collect()
}

pub(crate) async fn run(lobby: std::sync::Arc<Lobby>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        tick(&lobby, chrono::Local::now().naive_local());
    }
}

/// Feeds world input to the controller and applies its actions.
pub(crate) fn tick(lobby: &Lobby, now: chrono::NaiveDateTime) {
    let (deaths, alive) = lobby.monsters.as_ref().map_or((Vec::new(), 0), |w| {
        (w.take_event_deaths(), w.event_alive(EventTag::Temple))
    });
    let (pvp_deaths, killers) = {
        let mut hub = lobby.social.lock().unwrap();
        let killers: Vec<_> = deaths
            .iter()
            .map(|d| {
                d.killer
                    .as_deref()
                    .and_then(|name| hub.named(name))
                    .map(|id| &hub.players[&id].s)
                    .map(|s| (s.character.name.clone(), s.clan.id, s.clan.name.clone()))
            })
            .collect();
        (std::mem::take(&mut hub.events.deaths), killers)
    };
    // Killers who went offline keep their credit through the attacker's clan.
    let killers: Vec<(String, u16, String)> = deaths
        .iter()
        .zip(killers)
        .map(|(death, killer)| match killer {
            Some(killer) if killer.1 != 0 => killer,
            killer => {
                let name = killer
                    .map(|k| k.0)
                    .or_else(|| death.killer.clone())
                    .unwrap_or_default();
                let clan = death.attacker.map_or(0, |(_, clan)| clan);
                let clan_name = if clan == 0 {
                    String::new()
                } else {
                    lobby
                        .db
                        .lock()
                        .unwrap()
                        .query_row("SELECT name FROM clans WHERE id=?1", [clan], |r| r.get(0))
                        .unwrap_or_default()
                };
                (name, clan, clan_name)
            }
        })
        .collect();
    let (actions, gates, log) = {
        let mut c = lobby.events.lock().unwrap();
        c.set_temple_alive(alive);
        c.add_deaths(pvp_deaths[0], pvp_deaths[1]);
        for (death, (name, clan_id, clan)) in deaths.iter().zip(killers) {
            match death.tag {
                EventTag::War => match death.captured_by {
                    Some(nation) => c.monument_captured(death.id, nation, "a Nereid Monument"),
                    None => c.war_npc_killed(death.nation, death.prototype, &name, &clan, now),
                },
                EventTag::Siege if death.prototype == ARTIFACT => {
                    c.artifact_destroyed(clan_id, &clan)
                }
                _ => {}
            }
        }
        let mut actions = c.take_actions();
        actions.extend(c.tick(now));
        (actions, c.gates(), c.take_log())
    };
    for line in log {
        tracing::warn!("{line}");
    }
    lobby.social.lock().unwrap().events.gates = gates;
    apply(lobby, actions);
}

/// Runs an operator command and applies its actions immediately.
pub(crate) fn command(lobby: &Lobby, text: &str) -> Vec<String> {
    let now = chrono::Local::now().naive_local();
    let (lines, actions, gates) = {
        let mut c = lobby.events.lock().unwrap();
        let lines = c.command(text, now);
        (lines, c.take_actions(), c.gates())
    };
    lobby.social.lock().unwrap().events.gates = gates;
    apply(lobby, actions);
    lines
}

pub(crate) fn apply(lobby: &Lobby, actions: Vec<Action>) {
    for action in actions {
        let result = match action.clone() {
            Action::Notice { zone, nation, text } => {
                notice(lobby, zone, nation, &text);
                Ok(())
            }
            Action::SpawnWave(wave) => spawn_wave(lobby, wave),
            Action::Despawn(tag) => {
                if let Some(world) = &lobby.monsters {
                    world.despawn_event(tag);
                }
                Ok(())
            }
            Action::RewardZone {
                zone,
                items,
                source,
            } => reward(lobby, &items, source, |s| {
                s.position.zone == zone && s.skills.hp > 0
            }),
            Action::RewardNation {
                nation,
                zones,
                items,
                source,
            } => reward(lobby, &items, source, |s| {
                s.character.nation() == nation && zones.contains(&s.position.zone)
            }),
            Action::ArmWar { zone } => arm_war(lobby, zone),
            Action::SpawnArtifact { owner } => spawn_artifact(lobby, owner),
            Action::CastleOwner { clan } => set_castle_owner(lobby, clan),
            Action::Invasion { zone, open } => {
                set_invasion(lobby, zone, open);
                Ok(())
            }
        };
        if let Err(error) = result {
            tracing::error!(?action, %error, "event action failed");
        }
    }
}

fn set_invasion(lobby: &Lobby, zone: u8, open: bool) {
    tracing::info!(zone, open, "homeland invasion state");
    if let Some(world) = &lobby.monsters {
        world.set_guard_invaded(zone, open);
    }
}

pub(crate) fn notice(lobby: &Lobby, zone: Option<u8>, nation: Option<u8>, text: &str) {
    let bytes: Vec<u8> = text.chars().take(240).map(|c| c as u8).collect();
    let packet = crate::operator_chat::notice(&bytes);
    let mut hub = lobby.social.lock().unwrap();
    let ids: Vec<u16> = hub
        .players
        .iter()
        .filter(|(_, p)| {
            p.s.entered
                && zone.is_none_or(|z| p.s.position.zone == z)
                && nation.is_none_or(|n| p.s.character.nation() == n)
        })
        .map(|(&id, _)| id)
        .collect();
    hub.send_many(&ids, &packet);
}

fn random(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

/// Scatters a temple wave within its radius of the summon point.
fn spawn_wave(lobby: &Lobby, wave: events::Wave) -> anyhow::Result<()> {
    let world = lobby
        .monsters
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("monster world unavailable"))?;
    let center = Position {
        zone: events::TEMPLE_ZONE,
        x: wave.x.saturating_mul(10),
        z: wave.z.saturating_mul(10),
        y: 0,
    };
    let (template, reserved) = {
        let db = lobby.db.lock().unwrap();
        (
            crate::gm_spawn::template_spawn(&db, wave.sid, center)?,
            crate::gm_spawn::reserved_ids(&db)?,
        )
    };
    let mut seed = crate::skills::now_ms() ^ u64::from(wave.sid) << 20;
    let radius = i64::from(wave.range) * 10;
    let mut spawned = 0;
    for _ in 0..wave.count {
        let mut s = template.clone();
        let dx = (random(&mut seed) % (2 * radius as u64 + 1)) as i64 - radius;
        let dz = (random(&mut seed) % (2 * radius as u64 + 1)) as i64 - radius;
        s.position.x = (i64::from(center.x) + dx).clamp(0, 65535) as u16;
        s.position.z = (i64::from(center.z) + dz).clamp(0, 65535) as u16;
        s.bounds = [s.position.x, s.position.z, s.position.x, s.position.z];
        world.summon_with(s, &reserved, Some(EventTag::Temple), None, true)?;
        spawned += 1;
    }
    tracing::info!(
        stage = wave.stage,
        prototype = wave.sid,
        spawned,
        "temple wave spawned"
    );
    Ok(())
}

/// Queues rewards for every matching online player in one transaction.
fn reward(
    lobby: &Lobby,
    items: &[(u32, u32)],
    source: &str,
    eligible: impl Fn(&Session) -> bool,
) -> anyhow::Result<()> {
    let recipients: Vec<(String, u8, String)> = {
        let hub = lobby.social.lock().unwrap();
        hub.players
            .values()
            .filter(|p| eligible(&p.s))
            .map(|p| {
                (
                    p.account.clone(),
                    p.s.character.slot,
                    p.s.character.name.clone(),
                )
            })
            .collect()
    };
    if recipients.is_empty() {
        return Ok(());
    }
    {
        let db = lobby.db.lock().unwrap();
        queue_rewards(&db, &recipients, items, source)?;
    }
    let mut hub = lobby.social.lock().unwrap();
    for (_, _, name) in &recipients {
        let name = name.to_ascii_lowercase();
        hub.events.next_attempt.remove(&name);
        hub.events.pending.insert(name);
    }
    tracing::info!(
        source,
        recipients = recipients.len(),
        ?items,
        "event rewards queued"
    );
    Ok(())
}

pub(crate) fn queue_rewards(
    db: &Connection,
    recipients: &[(String, u8, String)],
    items: &[(u32, u32)],
    source: &str,
) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    let created = chrono::Local::now().to_rfc3339();
    for (account, slot, _) in recipients {
        for (item, count) in items {
            tx.execute(
                "INSERT INTO character_event_rewards(account,slot,item_id,count,source,created) VALUES(?1,?2,?3,?4,?5,?6)",
                params![account, slot, item, count, source, created],
            )?;
        }
    }
    tx.commit()
}

/// Builds the combat entities of the war zone from its static NPCs.
fn arm_war(lobby: &Lobby, zone: u8) -> anyhow::Result<()> {
    let world = lobby
        .monsters
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("monster world unavailable"))?;
    let monuments = zone == 64;
    let wanted: &[u16] = if monuments { &MONUMENTS } else { &WAR_NPCS };
    let positions: HashMap<u16, Position> = lobby
        .merchants
        .iter()
        .filter(|m| m.position.zone == zone && wanted.contains(&m.prototype))
        .map(|m| (m.id, m.position))
        .collect();
    let mut spawns = Vec::new();
    {
        let db = lobby.db.lock().unwrap();
        let mut q = db.prepare("SELECT id,prototype,appearance,name,kind,selling_group,nation,level,size,weapon1,weapon2,direction,hp,armor,fire_r,cold_r,lightning_r,magic_r,disease_r,poison_r FROM world_spawns WHERE id=?1")?;
        for (&id, &position) in &positions {
            let spawn = q.query_row([id], |r| {
                Ok(crate::population::Spawn {
                    id: r.get(0)?,
                    monster: true,
                    position,
                    prototype: r.get(1)?,
                    appearance: r.get(2)?,
                    name: r.get(3)?,
                    kind: r.get(4)?,
                    group: r.get(5)?,
                    nation: r.get(6)?,
                    level: r.get(7)?,
                    size: r.get(8)?,
                    weapons: [r.get(9)?, r.get(10)?],
                    direction: r.get(11)?,
                    bounds: [position.x, position.z, position.x, position.z],
                    hp: r.get(12)?,
                    armor: r.get(13)?,
                    resistance: [
                        r.get(14)?,
                        r.get(15)?,
                        r.get(16)?,
                        r.get(17)?,
                        r.get(18)?,
                        r.get(19)?,
                    ],
                    respawn_ms: 1000,
                    regen_type: 1,
                    move_type: 0,
                })
            })?;
            spawns.push(spawn);
        }
    }
    if !monuments {
        // Known 1453 behavior: war NPCs fight with half HP and a fifth of their armor.
        for s in &mut spawns {
            s.hp = (s.hp / 2).max(1);
            s.armor /= 5;
        }
    }
    let armed = world.arm(spawns, EventTag::War, monuments);
    tracing::info!(zone, armed = armed.len(), "war entities armed");
    if monuments {
        lobby.events.lock().unwrap().set_monuments(&armed);
    }
    Ok(())
}

/// Places the Center Artifact on its original Delos object position.
fn spawn_artifact(lobby: &Lobby, owner: u16) -> anyhow::Result<()> {
    let world = lobby
        .monsters
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("monster world unavailable"))?;
    let (npc, object, reserved) = {
        let db = lobby.db.lock().unwrap();
        let npc: String = db.query_row(
            "SELECT data FROM world_reference_rules WHERE table_name='OBJECT_NPC' AND rule_id=?1",
            [ARTIFACT],
            |r| r.get(0),
        )?;
        let object: String = db.query_row(
            "SELECT data FROM world_reference_rules WHERE table_name='K_OBJECTPOS' AND rule_id=?1",
            [u32::from(events::DELOS) * 100000 + u32::from(ARTIFACT)],
            |r| r.get(0),
        )?;
        (
            serde_json::from_str::<serde_json::Value>(&npc)?,
            serde_json::from_str::<serde_json::Value>(&object)?,
            crate::gm_spawn::reserved_ids(&db)?,
        )
    };
    let n = |v: &serde_json::Value, key: &str| -> f64 {
        v[key]
            .as_str()
            .and_then(|s| s.trim().parse().ok())
            .or_else(|| v[key].as_f64())
            .unwrap_or(0.0)
    };
    let position = Position {
        zone: events::DELOS,
        x: (n(&object, "PosX") * 10.0) as u16,
        z: (n(&object, "PosZ") * 10.0) as u16,
        y: (n(&object, "PosY") * 10.0) as i16,
    };
    let resistance = [
        "sFireR",
        "sColdR",
        "sLightningR",
        "sMagicR",
        "sDiseaseR",
        "sPoisonR",
    ]
    .map(|k| n(&npc, k).clamp(0.0, 65535.0) as u16);
    let spawn = crate::population::Spawn {
        id: 0,
        monster: false,
        position,
        prototype: ARTIFACT,
        appearance: n(&npc, "sPid") as u16,
        name: npc["strName"]
            .as_str()
            .unwrap_or("Center Artifact")
            .trim()
            .to_owned(),
        kind: n(&npc, "byType") as u8,
        group: 0,
        nation: 3,
        level: n(&npc, "sLevel") as u8,
        size: (n(&npc, "sSize") as u16).max(1),
        weapons: [0; 2],
        direction: 0,
        bounds: [position.x, position.z, position.x, position.z],
        hp: (n(&npc, "iHpPoint") as u32).max(1),
        armor: n(&npc, "sAc").clamp(0.0, 65535.0) as u16,
        resistance,
        respawn_ms: 1000,
        regen_type: 1,
        move_type: 0,
    };
    // The artifact is a Delos map object; NPC_Looks has no model for it.
    let id = world.summon_with(
        spawn,
        &reserved,
        Some(EventTag::Siege),
        Some(Restriction::Clan { owner }),
        false,
    )?;
    tracing::info!(id, owner, "center artifact placed");
    Ok(())
}

fn set_castle_owner(lobby: &Lobby, clan: u16) -> anyhow::Result<()> {
    lobby.db.lock().unwrap().execute(
        "INSERT INTO castle_siege_owner VALUES(1,?1,?2) ON CONFLICT(castle) DO UPDATE SET clan_id=excluded.clan_id,updated=excluded.updated",
        params![clan, chrono::Local::now().to_rfc3339()],
    )?;
    // Native WIZ_SIEGE owner update (clan ID and mark version) for Delos.
    let mut packet = vec![0x6d, 2];
    packet.extend(clan.to_le_bytes());
    packet.extend(0u16.to_le_bytes());
    let mut hub = lobby.social.lock().unwrap();
    let ids: Vec<u16> = hub
        .players
        .iter()
        .filter(|(_, p)| p.s.entered && p.s.position.zone == events::DELOS)
        .map(|(&id, _)| id)
        .collect();
    hub.send_many(&ids, &packet);
    tracing::info!(clan, "Delos castle owner saved");
    Ok(())
}

pub(crate) fn exit_position(db: &Connection, exit: Exit, nation: u8) -> anyhow::Result<Position> {
    match exit {
        Exit::Home => crate::suppression::home(db, nation),
        Exit::Moradon => {
            let (kx, kz, ex, ez): (u16, u16, u16, u16) = db.query_row(
                "SELECT karus_x,karus_z,elmo_x,elmo_z FROM world_town_starts WHERE zone=?1",
                [events::MORADON],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let (x, z) = if nation == 1 { (kx, kz) } else { (ex, ez) };
            Ok(Position {
                zone: events::MORADON,
                x: x.checked_mul(10)
                    .ok_or_else(|| anyhow::anyhow!("Invalid Moradon X."))?,
                z: z.checked_mul(10)
                    .ok_or_else(|| anyhow::anyhow!("Invalid Moradon Z."))?,
                y: 0,
            })
        }
    }
}

/// Moves a player out of the Forgotten Temple to Moradon. A dead player is
/// revived first; temple deaths cost no experience.
pub(crate) fn leave_temple(
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
) -> anyhow::Result<Vec<u8>> {
    let db = lobby.db.lock().unwrap();
    let destination = exit_position(&db, Exit::Moradon, s.character.nation())?;
    let tx = db.unchecked_transaction()?;
    let mut next = s.skills.clone();
    if next.hp == 0 {
        crate::travel::clear_zone_effects(&mut next, lobby.skills.as_ref());
        let stats =
            crate::utility::effective(s, &next, lobby.skills.as_ref(), lobby.combat.as_ref());
        next.set_limits(stats.hp, stats.mp);
        next.hp = next.max_hp;
        crate::skills::save_in(&tx, account, s.character.slot, &next)?;
    }
    crate::travel::save_destination(
        &tx,
        account,
        s.character.slot,
        s.position.zone,
        destination,
        lobby.skills.as_ref(),
    )?;
    tx.commit()?;
    drop(db);
    s.skills = next;
    Ok(crate::travel::begin(lobby, s, destination))
}

/// Per-session event duties: leave closed event zones and receive rewards.
pub(crate) fn session_tick(
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
) -> anyhow::Result<Vec<Vec<u8>>> {
    if !s.entered || s.zone_loading || s.skills.hp == 0 {
        return Ok(Vec::new());
    }
    let name = s.character.name.to_ascii_lowercase();
    let (exit, deliver_now) = {
        let mut hub = lobby.social.lock().unwrap();
        let exit = hub
            .events
            .gates
            .exit(s.position.zone, s.character.nation(), s.is_admin);
        let now = Instant::now();
        let due = hub.events.pending.contains(&name)
            && hub
                .events
                .next_attempt
                .get(&name)
                .is_none_or(|at| now >= *at);
        if due {
            hub.events
                .next_attempt
                .insert(name.clone(), now + REWARD_RETRY);
        }
        (exit, due)
    };
    if let Some(exit) = exit {
        let destination = {
            let db = lobby.db.lock().unwrap();
            let destination = exit_position(&db, exit, s.character.nation())?;
            crate::travel::save_destination(
                &db,
                account,
                s.character.slot,
                s.position.zone,
                destination,
                lobby.skills.as_ref(),
            )?;
            destination
        };
        tracing::info!(character=%s.character.name, from=s.position.zone, to=destination.zone, "event zone eviction");
        return Ok(vec![crate::travel::begin(lobby, s, destination)]);
    }
    if !deliver_now {
        return Ok(Vec::new());
    }
    let (packets, remaining, blocked) = deliver(lobby, account, s)?;
    let mut hub = lobby.social.lock().unwrap();
    if !remaining {
        hub.events.pending.remove(&name);
        hub.events.next_attempt.remove(&name);
    } else if blocked {
        hub.events
            .next_attempt
            .insert(name, Instant::now() + FULL_RETRY);
    }
    Ok(packets)
}

fn give(
    bag: &mut [starter::Item; 28],
    catalog: &crate::combat::Catalog,
    id: u32,
    mut quantity: u32,
) -> bool {
    let Some((row, Some(ext))) = catalog.item(id) else {
        return false;
    };
    let stack = if crate::combat::integer(row, 23) != 0 {
        9999
    } else {
        1
    };
    let durability =
        (crate::combat::integer(row, 19) + crate::combat::integer(ext, 12)).clamp(0, 65535) as u16;
    for item in bag.iter_mut() {
        if item.id != 0 && (stack == 1 || item.id != id) {
            continue;
        }
        let n = quantity.min(stack - u32::from(item.count));
        if n == 0 {
            continue;
        }
        item.id = id;
        item.durability = durability;
        item.count += n as u16;
        quantity -= n;
        if quantity == 0 {
            return true;
        }
    }
    false
}

type Delivery = (Vec<Vec<u8>>, bool, bool);

/// Delivers queued rewards in order until one does not fit.
/// Capped Noah stays queued while later rewards continue. Returns packets,
/// whether rewards remain, and whether the player must wait for room.
pub(crate) fn deliver(lobby: &Lobby, account: &str, s: &mut Session) -> anyhow::Result<Delivery> {
    let catalog = lobby
        .combat
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("item catalog unavailable"))?;
    let slot = s.character.slot;
    let db = lobby.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let rows = tx
        .prepare("SELECT id,item_id,count,source FROM character_event_rewards WHERE account=?1 AND slot=?2 ORDER BY id")?
        .query_map(params![account, slot], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?, r.get::<_, u32>(2)?, r.get::<_, String>(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok((Vec::new(), false, false));
    }
    let before = starter::inventory(&tx, account, slot)?;
    let mut bag = before;
    let coins_before = starter::coins(&tx, account, slot)?;
    let mut coins = coins_before;
    let mut state = crate::quest_rewards::State::new(&tx, account, s)?;
    let state_before = state.clone();
    let gear = catalog
        .calculate(&s.character, &s.progress, &s.equipment)?
        .weight;
    let mut delivered = Vec::new();
    let mut chests = 0;
    let mut blocked = false;
    let mut capped = false;
    for (id, item, count, source) in &rows {
        if *item != events::NOAH
            && !state.handles(*item)
            && !matches!(catalog.item(*item), Some((_, Some(_))))
        {
            // A wrong owner-configured item must not block later rewards; it
            // stays in the save for repair and is retried after a restart.
            tracing::error!(
                item,
                source,
                "event reward item is absent from the client tables"
            );
            continue;
        }
        let ok = if *item == events::NOAH {
            match coins.checked_add(*count).filter(|c| *c <= 2_100_000_000) {
                Some(total) => {
                    coins = total;
                    true
                }
                None => {
                    // Kept queued until the wallet has room; later rewards continue.
                    capped = true;
                    continue;
                }
            }
        } else if state.gain(*item, *count)? {
            true
        } else {
            let mut trial = bag;
            if give(&mut trial, catalog, *item, *count)
                && u32::from(gear) + u32::from(catalog.bag_weight(&trial))
                    <= u32::from(s.combat.max_weight)
            {
                bag = trial;
                if source == "forgotten-temple" {
                    chests += 1;
                }
                true
            } else {
                false
            }
        };
        if !ok {
            blocked = true;
            break;
        }
        delivered.push(*id);
    }
    for id in &delivered {
        tx.execute("DELETE FROM character_event_rewards WHERE id=?1", [id])?;
    }
    for (index, item) in bag.iter().enumerate() {
        if *item == before[index] {
            continue;
        }
        tx.execute("INSERT INTO character_inventory VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(account,slot,inventory_slot) DO UPDATE SET item_id=excluded.item_id,durability=excluded.durability,count=excluded.count", params![account, slot, index, item.id, item.durability, item.count])?;
    }
    if coins != coins_before {
        tx.execute(
            "UPDATE character_wallets SET coins=?3 WHERE account=?1 AND slot=?2",
            params![account, slot, coins],
        )?;
    }
    let mut next = s.clone();
    if state != state_before {
        state.save(&tx, account, slot)?;
        next.rewards = state.totals;
        next.progress = state.progress.clone();
        next.max_exp = state.max_exp;
        next.stat_points = state.stat_points;
    }
    next.inventory = bag;
    next.coins = coins;
    let level = next.progress.level != s.progress.level;
    if level || bag != before {
        crate::durability::refresh(&mut next, catalog, lobby.skills.as_ref());
    }
    if level {
        next.skills.set_limits(next.combat.hp, next.combat.mp);
        next.skills.hp = next.skills.max_hp;
        next.skills.mana = next.skills.max_mana;
        crate::skills::save_in(&tx, account, slot, &next.skills)?;
    }
    tx.commit()?;
    drop(db);
    let mut packets = Vec::new();
    if let Some(update) = crate::inventory::reward_packet(&before, &bag) {
        packets.push(update);
        let mut weight = vec![0x54];
        weight.extend(next.combat.weight.to_le_bytes());
        packets.push(weight);
    }
    if coins != coins_before {
        packets.push(crate::wallet::change(coins_before, coins));
    }
    if level {
        packets.push(crate::quest_rewards::level_packet(&next));
        packets.push(crate::inventory::attributes(&next.combat, 2));
    } else if next.rewards.experience != s.rewards.experience {
        packets.push(crate::quest_rewards::experience_packet(&next));
    }
    if next.rewards.loyalty != s.rewards.loyalty {
        packets.push(crate::quest_rewards::loyalty_packet(&next));
    }
    if chests > 0 {
        packets.push(crate::operator_chat::notice(
            b"Your Forgotten Temple treasure chest was delivered.",
        ));
    }
    if blocked {
        packets.push(crate::operator_chat::notice(
            b"Your bag is full or too heavy. Your event reward stays reserved and is delivered when space is free.",
        ));
    }
    if capped {
        packets.push(crate::operator_chat::notice(
            b"Your Noah is at the limit. Your event Noah stays reserved until you have room.",
        ));
    }
    tracing::info!(character=%s.character.name, delivered=delivered.len(), blocked, "event rewards delivered");
    *s = next;
    // Unknown items alone do not keep the player pending; they retry after a restart.
    let waiting = blocked || capped;
    Ok((packets, waiting, waiting))
}

/// 1453-adapted Iris menu. The supplied later script compares clock hours;
/// here the server decides the open window, bracket, fee and capacity.
pub(crate) fn iris_script(file: &str) -> Option<Vec<u8>> {
    (file == "13015_Iris.lua").then(|| {
        b"local NPC=13015
if EVENT==165 then
  local level=CheckLevel(UID)
  local ready=4134
  local closed=4138
  if level<60 then ready=4133 closed=4140 end
  if CheckMonsterChallengeTime(UID)>0 then
    SelectMsg(UID,2,-1,ready,NPC,4075,181,4076,169)
  else
    SelectMsg(UID,2,-1,closed,NPC,10,169)
  end
end
if EVENT==181 then
  local result=EnterForgottenTemple(UID)
  if result==1 then SelectMsg(UID,2,-1,4135,NPC,10,169)
  elseif result==2 then SelectMsg(UID,2,-1,4137,NPC,10,169)
  elseif result==3 then SelectMsg(UID,2,-1,4136,NPC,10,169) end
end
"
        .to_vec()
    })
}

/// Temple sign-up state for one player: bracket open, players inside.
pub(crate) fn temple_window(lobby: &Lobby, s: &Session) -> (Option<events::TempleGate>, usize) {
    let hub = lobby.social.lock().unwrap();
    let gate = hub
        .events
        .gates
        .temple
        .clone()
        .filter(|t| t.entry_open && (t.level_min..=t.level_max).contains(&s.progress.level));
    let inside = hub
        .players
        .values()
        .filter(|p| p.s.position.zone == events::TEMPLE_ZONE && p.s.player_id != s.player_id)
        .count();
    (gate, inside)
}

/// Temple entry outcome: 0 enter, 1 closed or wrong level, 2 full, 3 Noah.
pub(crate) fn temple_entry(
    gate: Option<&events::TempleGate>,
    inside: usize,
    coins: u32,
) -> (u8, u32) {
    let Some(gate) = gate else {
        return (1, 0);
    };
    if inside >= usize::from(gate.capacity) {
        return (2, 0);
    }
    if coins < gate.fee {
        return (3, 0);
    }
    (0, gate.fee)
}
