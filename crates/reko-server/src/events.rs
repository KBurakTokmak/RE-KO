//! Scheduled server events: Forgotten Temple, Lunar War, Dark Lunar War and
//! Castle Siege War. This module is a pure state machine driven by server-local
//! time; `event_runtime` feeds it world input and applies its actions.
//!
//! Timings, notices and win rules follow known 1453 server behavior. Schedules,
//! brackets and rewards are owner data (`EVENT_SCHEDULE` rules). Running events
//! are not resumed after a restart.
use crate::monsters::EventTag;
use chrono::{Datelike, Duration, NaiveDateTime, NaiveTime, Weekday};
use std::collections::HashMap;

pub(crate) const TEMPLE_ZONE: u8 = 55;
pub(crate) const DELOS: u8 = 30;
pub(crate) const MORADON: u8 = 21;
pub(crate) const NOAH: u32 = 900000000;
pub(crate) const EXPERIENCE: u32 = 900001000;
/// Sign-up window before the first wave announcement.
const TEMPLE_SIGNUP_S: i64 = 300;
/// The first stage spawns 45 s after a 30 s offset from the wave notice.
const TEMPLE_FIRST_WAVE_S: i64 = 375;
const TEMPLE_WAVE_S: i64 = 45;
/// Uncleared temples fail 50 minutes after the gate opens.
const TEMPLE_LIMIT_S: i64 = 3000;
const TEMPLE_MAX_STAGES: usize = 56;
/// Delay between a war closing and its zones being cleared.
const BANISH_S: i64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Temple,
    Lunar,
    DarkLunar,
    Siege,
}

impl Kind {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "temple" | "ft" | "forgottentemple" => Self::Temple,
            "lunar" | "lunarwar" | "war" => Self::Lunar,
            "darklunar" | "darklunarwar" | "dark" => Self::DarkLunar,
            "siege" | "csw" | "castlesiege" => Self::Siege,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Temple => "temple",
            Self::Lunar => "lunar",
            Self::DarkLunar => "darklunar",
            Self::Siege => "siege",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Temple => "Forgotten Temple",
            Self::Lunar => "Lunar War",
            Self::DarkLunar => "Dark Lunar War",
            Self::Siege => "Castle Siege War",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TempleRule {
    pub level_min: u8,
    pub level_max: u8,
    pub chest: u32,
    /// Wave list (`bLevel` of the 1453 summon list).
    pub waves: u8,
    pub fee: u32,
    pub capacity: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WarRule {
    /// Weekly rotation by ISO week.
    pub zones: Vec<u8>,
    pub duration_s: i64,
    pub level_min: u8,
    pub level_max: u8,
    pub nation_cap: u16,
    pub np_kill: u32,
    pub np_death: u32,
    pub invasion: bool,
    pub victory_noah: u32,
    pub victory_exp: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SiegeRule {
    pub countdown_s: i64,
    pub duration_s: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Rule {
    Temple(TempleRule),
    War(WarRule),
    Siege(SiegeRule),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub id: u32,
    pub kind: Kind,
    /// Empty means daily.
    pub days: Vec<Weekday>,
    pub time: NaiveTime,
    pub rule: Rule,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Days {
    Word(String),
    List(Vec<String>),
}

#[derive(serde::Deserialize)]
struct Row {
    event: String,
    days: Option<Days>,
    time: String,
    enabled: Option<bool>,
    level_min: Option<u8>,
    level_max: Option<u8>,
    chest: Option<u32>,
    waves: Option<u8>,
    fee: Option<u32>,
    capacity: Option<u16>,
    minutes: Option<i64>,
    zones: Option<Vec<u8>>,
    nation_cap: Option<u16>,
    np_kill: Option<u32>,
    np_death: Option<u32>,
    invasion: Option<bool>,
    victory_noah: Option<u32>,
    victory_exp: Option<u32>,
    countdown_minutes: Option<i64>,
}

fn weekday(text: &str) -> anyhow::Result<Weekday> {
    text.parse::<Weekday>()
        .map_err(|_| anyhow::anyhow!("unknown weekday {text}"))
}

/// Parses one owner schedule row. Disabled rows return `None`.
pub(crate) fn parse_entry(id: u32, text: &str) -> anyhow::Result<Option<Entry>> {
    let row: Row = serde_json::from_str(text)?;
    if row.enabled == Some(false) {
        return Ok(None);
    }
    let kind =
        Kind::parse(&row.event).ok_or_else(|| anyhow::anyhow!("unknown event {}", row.event))?;
    let days = match row.days {
        None => Vec::new(),
        Some(Days::Word(word)) if word.eq_ignore_ascii_case("daily") => Vec::new(),
        Some(Days::Word(word)) => vec![weekday(&word)?],
        Some(Days::List(list)) => list.iter().map(|d| weekday(d)).collect::<Result<_, _>>()?,
    };
    let time = NaiveTime::parse_from_str(&row.time, "%H:%M")?;
    let rule = match kind {
        Kind::Temple => {
            let rule = TempleRule {
                level_min: row.level_min.unwrap_or(60),
                level_max: row.level_max.unwrap_or(80),
                chest: row.chest.unwrap_or(379156000),
                waves: row.waves.unwrap_or(3),
                fee: row.fee.unwrap_or(100000),
                capacity: row.capacity.unwrap_or(33),
            };
            anyhow::ensure!(
                rule.level_min >= 1
                    && rule.level_min <= rule.level_max
                    && rule.level_max <= 83
                    && rule.chest > 0
                    && rule.capacity > 0,
                "invalid temple rule"
            );
            Rule::Temple(rule)
        }
        Kind::Lunar | Kind::DarkLunar => {
            let dark = kind == Kind::DarkLunar;
            let rule = WarRule {
                zones: row
                    .zones
                    .unwrap_or_else(|| if dark { vec![63] } else { vec![61, 62, 64] }),
                duration_s: row.minutes.unwrap_or(120) * 60,
                level_min: row.level_min.unwrap_or(35),
                level_max: row.level_max.unwrap_or(if dark { 59 } else { 83 }),
                nation_cap: row.nation_cap.unwrap_or(150),
                np_kill: row.np_kill.unwrap_or(64),
                np_death: row.np_death.unwrap_or(50),
                invasion: row.invasion.unwrap_or(!dark),
                victory_noah: row.victory_noah.unwrap_or(100000),
                victory_exp: row.victory_exp.unwrap_or(5000),
            };
            anyhow::ensure!(
                !rule.zones.is_empty()
                    && rule.zones.iter().all(|z| (61..=64).contains(z))
                    && rule.duration_s >= 120
                    && rule.level_min <= rule.level_max,
                "invalid war rule"
            );
            Rule::War(rule)
        }
        Kind::Siege => {
            let rule = SiegeRule {
                countdown_s: row.countdown_minutes.unwrap_or(10) * 60,
                duration_s: row.minutes.unwrap_or(60) * 60,
            };
            anyhow::ensure!(
                rule.countdown_s >= 0 && rule.duration_s > 0,
                "invalid siege rule"
            );
            Rule::Siege(rule)
        }
    };
    Ok(Some(Entry {
        id,
        kind,
        days,
        time,
        rule,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wave {
    pub stage: u8,
    pub sid: u16,
    pub count: u16,
    pub x: u16,
    pub z: u16,
    pub range: u16,
}

/// Groups summon-list rows by bracket. Like the 1453 server loop, only the
/// first row (lowest index) of each stage spawns, for at most 56 stages.
pub(crate) fn waves_from_rows(
    rows: impl IntoIterator<Item = (u32, serde_json::Value)>,
) -> HashMap<u8, Vec<Wave>> {
    let mut rows: Vec<_> = rows.into_iter().collect();
    rows.sort_by_key(|(index, _)| *index);
    let mut waves: HashMap<u8, Vec<Wave>> = HashMap::new();
    for (_, row) in rows {
        let n = |key: &str| row[key].as_u64().unwrap_or(0);
        let level = n("bLevel") as u8;
        let wave = Wave {
            stage: n("bStage") as u8,
            sid: n("sSid") as u16,
            count: n("sCount") as u16,
            x: n("sPosX") as u16,
            z: n("sPosZ") as u16,
            range: n("bRange") as u16,
        };
        if wave.sid == 0 || wave.count == 0 || wave.stage == 0 {
            continue;
        }
        let list = waves.entry(level).or_default();
        if !list.iter().any(|w| w.stage == wave.stage) {
            list.push(wave);
        }
    }
    for list in waves.values_mut() {
        list.sort_by_key(|w| w.stage);
        list.truncate(TEMPLE_MAX_STAGES);
    }
    waves
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// Native notice to everyone, a zone and/or a nation.
    Notice {
        zone: Option<u8>,
        nation: Option<u8>,
        text: String,
    },
    SpawnWave(Wave),
    Despawn(EventTag),
    /// Queue rewards for living players in a zone.
    RewardZone {
        zone: u8,
        items: Vec<(u32, u32)>,
        source: &'static str,
    },
    /// Queue rewards for a nation's players in the listed zones.
    RewardNation {
        nation: u8,
        zones: Vec<u8>,
        items: Vec<(u32, u32)>,
        source: &'static str,
    },
    /// Make the war-zone keepers, warders and monuments combat entities.
    ArmWar {
        zone: u8,
    },
    SpawnArtifact {
        owner: u16,
    },
    CastleOwner {
        clan: u16,
    },
    /// A homeland's invasion state (guards and hostility).
    Invasion {
        zone: u8,
        open: bool,
    },
}

fn notice(text: impl Into<String>) -> Action {
    Action::Notice {
        zone: None,
        nation: None,
        text: text.into(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TempleGate {
    pub level_min: u8,
    pub level_max: u8,
    pub fee: u32,
    pub capacity: u16,
    pub entry_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WarGate {
    pub zone: u8,
    pub entry_open: bool,
    pub loser: Option<u8>,
    pub level_min: u8,
    pub level_max: u8,
    pub nation_cap: u16,
    pub np_kill: u32,
    pub np_death: u32,
    /// Zones closed for the war's duration.
    pub lockout: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SiegeGate {
    pub battle: bool,
    pub owner: u16,
}

/// Zone policy shared with sessions, travel and PvP code.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Gates {
    pub temple: Option<TempleGate>,
    pub war: Option<WarGate>,
    /// Homeland zone (1 or 2) open to the other nation.
    pub invasion: Option<u8>,
    pub siege: Option<SiegeGate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    Moradon,
    Home,
}

impl Gates {
    /// Where a player standing in `zone` must be sent, if anywhere.
    pub fn exit(&self, zone: u8, nation: u8, admin: bool) -> Option<Exit> {
        if admin {
            return None;
        }
        match zone {
            TEMPLE_ZONE if self.temple.is_none() => Some(Exit::Moradon),
            61..=64 => match &self.war {
                Some(war) if war.zone == zone && war.loser != Some(nation) => None,
                _ => Some(Exit::Home),
            },
            1 | 2 if zone != nation && self.invasion != Some(zone) => Some(Exit::Home),
            DELOS if self.siege.is_some_and(|s| !s.battle) => Some(Exit::Moradon),
            _ if self.war.as_ref().is_some_and(|w| w.lockout.contains(&zone)) => Some(Exit::Home),
            _ => None,
        }
    }
    /// War-zone gate check; `present` is the nation's player count in the zone.
    pub fn war_entry(
        &self,
        zone: u8,
        nation: u8,
        level: u8,
        present: usize,
    ) -> Result<(), &'static str> {
        let war = self
            .war
            .as_ref()
            .filter(|w| w.zone == zone && w.entry_open)
            .ok_or("The war gate is closed.")?;
        if war.loser == Some(nation) {
            return Err("Your nation lost this war.");
        }
        if level < war.level_min || level > war.level_max {
            return Err("Your level does not qualify for this war.");
        }
        if present >= usize::from(war.nation_cap) {
            return Err("Your nation's war zone is full.");
        }
        Ok(())
    }
    /// National points (killer gain, victim loss) for a kill in a war zone.
    pub fn war_np(&self, zone: u8) -> Option<(u32, u32)> {
        self.war
            .as_ref()
            .filter(|w| w.zone == zone)
            .map(|w| (w.np_kill, w.np_death))
    }
    pub fn siege_battle(&self) -> bool {
        self.siege.is_some_and(|s| s.battle)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum TemplePhase {
    Signup,
    Waves,
    Cleared {
        end: NaiveDateTime,
    },
    Failed {
        notice: Option<NaiveDateTime>,
        end: NaiveDateTime,
    },
}

#[derive(Clone, Debug)]
struct Temple {
    rule: TempleRule,
    start: NaiveDateTime,
    waves: Vec<Wave>,
    next: usize,
    next_summon: NaiveDateTime,
    phase: TemplePhase,
}

#[derive(Clone, Debug)]
struct War {
    kind: Kind,
    rule: WarRule,
    zone: u8,
    start: NaiveDateTime,
    /// Keepers/warders killed, indexed by the NPC's nation (Karus, El Morad).
    killed: [u8; 2],
    /// Player deaths (Karus, El Morad).
    deaths: [u32; 2],
    monuments: HashMap<u16, u8>,
    points: [u32; 2],
    winner: Option<u8>,
    next_status: NaiveDateTime,
    nereid_checked: bool,
    half_checked: bool,
    /// Banish notice and final clear once closed.
    closing: Option<(Option<NaiveDateTime>, NaiveDateTime)>,
}

impl War {
    fn monument_counts(&self) -> [usize; 2] {
        [1, 2].map(|n| self.monuments.values().filter(|v| **v == n).count())
    }
    fn status_interval(&self) -> Duration {
        Duration::seconds(if self.zone == 64 && self.winner.is_none() {
            self.rule.duration_s / 48
        } else {
            self.rule.duration_s / 24
        })
    }
}

#[derive(Clone, Debug)]
struct Siege {
    battle_at: NaiveDateTime,
    end_at: NaiveDateTime,
    warned: Vec<i64>,
    battle: bool,
}

fn compare(karus: u64, elmorad: u64) -> Option<u8> {
    match karus.cmp(&elmorad) {
        std::cmp::Ordering::Greater => Some(1),
        std::cmp::Ordering::Less => Some(2),
        std::cmp::Ordering::Equal => None,
    }
}

fn nation_name(nation: u8) -> &'static str {
    if nation == 1 { "Karus" } else { "El Morad" }
}

#[derive(Default)]
pub(crate) struct Controller {
    pub schedule: Vec<Entry>,
    pub waves: HashMap<u8, Vec<Wave>>,
    pub castle_owner: Option<(u16, String)>,
    last: Option<NaiveDateTime>,
    temple: Option<Temple>,
    war: Option<War>,
    siege: Option<Siege>,
    temple_alive: usize,
    actions: Vec<Action>,
    log: Vec<String>,
}

impl Controller {
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }
    fn push(&mut self, action: Action) {
        self.actions.push(action);
    }
    pub fn running(&self, kind: Kind) -> bool {
        match kind {
            Kind::Temple => self.temple.is_some(),
            Kind::Lunar | Kind::DarkLunar => self.war.as_ref().is_some_and(|w| w.kind == kind),
            Kind::Siege => self.siege.is_some(),
        }
    }

    pub fn gates(&self) -> Gates {
        Gates {
            temple: self.temple.as_ref().map(|t| TempleGate {
                level_min: t.rule.level_min,
                level_max: t.rule.level_max,
                fee: t.rule.fee,
                capacity: t.rule.capacity,
                entry_open: t.phase == TemplePhase::Signup,
            }),
            war: self.war.as_ref().map(|w| WarGate {
                zone: w.zone,
                entry_open: w.closing.is_none(),
                loser: w.winner.map(|n| 3 - n),
                level_min: w.rule.level_min,
                level_max: w.rule.level_max,
                nation_cap: w.rule.nation_cap,
                np_kill: w.rule.np_kill,
                np_death: w.rule.np_death,
                lockout: match (w.closing, w.kind) {
                    (Some(_), _) => Vec::new(),
                    (None, Kind::DarkLunar) => vec![72],
                    (None, _) => vec![71, 72, 73],
                },
            }),
            invasion: self.invasion(),
            siege: self.siege.as_ref().map(|s| SiegeGate {
                battle: s.battle,
                owner: self.castle_owner.as_ref().map_or(0, |(id, _)| *id),
            }),
        }
    }

    /// The loser's homeland stays open to the winner until the war clears.
    fn invasion(&self) -> Option<u8> {
        let war = self.war.as_ref()?;
        war.rule.invasion.then_some(())?;
        war.winner.map(|n| 3 - n)
    }

    /// Living temple monsters, sampled before each tick.
    pub fn set_temple_alive(&mut self, alive: usize) {
        self.temple_alive = alive;
    }

    pub fn tick(&mut self, now: NaiveDateTime) -> Vec<Action> {
        let last = self.last.replace(now);
        // Missed ticks fire late; clock jumps over five minutes never replay.
        if let Some(last) = last.filter(|l| *l < now && now - *l <= Duration::minutes(5)) {
            for entry in self.schedule.clone() {
                self.fire(&entry, last, now);
            }
        }
        self.tick_temple(now);
        self.tick_war(now);
        self.tick_siege(now);
        self.take_actions()
    }

    fn fire(&mut self, entry: &Entry, last: NaiveDateTime, now: NaiveDateTime) {
        let today = now.date();
        for date in [today.pred_opt(), Some(today), today.succ_opt()]
            .into_iter()
            .flatten()
        {
            if !entry.days.is_empty() && !entry.days.contains(&date.weekday()) {
                continue;
            }
            let at = date.and_time(entry.time);
            let crossed = |offset: i64| {
                let t = at - Duration::seconds(offset);
                last < t && t <= now
            };
            match &entry.rule {
                Rule::Temple(rule) => {
                    if crossed(300) || crossed(20) {
                        self.push(notice(format!(
                            "The Forgotten Temple Event for level {}~{} will start at {}.",
                            rule.level_min,
                            rule.level_max,
                            entry.time.format("%H:%M")
                        )));
                    }
                    if crossed(0)
                        && let Err(error) = self.start_temple(rule.clone(), now)
                    {
                        self.log.push(format!("scheduled temple skipped: {error}"));
                    }
                }
                Rule::War(rule) => {
                    if crossed(300) {
                        self.push(notice(
                            "There are 5 minutes remaining until the war commences.",
                        ));
                    }
                    if crossed(0) {
                        let zone = rotation(rule, date);
                        if let Err(error) = self.start_war(entry.kind, rule.clone(), zone, now) {
                            self.log.push(format!("scheduled war skipped: {error}"));
                        }
                    }
                }
                Rule::Siege(rule) => {
                    if crossed(0)
                        && let Err(error) = self.start_siege(rule.clone(), now)
                    {
                        self.log.push(format!("scheduled siege skipped: {error}"));
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- temple

    pub fn start_temple(&mut self, rule: TempleRule, now: NaiveDateTime) -> Result<(), String> {
        if self.temple.is_some() {
            return Err("The Forgotten Temple is already running.".into());
        }
        let waves = self.waves.get(&rule.waves).cloned().unwrap_or_default();
        if waves.is_empty() {
            return Err("Forgotten Temple waves are not imported; run the database update.".into());
        }
        self.temple = Some(Temple {
            rule,
            start: now,
            waves,
            next: 0,
            next_summon: now + Duration::seconds(TEMPLE_FIRST_WAVE_S),
            phase: TemplePhase::Signup,
        });
        self.push(notice("The gate to the Forgotten Temple is now open."));
        Ok(())
    }

    fn tick_temple(&mut self, now: NaiveDateTime) {
        let Some(mut temple) = self.temple.take() else {
            return;
        };
        let elapsed = (now - temple.start).num_seconds();
        let mut keep = true;
        match temple.phase.clone() {
            TemplePhase::Signup => {
                if elapsed >= TEMPLE_SIGNUP_S {
                    temple.phase = TemplePhase::Waves;
                    self.push(notice("The Monsters are appearing, Get ready!!!"));
                }
            }
            TemplePhase::Waves => {
                while temple.next < temple.waves.len() && now >= temple.next_summon {
                    self.push(Action::SpawnWave(temple.waves[temple.next]));
                    temple.next += 1;
                    temple.next_summon += Duration::seconds(TEMPLE_WAVE_S);
                }
                let summoned = temple.next >= temple.waves.len() && now >= temple.next_summon;
                if summoned && self.temple_alive == 0 {
                    self.push(Action::Notice {
                        zone: Some(TEMPLE_ZONE),
                        nation: None,
                        text: "You have eliminated all the monsters in the temple. You've received the treasure chest.".into(),
                    });
                    self.push(Action::RewardZone {
                        zone: TEMPLE_ZONE,
                        items: vec![(temple.rule.chest, 1)],
                        source: "forgotten-temple",
                    });
                    temple.phase = TemplePhase::Cleared {
                        end: now + Duration::seconds(30),
                    };
                } else if elapsed >= TEMPLE_LIMIT_S {
                    temple.phase = TemplePhase::Failed {
                        notice: Some(now + Duration::seconds(30)),
                        end: now + Duration::seconds(230),
                    };
                }
            }
            TemplePhase::Cleared { end } => keep = now < end,
            TemplePhase::Failed { notice: at, end } => {
                if at.is_some_and(|t| now >= t) {
                    self.push(notice("Forgotten Temple is now over. Users remaining in the Forgotten Temple zone will be Teleported out."));
                    temple.phase = TemplePhase::Failed { notice: None, end };
                }
                keep = now < end;
            }
        }
        if keep {
            self.temple = Some(temple);
        } else {
            self.push(Action::Despawn(EventTag::Temple));
        }
    }

    // ------------------------------------------------------------------- war

    pub fn start_war(
        &mut self,
        kind: Kind,
        rule: WarRule,
        zone: u8,
        now: NaiveDateTime,
    ) -> Result<(), String> {
        if let Some(war) = &self.war {
            return Err(format!("The {} is already running.", war.kind.title()));
        }
        if !(61..=64).contains(&zone) {
            return Err("War zones are 61-64.".into());
        }
        let mut war = War {
            kind,
            zone,
            start: now,
            killed: [0; 2],
            deaths: [0; 2],
            monuments: HashMap::new(),
            points: [0; 2],
            winner: None,
            next_status: now,
            nereid_checked: false,
            half_checked: false,
            closing: None,
            rule,
        };
        war.next_status = now + war.status_interval();
        self.war = Some(war);
        self.push(notice(if kind == Kind::DarkLunar {
            "Dark Lunar Gate is now open. The war has begun."
        } else {
            "Lunar Gate is open. War has commenced."
        }));
        self.push(Action::ArmWar { zone });
        Ok(())
    }

    /// Initial monument owners of the armed war zone.
    pub fn set_monuments(&mut self, monuments: &[(u16, u8)]) {
        if let Some(war) = self.war.as_mut().filter(|w| w.zone == 64) {
            war.monuments = monuments.iter().copied().collect();
        }
    }

    pub fn add_deaths(&mut self, karus: u32, elmorad: u32) {
        if let Some(war) = self.war.as_mut() {
            war.deaths[0] += karus;
            war.deaths[1] += elmorad;
        }
    }

    /// A keeper or warder (zones 61-63) died; `npc_nation` owned it.
    pub fn war_npc_killed(
        &mut self,
        npc_nation: u8,
        prototype: u16,
        killer: &str,
        clan: &str,
        now: NaiveDateTime,
    ) {
        let Some(war) = self.war.as_mut() else {
            return;
        };
        if war.winner.is_some()
            || war.closing.is_some()
            || !(61..=63).contains(&war.zone)
            || !matches!(npc_nation, 1 | 2)
        {
            return;
        }
        let text = match prototype {
            11041 | 21041 => {
                format!(
                    "The user that killed the enemy's gatekeeper is [{clan}][{killer}]. Congratulations!!"
                )
            }
            21042 => format!(
                "The sentry at the 1st Karus fortress has been killed. The user that killed the sentry is [{clan}][{killer}]"
            ),
            21045 => format!(
                "The sentry at the 2nd Karus fortress has been killed. The user that killed the sentry is [{clan}][{killer}]"
            ),
            11042 => format!(
                "The sentry at the 1st El Morad fortress has been killed. The user that killed the sentry is [{clan}][{killer}]"
            ),
            11047 => format!(
                "The sentry at the 2nd El Morad fortress has been killed. The user that killed the sentry is [{clan}][{killer}]"
            ),
            _ => return,
        };
        war.killed[usize::from(npc_nation - 1)] += 1;
        let decisive = war.killed[usize::from(npc_nation - 1)] >= 3;
        self.push(notice(text));
        if decisive {
            self.war_result(3 - npc_nation, now);
        }
    }

    /// A Nereid monument changed hands in zone 64.
    pub fn monument_captured(&mut self, id: u16, nation: u8, name: &str) {
        let Some(war) = self.war.as_mut() else {
            return;
        };
        if war.zone != 64
            || war.winner.is_some()
            || war.closing.is_some()
            || !matches!(nation, 1 | 2)
        {
            return;
        }
        war.monuments.insert(id, nation);
        let index = usize::from(nation - 1);
        war.points[index] += 2;
        if war.monument_counts()[index] >= 7 {
            war.points[index] += 10;
        }
        let zone = Some(war.zone);
        self.push(Action::Notice {
            zone,
            nation: Some(nation),
            text: format!("You have occupied {name}."),
        });
        self.push(Action::Notice {
            zone,
            nation: Some(3 - nation),
            text: format!("{name} has been occupied by the opponent country."),
        });
    }

    fn war_result(&mut self, winner: u8, now: NaiveDateTime) {
        let Some(war) = self.war.as_mut() else {
            return;
        };
        if war.winner.is_some() || war.closing.is_some() {
            return;
        }
        war.winner = Some(winner);
        war.next_status = now + war.status_interval();
        let [karus_dead, elmorad_dead] = war.deaths;
        let (kind, zone, rule) = (war.kind, war.zone, war.rule.clone());
        let loser = 3 - winner;
        self.push(notice(if winner == 1 {
            format!("Karus is victorious. They killed {elmorad_dead} warriors and lost {karus_dead} warriors")
        } else {
            format!("El Morad is victorious. They killed {karus_dead} warriors and lost {elmorad_dead} warriors")
        }));
        self.push(notice(if loser == 1 {
            format!("Karus have lost the war. They killed [{elmorad_dead}] warriors and lost [{karus_dead}] warriors")
        } else {
            format!("El Morad have lost the war. They killed [{karus_dead}] warriors and lost [{elmorad_dead}] warriors")
        }));
        self.push(notice(
            "The users of the defeated nation in Lunar Valley will be teleported back to their native country.",
        ));
        let items: Vec<_> = [(NOAH, rule.victory_noah), (EXPERIENCE, rule.victory_exp)]
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .collect();
        if !items.is_empty() {
            self.push(Action::RewardNation {
                nation: winner,
                zones: vec![winner, zone],
                items,
                source: "war-victory",
            });
        }
        // The mid-level war closes on its result; the nation war opens the
        // loser's homeland to the winner until the war closes.
        if kind == Kind::DarkLunar {
            self.close_war(now);
        } else if rule.invasion {
            self.push(notice(
                "The victorious nation can invade the opposite nation.",
            ));
            self.push(notice(if loser == 1 {
                "Karus is being invaded."
            } else {
                "El Morad is being invaded."
            }));
            self.push(Action::Invasion {
                zone: loser,
                open: true,
            });
        }
    }

    fn close_war(&mut self, now: NaiveDateTime) {
        let Some(war) = self.war.as_mut() else {
            return;
        };
        if war.closing.is_some() {
            return;
        }
        war.closing = Some((
            Some(now + Duration::seconds(20)),
            now + Duration::seconds(BANISH_S),
        ));
        self.push(notice("Lunar Gate is closing."));
    }

    fn decide(war: &War) -> Option<u8> {
        let npc = compare(u64::from(war.killed[1]), u64::from(war.killed[0]));
        let kills = compare(u64::from(war.deaths[1]), u64::from(war.deaths[0]));
        if war.zone == 64 {
            compare(u64::from(war.points[0]), u64::from(war.points[1])).or(kills)
        } else {
            npc.or(kills)
        }
    }

    fn tick_war(&mut self, now: NaiveDateTime) {
        let Some(war) = self.war.as_mut() else {
            return;
        };
        if let Some((banish_notice, end)) = war.closing {
            if banish_notice.is_some_and(|t| now >= t) {
                war.closing = Some((None, end));
                self.push(notice("Users in the Lunar Valley and the enemy's territory will be teleported back to their own Nations."));
            }
            if now >= end {
                if let Some(zone) = self.invasion() {
                    self.push(Action::Invasion { zone, open: false });
                }
                self.war = None;
                self.push(Action::Despawn(EventTag::War));
            }
            return;
        }
        let elapsed = (now - war.start).num_seconds();
        if war.winner.is_none()
            && war.zone == 64
            && !war.nereid_checked
            && elapsed >= war.rule.duration_s / 8
        {
            war.nereid_checked = true;
            let [karus, elmorad] = war.monument_counts();
            let winner = if karus >= 7 && elmorad == 0 {
                Some(1)
            } else if elmorad >= 7 && karus == 0 {
                Some(2)
            } else {
                None
            };
            if let Some(winner) = winner {
                self.war_result(winner, now);
                return;
            }
        }
        let war = self.war.as_mut().unwrap();
        if war.winner.is_none() && !war.half_checked && elapsed >= war.rule.duration_s / 2 {
            war.half_checked = true;
            match Self::decide(war) {
                Some(winner) => self.war_result(winner, now),
                None => self.close_war(now),
            }
            return;
        }
        if now >= war.next_status {
            war.next_status = now + war.status_interval();
            let [karus_dead, elmorad_dead] = war.deaths;
            let text = match war.winner {
                None if war.zone == 64 => format!(
                    "Karus : {} Points, El Morad {} Points, Number of Dead Karus Users : {karus_dead}, Number of Dead El Morad Users : {elmorad_dead}",
                    war.points[0], war.points[1]
                ),
                None => format!(
                    "Number of dead Karus users: {karus_dead}, Number of dead El Morad users: {elmorad_dead}"
                ),
                Some(1) => "El Morad is being invaded.".into(),
                Some(_) => "Karus is being invaded.".into(),
            };
            self.push(notice(text));
        }
        if elapsed >= self.war.as_ref().unwrap().rule.duration_s {
            self.close_war(now);
        }
    }

    // ----------------------------------------------------------------- siege

    pub fn start_siege(&mut self, rule: SiegeRule, now: NaiveDateTime) -> Result<(), String> {
        if self.siege.is_some() {
            return Err("The Castle Siege War is already running.".into());
        }
        let battle_at = now + Duration::seconds(rule.countdown_s);
        self.siege = Some(Siege {
            battle_at,
            end_at: battle_at + Duration::seconds(rule.duration_s),
            warned: Vec::new(),
            battle: false,
        });
        self.push(notice(
            "Castle Siege War is about to begin. The designated Knights should enter the Castle Siege War zone.",
        ));
        let minutes = rule.countdown_s / 60;
        if minutes > 0 {
            self.push(notice(format!(
                "There are {minutes} minutes remaining until the Castle Siege War. Knights should enter the CSW now."
            )));
        }
        Ok(())
    }

    /// A clan member destroyed the Center Artifact.
    pub fn artifact_destroyed(&mut self, clan: u16, name: &str) {
        if !self.siege.as_ref().is_some_and(|s| s.battle) || clan == 0 {
            return;
        }
        self.castle_owner = Some((clan, name.to_owned()));
        self.push(Action::CastleOwner { clan });
        self.push(notice("Center Artifact has been destroyed"));
        self.push(notice(format!(
            "{name} clan has destroyed the Center Artifact."
        )));
    }

    fn tick_siege(&mut self, now: NaiveDateTime) {
        let owner = self.castle_owner.clone();
        let Some(siege) = self.siege.as_mut() else {
            return;
        };
        let mut pending = Vec::new();
        if !siege.battle {
            for minutes in [3, 2, 1] {
                if !siege.warned.contains(&minutes)
                    && now >= siege.battle_at - Duration::minutes(minutes)
                    && now < siege.battle_at
                {
                    siege.warned.push(minutes);
                    pending.push(notice(format!(
                        "There are {minutes} minutes remaining until the Castle Siege War. Knights should enter the CSW now."
                    )));
                }
            }
            if now >= siege.battle_at {
                siege.battle = true;
                pending.push(notice("The Castle Siege War has begun!"));
                pending.push(Action::SpawnArtifact {
                    owner: owner.as_ref().map_or(0, |(id, _)| *id),
                });
            }
        } else if now >= siege.end_at {
            self.siege = None;
            pending.push(notice("The Castle Siege War has ended!"));
            if let Some((_, name)) = owner {
                pending.push(notice(format!("The Knights {name} is victorious")));
            }
            pending.push(Action::Despawn(EventTag::Siege));
        }
        self.actions.extend(pending);
    }

    // ----------------------------------------------------------------- admin

    fn entry_for(&self, kind: Kind, arg: Option<u16>) -> Option<&Entry> {
        let matching = self.schedule.iter().filter(|e| e.kind == kind);
        let mut first = None;
        for entry in matching {
            first = first.or(Some(entry));
            match (&entry.rule, arg) {
                (Rule::Temple(r), Some(level))
                    if (u16::from(r.level_min)..=u16::from(r.level_max)).contains(&level) =>
                {
                    return Some(entry);
                }
                (Rule::War(r), Some(zone)) if r.zones.iter().any(|z| u16::from(*z) == zone) => {
                    return Some(entry);
                }
                _ => {}
            }
        }
        first
    }

    /// Stops an event at once: monsters despawn, nobody is rewarded, and
    /// players leave on their next tick.
    pub fn stop(&mut self, kind: Kind) -> Result<(), String> {
        if !self.running(kind) {
            return Err(format!("The {} is not running.", kind.title()));
        }
        match kind {
            Kind::Temple => {
                self.temple = None;
                self.push(Action::Despawn(EventTag::Temple));
            }
            Kind::Lunar | Kind::DarkLunar => {
                if let Some(zone) = self.invasion() {
                    self.push(Action::Invasion { zone, open: false });
                }
                self.war = None;
                self.push(Action::Despawn(EventTag::War));
                self.push(notice("Lunar Gate is closing."));
            }
            Kind::Siege => {
                self.siege = None;
                self.push(Action::Despawn(EventTag::Siege));
                self.push(notice("The Castle Siege War has ended!"));
            }
        }
        self.push(notice(format!(
            "The {} was stopped by an operator.",
            kind.title()
        )));
        Ok(())
    }

    /// Skips the current waiting phase (for testing and live operation).
    pub fn advance(&mut self, kind: Kind, now: NaiveDateTime) -> Result<String, String> {
        match kind {
            Kind::Temple => {
                let temple = self
                    .temple
                    .as_mut()
                    .ok_or("The Forgotten Temple is not running.")?;
                match temple.phase {
                    TemplePhase::Signup => {
                        temple.start = now - Duration::seconds(TEMPLE_SIGNUP_S);
                        temple.next_summon =
                            now + Duration::seconds(TEMPLE_FIRST_WAVE_S - TEMPLE_SIGNUP_S);
                        Ok("Sign-up closed; waves start shortly.".into())
                    }
                    TemplePhase::Waves => {
                        temple.next_summon = now;
                        Ok(format!(
                            "Next wave {} of {}.",
                            temple.next + 1,
                            temple.waves.len()
                        ))
                    }
                    _ => Err("The temple is already ending.".into()),
                }
            }
            Kind::Lunar | Kind::DarkLunar => {
                let war = self.war.as_mut().ok_or("No war is running.")?;
                if war.winner.is_some() || war.closing.is_some() {
                    return Err("The war already has a result.".into());
                }
                war.start = now - Duration::seconds(war.rule.duration_s / 2);
                war.nereid_checked = true;
                Ok("The war result is decided on the next tick.".into())
            }
            Kind::Siege => {
                let siege = self
                    .siege
                    .as_mut()
                    .ok_or("The Castle Siege War is not running.")?;
                if siege.battle {
                    return Err("The siege battle already began.".into());
                }
                let shift = siege.battle_at - now;
                siege.battle_at = now;
                siege.end_at -= shift;
                Ok("The siege battle begins on the next tick.".into())
            }
        }
    }

    /// Operator command text after `op event`. Returns reply lines.
    pub fn command(&mut self, text: &str, now: NaiveDateTime) -> Vec<String> {
        let mut words = text.split_whitespace();
        let verb = words.next().unwrap_or("status").to_ascii_lowercase();
        let name = words.next();
        let arg = words.next().and_then(|w| w.parse::<u16>().ok());
        let kind = name.and_then(Kind::parse);
        if name.is_some_and(|n| n.eq_ignore_ascii_case("bdw")) {
            return vec!["Border Defence War is pending owner rules.".into()];
        }
        let result = match (verb.as_str(), kind) {
            ("list", _) => return self.list(),
            ("status", _) => return self.status(now),
            ("start", Some(kind)) => self.admin_start(kind, arg, now),
            ("stop", Some(kind)) => self
                .stop(kind)
                .map(|()| format!("{} stopped.", kind.title())),
            ("advance", Some(kind)) => self.advance(kind, now),
            ("result", _) => self.admin_result(name, now),
            _ => Err("Use op event list|status, op event start|stop|advance temple|lunar|darklunar|siege [level|zone], or op event result karus|elmorad|draw.".into()),
        };
        vec![result.unwrap_or_else(|e| e)]
    }

    fn admin_start(
        &mut self,
        kind: Kind,
        arg: Option<u16>,
        now: NaiveDateTime,
    ) -> Result<String, String> {
        let entry = self
            .entry_for(kind, arg)
            .cloned()
            .ok_or_else(|| format!("No {} schedule row is configured.", kind.title()))?;
        match entry.rule {
            Rule::Temple(rule) => {
                let text = format!(
                    "Forgotten Temple started for levels {}-{}.",
                    rule.level_min, rule.level_max
                );
                self.start_temple(rule, now).map(|()| text)
            }
            Rule::War(rule) => {
                let zone = arg
                    .and_then(|z| u8::try_from(z).ok())
                    .filter(|z| rule.zones.contains(z))
                    .unwrap_or_else(|| rotation(&rule, now.date()));
                self.start_war(kind, rule, zone, now)
                    .map(|()| format!("{} started in zone {zone}.", kind.title()))
            }
            Rule::Siege(rule) => self
                .start_siege(rule, now)
                .map(|()| "Castle Siege War countdown started.".into()),
        }
    }

    fn admin_result(&mut self, name: Option<&str>, now: NaiveDateTime) -> Result<String, String> {
        let war = self.war.as_ref().ok_or("No war is running.")?;
        if war.winner.is_some() || war.closing.is_some() {
            return Err("The war already has a result.".into());
        }
        match name.map(str::to_ascii_lowercase).as_deref() {
            Some("karus") => self.war_result(1, now),
            Some("elmorad") => self.war_result(2, now),
            Some("draw") => self.close_war(now),
            _ => return Err("Use op event result karus|elmorad|draw.".into()),
        }
        Ok("War result applied.".into())
    }

    fn list(&self) -> Vec<String> {
        if self.schedule.is_empty() {
            return vec!["No events are scheduled.".into()];
        }
        self.schedule
            .iter()
            .map(|e| {
                let days = if e.days.is_empty() {
                    "daily".to_owned()
                } else {
                    e.days
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let detail = match &e.rule {
                    Rule::Temple(r) => {
                        format!("levels {}-{} chest {}", r.level_min, r.level_max, r.chest)
                    }
                    Rule::War(r) => format!(
                        "{} min zones {:?} levels {}-{}",
                        r.duration_s / 60,
                        r.zones,
                        r.level_min,
                        r.level_max
                    ),
                    Rule::Siege(r) => format!(
                        "countdown {} min, {} min",
                        r.countdown_s / 60,
                        r.duration_s / 60
                    ),
                };
                format!(
                    "#{} {} {} {} {}",
                    e.id,
                    e.kind.name(),
                    days,
                    e.time.format("%H:%M"),
                    detail
                )
            })
            .collect()
    }

    fn status(&self, now: NaiveDateTime) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(t) = &self.temple {
            let phase = match t.phase {
                TemplePhase::Signup => "sign-up",
                TemplePhase::Waves => "waves",
                TemplePhase::Cleared { .. } => "cleared",
                TemplePhase::Failed { .. } => "failed",
            };
            lines.push(format!(
                "Temple {}-{}: {phase}, stage {}/{}, {} alive, {} s elapsed.",
                t.rule.level_min,
                t.rule.level_max,
                t.next,
                t.waves.len(),
                self.temple_alive,
                (now - t.start).num_seconds()
            ));
        }
        if let Some(w) = &self.war {
            lines.push(format!(
                "{} zone {}: {} s, keepers killed K{}/E{}, dead K{}/E{}, points K{}/E{}, winner {}{}.",
                w.kind.title(),
                w.zone,
                (now - w.start).num_seconds(),
                w.killed[0],
                w.killed[1],
                w.deaths[0],
                w.deaths[1],
                w.points[0],
                w.points[1],
                w.winner.map_or("none", nation_name),
                if w.closing.is_some() { ", closing" } else { "" }
            ));
        }
        if let Some(s) = &self.siege {
            lines.push(format!(
                "Siege: {}, owner {}.",
                if s.battle {
                    format!("battle, {} s left", (s.end_at - now).num_seconds())
                } else {
                    format!("countdown, {} s left", (s.battle_at - now).num_seconds())
                },
                self.castle_owner
                    .as_ref()
                    .map_or("none", |(_, n)| n.as_str())
            ));
        }
        if lines.is_empty() {
            lines.push("No event is running.".into());
        }
        lines
    }
}

/// Weekly zone rotation by ISO week number.
fn rotation(rule: &WarRule, date: chrono::NaiveDate) -> u8 {
    rule.zones[date.iso_week().week() as usize % rule.zones.len()]
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
