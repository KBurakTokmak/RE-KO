//! Shared suppression-zone completion and safe, persisted return to town.
use crate::{lobby::Lobby, movement::Position, world_entry::Session};
use std::collections::HashMap;

pub(crate) const LOOT_SECONDS: u64 = 60;
#[derive(Default)]
pub(crate) struct Rooms {
    bosses: HashMap<u16, u8>,
    deadlines: HashMap<u8, u64>,
}
impl Rooms {
    pub fn load(spawns: &[crate::population::Spawn]) -> Self {
        Self {
            bosses: spawns
                .iter()
                .filter(|s| {
                    s.monster && matches!((s.position.zone, s.prototype), (82, 7008) | (83, 7013))
                })
                .map(|s| (s.id, s.position.zone))
                .collect(),
            ..Self::default()
        }
    }
    pub fn death(&mut self, id: u16, now: u64) {
        if let Some(&zone) = self.bosses.get(&id) {
            self.deadlines
                .entry(zone)
                .or_insert_with(|| now.saturating_add(LOOT_SECONDS * 1000));
        }
    }
    pub fn deadline(&self, zone: u8) -> Option<u64> {
        self.deadlines.get(&zone).copied()
    }
}
pub(crate) fn home(db: &rusqlite::Connection, nation: u8) -> anyhow::Result<Position> {
    anyhow::ensure!(matches!(nation, 1 | 2), "Invalid home nation.");
    let (kx, kz, ex, ez): (u16, u16, u16, u16) = db.query_row(
        "SELECT karus_x,karus_z,elmo_x,elmo_z FROM world_town_starts WHERE zone=?1",
        [nation],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let (x, z) = if nation == 1 { (kx, kz) } else { (ex, ez) };
    Ok(Position {
        zone: nation,
        x: x.checked_mul(10)
            .ok_or_else(|| anyhow::anyhow!("Invalid home X."))?,
        z: z.checked_mul(10)
            .ok_or_else(|| anyhow::anyhow!("Invalid home Z."))?,
        y: 0,
    })
}
pub(crate) fn tick(
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    now: u64,
) -> anyhow::Result<Vec<Vec<u8>>> {
    if !s.entered || !matches!(s.position.zone, 82 | 83) {
        s.suppression_notice = None;
        return Ok(Vec::new());
    }
    let Some(deadline) = lobby
        .monsters
        .as_ref()
        .and_then(|w| w.suppression_deadline(s.position.zone))
    else {
        s.suppression_notice = None;
        return Ok(Vec::new());
    };
    if now >= deadline {
        let db = lobby.db.lock().unwrap();
        let position = home(&db, s.character.nation())?;
        crate::movement::save(&db, account, s.character.slot, position)?;
        drop(db);
        return Ok(vec![crate::travel::begin(lobby, s, position)]);
    }
    let seconds = (deadline - now).div_ceil(1000);
    let first = s
        .suppression_notice
        .is_none_or(|(zone, end, _)| zone != s.position.zone || end != deadline);
    let previous = s.suppression_notice.map(|(_, _, seconds)| seconds);
    s.suppression_notice = Some((s.position.zone, deadline, seconds));
    if first || previous != Some(seconds) && (seconds.is_multiple_of(10) || seconds <= 5) {
        let text = format!(
            "Suppression boss defeated. Collect your loot; returning to your castle in {seconds} seconds."
        );
        return Ok(vec![crate::chat::packet(
            7,
            s.character.nation(),
            u16::MAX,
            "",
            text.as_bytes(),
        )]);
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    #[test]
    fn completion_countdown_returns_home_after_loot_time_and_retries_failed_saves() {
        for (zone, prototype) in [(82, 7008), (83, 7013)] {
            let Some((mut lobby, mut s)) = crate::world_service_tests::setup() else {
                return;
            };
            s.position = Position {
                zone,
                x: 2000,
                z: 2000,
                y: 0,
            };
            {
                let db = lobby.db.lock().unwrap();
                crate::population::tests::seed(&db);
                db.execute("UPDATE world_spawns SET zone=?1,prototype=?2,hp=50,regen_type=1,x0=2000,x1=2000,z0=2000,z1=2000",params![zone,prototype]).unwrap();
                db.execute(
                    "INSERT INTO world_town_starts VALUES(1,354,1610,1644,333,0,0,'test')",
                    [],
                )
                .unwrap();
                db.execute("INSERT INTO character_quests VALUES ('test',0,603,1)", [])
                    .unwrap();
                db.execute(
                    "INSERT INTO character_inventory VALUES ('test',0,0,910135000,1,1)",
                    [],
                )
                .unwrap();
                crate::movement::save(&db, "test", 0, s.position).unwrap();
            }
            let world = std::sync::Arc::new(
                crate::monsters::World::load(&reko_common::game_dir()).unwrap(),
            );
            world
                .load_population(&lobby.db.lock().unwrap(), &reko_common::game_dir())
                .unwrap();
            lobby.monsters = Some(world.clone());
            assert!(
                tick(&lobby, "test", &mut s, crate::skills::now_ms())
                    .unwrap()
                    .is_empty()
            );
            world
                .hit_owned(
                    12000,
                    crate::monsters::Attack {
                        position: s.position,
                        range: 5.0,
                        attack: 0,
                        percent: 0,
                        extra: 100,
                        skill: true,
                        drain_percent: 0,
                        arrow: false,
                        penetrate: false,
                        magic: None,
                    },
                    Some("ServiceTest"),
                    || Ok(()),
                )
                .unwrap();
            let deadline = world.suppression_deadline(zone).unwrap();
            let start = deadline - LOOT_SECONDS * 1000;
            let packets = tick(&lobby, "test", &mut s, start).unwrap();
            assert!(packets[0].windows(10).any(|w| w == b"60 seconds"));
            assert!(tick(&lobby, "test", &mut s, start + 1).unwrap().is_empty());
            // Reconnecting receives the remaining time, rather than a fresh full minute.
            let mut reconnect = s.clone();
            reconnect.suppression_notice = None;
            let p = tick(&lobby, "test", &mut reconnect, deadline - 1000).unwrap();
            assert!(p[0].windows(9).any(|w| w == b"1 seconds"));
            lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_return BEFORE INSERT ON character_positions BEGIN SELECT RAISE(ABORT,'test');END;").unwrap();
            assert!(tick(&lobby, "test", &mut s, deadline).is_err());
            assert_eq!(s.position.zone, zone);
            assert!(s.entered && !s.zone_loading);
            lobby
                .db
                .lock()
                .unwrap()
                .execute_batch("DROP TRIGGER fail_return")
                .unwrap();
            let p = tick(&lobby, "test", &mut s, deadline + 1).unwrap();
            assert_eq!(&p[0][..4], &[0x27, 3, 1, 0]);
            assert_eq!((s.position.x, s.position.z), (3540, 16100));
            assert!(s.zone_loading && !s.entered);
            assert!(
                tick(&lobby, "test", &mut s, deadline + 2)
                    .unwrap()
                    .is_empty()
            );
            let db = lobby.db.lock().unwrap();
            assert_eq!(crate::movement::load(&db, "test", 0).unwrap().zone, 1);
            assert_eq!(
                crate::starter::inventory(&db, "test", 0).unwrap()[0].id,
                910135000
            );
            assert_eq!(db.query_row("SELECT state FROM character_quests WHERE account='test' AND slot=0 AND quest=603",[],|r|r.get::<_,u8>(0)).unwrap(),1);
            assert_eq!(crate::starter::coins(&db, "test", 0).unwrap(), 1000000);
        }
    }
}
