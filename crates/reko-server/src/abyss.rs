//! Validation for the shared Abyss teleporter script and source trap records.
use crate::{movement::Position, shops::Merchant, world_entry::Session};
use rusqlite::Connection;

pub(crate) fn trigger(
    db: &Connection,
    npc: &Merchant,
    s: &Session,
    file: &str,
) -> anyhow::Result<Option<i32>> {
    if file != "7001_dungeun.lua" {
        return Ok(None);
    }
    anyhow::ensure!(
        npc.prototype == 7001
            && npc.kind == 28
            && npc.nearby(s)
            && matches!(s.position.zone, 32..=34),
        "Abyss teleporter unavailable"
    );
    let trap: u16 = db.query_row("SELECT c.trap FROM world_spawn_conditions c JOIN world_spawns s ON s.id=c.entity_id WHERE s.id=?1 AND s.zone=?2 AND s.prototype=7001 AND s.kind=28 AND c.enabled=1",rusqlite::params![npc.id,s.position.zone],|r|r.get(0))?;
    let mut q = db.prepare(
        "SELECT data FROM world_reference_rules WHERE table_name='EVENT_TRIGGER' ORDER BY rule_id",
    )?;
    let mut found = None;
    for text in q.query_map([], |r| r.get::<_, String>(0))? {
        let row: serde_json::Value = serde_json::from_str(&text?)?;
        let n = |key: &str| {
            row[key]
                .as_str()
                .and_then(|s| s.parse::<i32>().ok())
                .or_else(|| row[key].as_i64().and_then(|n| n.try_into().ok()))
                .unwrap_or(0)
        };
        if n("bNpcType") == 28 && n("sNpcID") == i32::from(trap) {
            anyhow::ensure!(found.is_none(), "Ambiguous Abyss trigger");
            found = Some(n("nTriggerNum"));
        }
    }
    let event = found.ok_or_else(|| anyhow::anyhow!("Abyss trigger unavailable"))?;
    anyhow::ensure!(
        match s.position.zone {
            32 => (40001..=40020).contains(&event),
            33 => (42001..=42016).contains(&event),
            34 => event == 43601,
            _ => false,
        },
        "Abyss trigger does not match this zone"
    );
    Ok(Some(event))
}

pub(crate) fn destination(db: &Connection, s: &Session, args: &[i64]) -> anyhow::Result<Position> {
    anyhow::ensure!(
        args.len() == 4 && args[0] == i64::from(s.player_id),
        "Invalid Abyss travel request"
    );
    anyhow::ensure!(
        !s.skills
            .buffs
            .get(&22)
            .is_some_and(|b| crate::skills::now_ms() < b.until),
        "You cannot teleport while frozen"
    );
    let zone = u8::try_from(args[1])?;
    anyhow::ensure!(
        matches!(zone, 30 | 32..=34),
        "Unsupported Abyss destination"
    );
    let x = u16::try_from(args[2])?
        .checked_mul(10)
        .ok_or_else(|| anyhow::anyhow!("Invalid Abyss X"))?;
    let z = u16::try_from(args[3])?
        .checked_mul(10)
        .ok_or_else(|| anyhow::anyhow!("Invalid Abyss Z"))?;
    let configured: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM world_town_starts WHERE zone=?1)",
        [zone],
        |r| r.get(0),
    )?;
    anyhow::ensure!(configured, "Abyss destination is not configured");
    let anchor: bool = if zone == 30 {
        db.query_row("SELECT EXISTS(SELECT 1 FROM world_town_starts WHERE zone=30 AND (karus_x*10-?1)*(karus_x*10-?1)+(karus_z*10-?2)*(karus_z*10-?2)<=90000)",[x,z],|r|r.get(0))?
    } else {
        db.query_row("SELECT EXISTS(SELECT 1 FROM world_spawns WHERE zone=?1 AND prototype=7001 AND kind=28 AND (x0-?2)*(x0-?2)+(z0-?3)*(z0-?3)<=90000)",rusqlite::params![zone,x,z],|r|r.get(0))?
    };
    anyhow::ensure!(
        anchor,
        "Abyss destination does not match source floor geometry"
    );
    Ok(Position { zone, x, z, y: 0 })
}
