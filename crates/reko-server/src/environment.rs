//! Shared server clock and hourly sky, projected into each loaded map.
use chrono::{DateTime, Datelike, Local, Timelike};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::net::TcpStream;

use crate::{lobby::Lobby, world_entry::Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Weather {
    kind: u8,
    amount: u8,
}

impl Weather {
    fn from_roll(kind: u32, amount: u8) -> Self {
        match kind {
            0..=1 => Self { kind: 3, amount },
            2..=6 => Self { kind: 2, amount },
            _ => Self {
                kind: 1,
                amount: if amount > 70 { amount / 2 } else { 0 },
            },
        }
    }

    fn packet(self, zone: u8) -> [u8; 4] {
        let kind = if matches!(zone, 32 | 33 | 48) {
            1
        } else {
            self.kind
        };
        [0x14, kind, self.amount, 0]
    }
}

pub(crate) struct Sky {
    hour: Option<(i32, u32, u32, i32)>,
    weather: Weather,
    seed: u64,
}

impl Default for Sky {
    fn default() -> Self {
        Self {
            hour: None,
            weather: Weather { kind: 1, amount: 0 },
            seed: crate::skills::now_ms(),
        }
    }
}

impl Sky {
    fn advance(&mut self, now: &DateTime<Local>) {
        let hour = (
            now.year(),
            now.ordinal(),
            now.hour(),
            now.offset().local_minus_utc(),
        );
        if self.hour != Some(hour) {
            self.hour = Some(hour);
            let kind = crate::loot::roll(&mut self.seed, 0, 100);
            let amount = crate::loot::roll(&mut self.seed, 0, 100) as u8;
            self.weather = Weather::from_roll(kind, amount);
        }
    }

    pub(crate) fn set_weather(&mut self, args: &[u8], now: &DateTime<Local>) -> &'static [u8] {
        let usage = b"Use op weather clear|rain|snow <0-100>.";
        let Ok(args) = std::str::from_utf8(args) else {
            return usage;
        };
        let mut words = args.split_ascii_whitespace();
        let kind = match words.next() {
            Some("clear") => 1,
            Some("rain") => 2,
            Some("snow") => 3,
            _ => return usage,
        };
        let amount = words.next().and_then(|word| word.parse::<u8>().ok());
        let Some(amount) = amount.filter(|n| *n <= 100 && words.next().is_none()) else {
            return usage;
        };
        self.advance(now);
        self.weather = Weather { kind, amount };
        b"World weather set until the next hour."
    }
}

#[derive(Clone, Default)]
pub(crate) struct View {
    next_check: Option<Instant>,
    weather: Option<[u8; 4]>,
}

pub(crate) async fn sync(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    s: &mut Session,
    entry: bool,
    clock: impl FnOnce() -> DateTime<Local>,
) {
    let tick = Instant::now();
    if !s.entered || s.zone_loading || (!entry && s.sky.next_check.is_some_and(|next| tick < next))
    {
        return;
    }
    s.sky.next_check = Some(tick + Duration::from_secs(6));
    let (now, weather) = {
        let mut sky = lobby.sky.lock().unwrap();
        let now = clock();
        sky.advance(&now);
        let weather = sky.weather.packet(s.position.zone);
        (now, weather)
    };
    if entry {
        let mut packet = vec![0x13];
        for field in [
            now.year() as u16,
            now.month() as u16,
            now.day() as u16,
            now.hour() as u16,
            now.minute() as u16,
        ] {
            packet.extend(field.to_le_bytes());
        }
        crate::send_response(sock, peer, &None, &packet, "WORLD_TIME").await;
    }
    if entry || s.sky.weather != Some(weather) {
        crate::send_response(sock, peer, &None, &weather, "WORLD_WEATHER").await;
        s.sky.weather = Some(weather);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[tokio::test]
    async fn weather_updates_leave_the_client_clock_running() {
        let lobby = crate::social::tests::lobby();
        let mut player = crate::social::tests::player(&lobby, "SkyTest", 1, 80);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut server, peer) = listener.accept().await.unwrap();
        let now = Local::now().with_minute(0).unwrap();
        sync(&mut server, peer, &lobby, &mut player, true, || now).await;
        let time = crate::tests::receive(&mut client).await;
        assert_eq!((time[0], time.len()), (0x13, 11));
        let initial = crate::tests::receive(&mut client).await;
        let (command, kind) = if initial[1] == 2 {
            ("snow 65", 3)
        } else {
            ("rain 65", 2)
        };
        lobby
            .sky
            .lock()
            .unwrap()
            .set_weather(command.as_bytes(), &now);
        player.sky.next_check = None;
        sync(&mut server, peer, &lobby, &mut player, false, || {
            now + Duration::minutes(1)
        })
        .await;
        assert_eq!(
            crate::tests::receive(&mut client).await,
            [0x14, kind, 65, 0]
        );
        player.sky.next_check = None;
        sync(&mut server, peer, &lobby, &mut player, false, || {
            now + Duration::minutes(1)
        })
        .await;
        // A marker after the unchanged forecast detects any redundant packets.
        crate::send_response(&mut server, peer, &None, &[0x6b, 0, 0], "TEST").await;
        assert_eq!(crate::tests::receive(&mut client).await, [0x6b, 0, 0]);
    }

    #[test]
    fn shared_forecast_overrides_expire_at_hour_boundary_and_project_per_zone() {
        let mut sky = Sky::default();
        let now = Local::now();
        assert_eq!(
            sky.set_weather(b"snow 80", &now),
            b"World weather set until the next hour."
        );
        sky.advance(&now);
        let seed = sky.seed;
        assert_eq!(sky.weather.packet(21), [0x14, 3, 80, 0]);
        for zone in [32, 33, 48] {
            assert_eq!(sky.weather.packet(zone), [0x14, 1, 80, 0]);
        }
        for invalid in [b"rain 101".as_slice(), b"clear", b"snow 1 2"] {
            assert!(sky.set_weather(invalid, &now).starts_with(b"Use "));
            assert_eq!(sky.weather.packet(21), [0x14, 3, 80, 0]);
        }
        sky.advance(&(now + Duration::hours(1)));
        assert_ne!(sky.seed, seed);
        assert_eq!(Weather::from_roll(1, 72).kind, 3);
        assert_eq!(Weather::from_roll(2, 72).kind, 2);
        assert_eq!(Weather::from_roll(6, 72).kind, 2);
        assert_eq!(Weather::from_roll(7, 71).packet(21), [0x14, 1, 35, 0]);
        assert_eq!(Weather::from_roll(100, 70).packet(21), [0x14, 1, 0, 0]);
    }
}
