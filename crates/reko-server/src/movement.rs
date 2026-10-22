//! Persist validated movement in protocol coordinate units.

use rusqlite::{Connection, OptionalExtension, params};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Position {
    pub zone: u8,
    pub x: u16,
    pub z: u16,
    pub y: i16,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            zone: 21,
            x: 3200,
            z: 4000,
            y: 0,
        }
    }
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS character_positions (
        account TEXT NOT NULL COLLATE NOCASE,
        slot INTEGER NOT NULL,
        zone INTEGER NOT NULL,
        x INTEGER NOT NULL,
        z INTEGER NOT NULL,
        y INTEGER NOT NULL,
        PRIMARY KEY(account, slot),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
    );",
    )
}

pub(crate) fn load(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<Position> {
    Ok(db
        .query_row(
            "SELECT zone,x,z,y FROM character_positions WHERE account = ?1 AND slot = ?2",
            params![account, slot],
            |row| {
                Ok(Position {
                    zone: row.get(0)?,
                    x: row.get(1)?,
                    z: row.get(2)?,
                    y: row.get(3)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

pub(crate) fn save(
    db: &Connection,
    account: &str,
    slot: u8,
    position: Position,
) -> rusqlite::Result<()> {
    db.execute("INSERT INTO character_positions(account,slot,zone,x,z,y) VALUES (?1,?2,?3,?4,?5,?6)
        ON CONFLICT(account,slot) DO UPDATE SET zone=excluded.zone,x=excluded.x,z=excluded.z,y=excluded.y", params![account,slot,position.zone,position.x,position.z,position.y])?;
    Ok(())
}

#[derive(Clone)]
pub(crate) struct Validator {
    heading: Option<(f32, f32)>,
    forecast: Option<Forecast>,
    last: Instant,
    distance_budget: f32,
    origin: Option<(f32, f32)>,
    observations: std::collections::VecDeque<(f32, f32, f32, f32)>,
    travel: f32,
    last_speed: u16,
}
impl Default for Validator {
    fn default() -> Self {
        Self {
            heading: None,
            forecast: None,
            last: Instant::now(),
            distance_budget: 8.25,
            origin: None,
            observations: std::collections::VecDeque::new(),
            travel: 0.0,
            last_speed: 45,
        }
    }
}
impl Validator {
    pub(crate) fn forward(&self, position: Position, meters: u16) -> Option<Position> {
        let (dx, dz) = self.heading?;
        let x = f32::from(position.x) + dx * f32::from(meters) * 10.0;
        let z = f32::from(position.z) + dz * f32::from(meters) * 10.0;
        (x >= 0.0 && z >= 0.0 && x <= 65535.0 && z <= 65535.0).then_some(Position {
            x: x.round() as u16,
            z: z.round() as u16,
            ..position
        })
    }
    pub(crate) fn current_position(&self, reported: Position, now: Instant) -> Position {
        self.forecast
            .as_ref()
            .filter(|p| p.to == reported)
            .map_or(reported, |p| p.current(now))
    }
    pub(crate) fn combat_position(
        &self,
        reported: Position,
        target: Position,
        now: Instant,
    ) -> Position {
        let Some(path) = self.forecast.as_ref().filter(|p| {
            p.to == reported
                && target.zone == reported.zone
                && now.saturating_duration_since(p.at).as_secs_f32() < 1.5
        }) else {
            return reported;
        };
        let from = path.current(now);
        let dx = f32::from(reported.x) - f32::from(from.x);
        let dz = f32::from(reported.z) - f32::from(from.z);
        let length = dx * dx + dz * dz;
        if length < 1.0 {
            return reported;
        }
        let t = ((f32::from(target.x) - f32::from(from.x)) * dx
            + (f32::from(target.z) - f32::from(from.z)) * dz)
            / length;
        let t = t.clamp(0.0, 1.0);
        Position {
            x: (f32::from(from.x) + dx * t).round() as u16,
            z: (f32::from(from.z) + dz * t).round() as u16,
            ..reported
        }
    }
    pub fn validate(
        &mut self,
        payload: &[u8],
        previous: Position,
        now: Instant,
        max_speed: u16,
    ) -> Option<Position> {
        if payload.len() != 10 || payload[0] != 6 || payload[9] > 3 {
            return None;
        }
        let speed = i16::from_le_bytes([payload[7], payload[8]]);
        if speed.unsigned_abs() > max_speed {
            return None;
        }
        let next = Position {
            zone: previous.zone,
            x: u16::from_le_bytes([payload[1], payload[2]]),
            z: u16::from_le_bytes([payload[3], payload[4]]),
            y: i16::from_le_bytes([payload[5], payload[6]]),
        };
        let elapsed = now
            .saturating_duration_since(self.last)
            .as_secs_f32()
            .min(3.0);
        let added = if self.observations.is_empty() {
            0.0
        } else {
            elapsed * f32::from(max_speed.max(self.last_speed)) / 10.0
        };
        let total = self.travel + added;
        let nx = f32::from(next.x) / 10.0;
        let nz = f32::from(next.z) / 10.0;
        let lead = if payload[9] & 1 != 0 {
            f32::from(speed.unsigned_abs()) / 10.0 * 1.5
        } else {
            0.0
        };
        let (ox, oz) = self
            .origin
            .unwrap_or((f32::from(previous.x) / 10.0, f32::from(previous.z) / 10.0));
        // Forecasts describe a possible current position inside a 1.5 second radius.
        // Keep overlapping time envelopes instead of charging direction changes as travel.
        if (nx - ox).hypot(nz - oz) > self.distance_budget + total + lead + 0.3 {
            return None;
        }
        for &(x, z, prediction, at) in &self.observations {
            if (nx - x).hypot(nz - z) > prediction + total - at + lead + 0.3 {
                return None;
            }
        }
        let dx = f32::from(next.x) - f32::from(previous.x);
        let dz = f32::from(next.z) - f32::from(previous.z);
        let length = dx.hypot(dz);
        if speed != 0 && length >= 1.0 {
            self.heading = Some((dx / length, dz / length));
        }
        self.forecast = if payload[9] & 1 != 0 && speed != 0 {
            let anchor = self
                .forecast
                .as_ref()
                .filter(|p| p.to == previous)
                .map_or(previous, |p| p.current(now));
            let dx = f32::from(next.x) - f32::from(anchor.x);
            let dz = f32::from(next.z) - f32::from(anchor.z);
            let length = dx.hypot(dz);
            if length > 0.0 {
                let trail = (f32::from(speed.unsigned_abs()) * 1.5).min(length);
                let from = Position {
                    x: (f32::from(next.x) - dx / length * trail)
                        .round()
                        .clamp(0.0, 65535.0) as u16,
                    z: (f32::from(next.z) - dz / length * trail)
                        .round()
                        .clamp(0.0, 65535.0) as u16,
                    ..next
                };
                Some(Forecast {
                    from,
                    to: next,
                    at: now,
                })
            } else {
                None
            }
        } else {
            None
        };
        self.origin = Some((ox, oz));
        self.travel = total;
        self.last = now;
        self.last_speed = max_speed;
        self.observations.push_back((nx, nz, lead, total));
        if self.observations.len() > 32 {
            self.observations.pop_front();
        }
        Some(next)
    }
}

#[derive(Clone)]
struct Forecast {
    from: Position,
    to: Position,
    at: Instant,
}
impl Forecast {
    fn current(&self, now: Instant) -> Position {
        let t = (now.saturating_duration_since(self.at).as_secs_f32() / 1.5).clamp(0.0, 1.0);
        Position {
            x: (f32::from(self.from.x) + (f32::from(self.to.x) - f32::from(self.from.x)) * t)
                .round() as u16,
            z: (f32::from(self.from.z) + (f32::from(self.to.z) - f32::from(self.from.z)) * t)
                .round() as u16,
            ..self.to
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn melee_uses_only_the_recent_validated_forecast_segment_and_stop_expires_it() {
        let mut movement = Validator::default();
        let now = movement.last;
        let anchor = Position {
            zone: 21,
            x: 3200,
            z: 4000,
            y: 0,
        };
        let mut packet = vec![6];
        packet.extend(3335u16.to_le_bytes());
        packet.extend(4000u16.to_le_bytes());
        packet.extend(0i16.to_le_bytes());
        packet.extend(90i16.to_le_bytes());
        packet.push(1);
        let reported = movement.validate(&packet, anchor, now, 90).unwrap();
        let target = Position { x: 3210, ..anchor };
        assert_eq!(movement.combat_position(reported, target, now), target);
        let halfway = movement.combat_position(
            reported,
            target,
            now + std::time::Duration::from_millis(750),
        );
        assert!((3267..=3268).contains(&halfway.x));
        let sideways = Position {
            x: 3210,
            z: 4200,
            ..anchor
        };
        assert_eq!(movement.combat_position(reported, sideways, now).z, 4000);
        assert_eq!(
            movement.combat_position(
                reported,
                target,
                now + std::time::Duration::from_millis(1501)
            ),
            reported
        );
        assert_eq!(
            movement.combat_position(reported, Position { zone: 1, ..target }, now),
            reported
        );
        let mut forged = packet.clone();
        forged[1..3].copy_from_slice(&6000u16.to_le_bytes());
        assert!(movement.validate(&forged, reported, now, 90).is_none());
        assert_eq!(movement.combat_position(reported, target, now), target);
        packet[9] = 0;
        packet[7..9].copy_from_slice(&0i16.to_le_bytes());
        packet[1..3].copy_from_slice(&3200u16.to_le_bytes());
        let stopped = movement.validate(&packet, reported, now, 90).unwrap();
        assert_eq!(movement.combat_position(stopped, target, now), stopped);
    }
    #[test]
    fn native_walk_capture_keeps_server_position_through_turns_and_stops() {
        let path = std::path::Path::new("../../logs/capture-20261015-170007.log");
        let Ok(log) = std::fs::read_to_string(path) else {
            return;
        };
        let mut validator = Validator::default();
        let start = validator.last;
        let mut position = Position {
            zone: 21,
            x: 3700,
            z: 783,
            y: 10,
        };
        let mut decoder = crate::framing::Decoder::default();
        let mut first = None;
        let mut seen = 0;
        let mut refused = Vec::new();
        for line in log.lines().filter(|l| {
            l.contains("connection=3 service=\"game\"")
                && (l.contains("received game server packet") || l.contains("name=\"TOWN_WARP\""))
        }) {
            let seconds = line[11..13].parse::<f64>().unwrap() * 3600.0
                + line[14..16].parse::<f64>().unwrap() * 60.0
                + line[17..26].parse::<f64>().unwrap();
            let elapsed = seconds - *first.get_or_insert(seconds);
            let hex = line
                .split("dump=")
                .nth(1)
                .unwrap()
                .split(" | ")
                .next()
                .unwrap();
            let bytes = hex
                .split_whitespace()
                .map(|b| u8::from_str_radix(b, 16).unwrap())
                .collect::<Vec<_>>();
            if line.contains("name=\"TOWN_WARP\"") {
                position.x = u16::from_le_bytes([bytes[5], bytes[6]]);
                position.z = u16::from_le_bytes([bytes[7], bytes[8]]);
                validator = Validator {
                    last: start + std::time::Duration::from_secs_f64(elapsed),
                    ..Validator::default()
                };
                continue;
            }
            decoder.push(&bytes);
            while let Some(p) = decoder.next().unwrap() {
                if p[0] != 6 {
                    continue;
                }
                seen += 1;
                if let Some(next) = validator.validate(
                    &p,
                    position,
                    start + std::time::Duration::from_secs_f64(elapsed),
                    45,
                ) {
                    position = next;
                } else {
                    refused.push((elapsed, p));
                }
            }
        }
        assert_eq!(seen, 222);
        assert!(
            refused.is_empty(),
            "legitimate movement rejected: {:?}",
            refused.first()
        );
    }

    #[test]
    fn captured_direction_changes_and_light_feet_stop_are_valid() {
        let mut validator = Validator::default();
        let start = validator.last;
        let mut position = Position {
            zone: 21,
            x: 3213,
            z: 3983,
            y: 134,
        };
        for (milliseconds, x, z, speed, flag, allowed) in [
            (0, 3199, 4051, 45i16, 3, 45),
            (1505, 3193, 3938, 45, 3, 45),
            (2366, 3226, 4024, 0, 0, 90),
            (3006, 3049, 3999, 90, 3, 90),
            (3489, 3140, 4009, 0, 0, 90),
            (4510, 3355, 3991, 90, 3, 90),
            (4592, 3229, 4001, 0, 0, 90),
        ] {
            let mut payload = vec![6];
            payload.extend(u16::to_le_bytes(x));
            payload.extend(u16::to_le_bytes(z));
            payload.extend(134i16.to_le_bytes());
            payload.extend(speed.to_le_bytes());
            payload.push(flag);
            position = validator
                .validate(
                    &payload,
                    position,
                    start + std::time::Duration::from_millis(milliseconds),
                    allowed,
                )
                .unwrap_or_else(|| {
                    panic!("captured legitimate stop/forecast rejected at {milliseconds}")
                });
        }
        assert_eq!((position.x, position.z), (3229, 4001));
    }

    #[test]
    fn forecast_stop_correction_does_not_consume_travel_twice() {
        let mut validator = Validator {
            distance_budget: 5.0,
            ..Validator::default()
        };
        let now = validator.last;
        let packet = |x: u16, flag, speed: u16| {
            let mut p = vec![6];
            p.extend(x.to_le_bytes());
            p.extend(4000u16.to_le_bytes());
            p.extend(0i16.to_le_bytes());
            p.extend(speed.to_le_bytes());
            p.push(flag);
            p
        };
        let start = Position::default();
        let forecast = validator
            .validate(&packet(3245, 3, 45), start, now, 45)
            .unwrap();
        let stopped = validator
            .validate(&packet(3205, 0, 0), forecast, now, 45)
            .unwrap();
        assert_eq!(stopped.x, 3205);
        assert!(
            validator
                .validate(&packet(3260, 0, 0), stopped, now, 45)
                .is_none()
        );
        let future = validator
            .validate(&packet(3250, 1, 45), stopped, now, 45)
            .unwrap();
        assert!(
            validator
                .validate(&packet(3290, 0, 0), future, now, 45)
                .is_none()
        );
    }

    #[test]
    fn boosted_speed_requires_server_buff_limit() {
        let mut validator = Validator::default();
        let now = validator.last;
        let packet = [6, 0x80, 0x0c, 0xa0, 0x0f, 0, 0, 68, 0, 3];
        let position = Position::default();
        assert!(validator.validate(&packet, position, now, 45).is_none());
        assert_eq!(
            validator.validate(&packet, position, now, 68),
            Some(position)
        );
        assert!(validator.validate(&packet, position, now, 45).is_none());
    }

    #[test]
    fn movement_budget_cannot_be_reset_by_repeating_packets() {
        let mut validator = Validator::default();
        let now = validator.last;
        let mut pos = Position::default();
        let packet = |x: u16| {
            let mut bytes = vec![6];
            bytes.extend(x.to_le_bytes());
            bytes.extend(4000u16.to_le_bytes());
            bytes.extend(0i16.to_le_bytes());
            bytes.extend(45i16.to_le_bytes());
            bytes.push(3);
            bytes
        };
        pos = validator.validate(&packet(3300), pos, now, 45).unwrap();
        assert!(validator.validate(&packet(3400), pos, now, 45).is_none());
        assert!(
            validator
                .validate(
                    &packet(3400),
                    pos,
                    now + std::time::Duration::from_secs(2),
                    45
                )
                .is_some()
        );
        assert!(validator.validate(&[6], pos, now, 45).is_none());
    }

    #[test]
    fn saved_coordinates_are_scoped_to_account_and_slot() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "admin").unwrap();
        crate::characters::set_nation(&db, "admin", 1).unwrap();
        let character = crate::characters::Character {
            slot: 0,
            name: "tester".into(),
            race: 2,
            class: 102,
            face: 0,
            hair: 0,
            stats: [60, 60, 80, 50, 50],
        };
        assert_eq!(
            crate::characters::create(&mut db, "admin", &character).unwrap(),
            0
        );
        initialize(&db).unwrap();
        let pos = Position {
            zone: 21,
            x: 2800,
            z: 3500,
            y: 87,
        };
        save(&db, "admin", 0, pos).unwrap();
        assert_eq!(load(&db, "ADMIN", 0).unwrap(), pos);
        assert_eq!(load(&db, "admin", 1).unwrap(), Position::default());
        assert_eq!(load(&db, "other", 0).unwrap(), Position::default());
    }
}
