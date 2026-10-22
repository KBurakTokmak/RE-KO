//! Scoped Bifrost gate/return scripts with an explicit local access policy.
use crate::{movement::Position, shops::Merchant, world_entry::Session};
use rusqlite::{Connection, OptionalExtension};

pub(crate) fn script(npc: &Merchant, s: &Session, file: &str) -> anyhow::Result<bool> {
    let matches = match file {
        "602_Bifmove.lua" => npc.prototype == 602 && npc.kind == 150 && s.position.zone == 71,
        "16094_Asan.lua" => npc.prototype == 16094 && npc.kind == 39 && s.position.zone == 31,
        _ => return Ok(false),
    };
    anyhow::ensure!(matches && npc.nearby(s), "Bifrost teleporter unavailable");
    Ok(true)
}

pub(crate) fn victory(db: &Connection) -> anyhow::Result<u8> {
    let value: Option<String> = db
        .query_row(
            "SELECT data FROM world_reference_rules WHERE table_name='BIFROST_ACCESS' AND rule_id=31",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let policy = value
        .map(|text| serde_json::from_str::<serde_json::Value>(&text))
        .transpose()?;
    Ok(if policy.is_some_and(|v| v["mode"] == "always_open") {
        3
    } else {
        0
    })
}

pub(crate) fn destination(db: &Connection, s: &Session, args: &[i64]) -> anyhow::Result<Position> {
    anyhow::ensure!(
        args.len() == 4 && args[0] == i64::from(s.player_id),
        "Invalid Bifrost travel request"
    );
    anyhow::ensure!(
        !s.skills
            .buffs
            .get(&22)
            .is_some_and(|b| crate::skills::now_ms() < b.until),
        "You cannot teleport while frozen"
    );
    let nation = s.character.nation();
    let expected = match s.position.zone {
        71 => {
            anyhow::ensure!(victory(db)? == 3, "Bifrost access is closed");
            let (kx, kz, ex, ez): (i64, i64, i64, i64) = db.query_row(
                "SELECT karus_x,karus_z,elmo_x,elmo_z FROM world_town_starts WHERE zone=31",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            if nation == 1 {
                [31, kx, kz]
            } else {
                [31, ex, ez]
            }
        }
        31 if s.progress.level < 30 => {
            if nation == 1 {
                [1, 441, 1625]
            } else {
                [2, 1595, 412]
            }
        }
        31 => {
            if nation == 1 {
                [71, 1014, 1020]
            } else {
                [71, 1026, 984]
            }
        }
        _ => anyhow::bail!("Unsupported Bifrost source zone"),
    };
    anyhow::ensure!(
        args[1..] == expected,
        "Bifrost destination does not match the source route"
    );
    let zone = u8::try_from(args[1])?;
    let x = u16::try_from(args[2])?
        .checked_mul(10)
        .ok_or_else(|| anyhow::anyhow!("Invalid Bifrost X"))?;
    let z = u16::try_from(args[3])?
        .checked_mul(10)
        .ok_or_else(|| anyhow::anyhow!("Invalid Bifrost Z"))?;
    let configured: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM world_town_starts WHERE zone=?1)",
        [zone],
        |r| r.get(0),
    )?;
    anyhow::ensure!(configured, "Bifrost destination is not configured");
    Ok(Position { zone, x, z, y: 0 })
}
