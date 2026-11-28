use super::*;
use chrono::NaiveDate;

fn date(weekday: Weekday) -> NaiveDate {
    let mut day = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
    while day.weekday() != weekday {
        day = day.succ_opt().unwrap();
    }
    day
}

fn at(weekday: Weekday, h: u32, m: u32, s: u32) -> NaiveDateTime {
    date(weekday).and_hms_opt(h, m, s).unwrap()
}

fn owner_schedule() -> Vec<Entry> {
    [
        r#"{"event":"temple","days":"daily","time":"22:15","level_min":60,"level_max":80,"chest":379156000,"waves":3,"fee":100000,"capacity":33}"#,
        r#"{"event":"temple","days":"daily","time":"23:30","level_min":45,"level_max":59,"chest":379155000,"waves":2,"fee":100000,"capacity":33}"#,
        r#"{"event":"lunar","days":["fri"],"time":"20:00","minutes":120,"zones":[61,62,64]}"#,
        r#"{"event":"darklunar","days":["thu"],"time":"20:00","minutes":120,"zones":[63],"level_min":35,"level_max":59,"invasion":false}"#,
        r#"{"event":"siege","days":["sun"],"time":"20:00","countdown_minutes":10,"minutes":60}"#,
    ]
    .iter()
    .enumerate()
    .map(|(i, row)| parse_entry(i as u32 + 1, row).unwrap().unwrap())
    .collect()
}

fn controller() -> Controller {
    let mut c = Controller {
        schedule: owner_schedule(),
        ..Controller::default()
    };
    for level in [2u8, 3] {
        c.waves.insert(
            level,
            (1..=56)
                .map(|stage| Wave {
                    stage,
                    sid: 1000 + u16::from(stage),
                    count: 2,
                    x: 126,
                    z: 126,
                    range: 45,
                })
                .collect(),
        );
    }
    c
}

/// Ticks once per second from `from` (exclusive) to `to` (inclusive).
fn run(c: &mut Controller, from: NaiveDateTime, to: NaiveDateTime) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut now = from;
    while now < to {
        now += Duration::seconds(1);
        actions.extend(c.tick(now));
    }
    actions
}

fn texts(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Notice { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn schedule_rows_parse_owner_data_and_reject_invalid_rows() {
    let schedule = owner_schedule();
    assert_eq!(
        schedule[1].time,
        NaiveTime::from_hms_opt(23, 30, 0).unwrap()
    );
    assert!(
        matches!(&schedule[1].rule, Rule::Temple(r) if r.level_min == 45 && r.chest == 379155000)
    );
    assert_eq!(schedule[2].days, vec![Weekday::Fri]);
    assert!(matches!(&schedule[3].rule, Rule::War(r) if r.level_max == 59 && !r.invasion));
    assert!(
        matches!(&schedule[2].rule, Rule::War(r) if r.invasion && r.np_kill == 64 && r.nation_cap == 150)
    );
    assert!(
        parse_entry(9, r#"{"event":"siege","time":"20:00","enabled":false}"#)
            .unwrap()
            .is_none()
    );
    for bad in [
        r#"{"event":"bdw","time":"20:00"}"#,
        r#"{"event":"lunar","time":"20:00","zones":[71]}"#,
        r#"{"event":"temple","time":"25:00"}"#,
        r#"{"event":"temple","time":"20:00","days":["someday"]}"#,
        r#"{"event":"temple","time":"20:00","level_min":70,"level_max":60}"#,
    ] {
        assert!(parse_entry(9, bad).is_err(), "{bad}");
    }
}

#[test]
fn waves_keep_the_first_row_of_each_stage_up_to_56() {
    let rows = (0..170u32).map(|i| {
        (
            i,
            serde_json::json!({"bLevel":3,"bStage":i / 3 + 1,"sSid":2000 + i,"sCount":1,"sPosX":126,"sPosZ":126,"bRange":45}),
        )
    });
    let waves = waves_from_rows(rows.rev());
    let list = &waves[&3];
    assert_eq!(list.len(), 56);
    assert_eq!((list[0].stage, list[0].sid), (1, 2000));
    assert_eq!((list[1].stage, list[1].sid), (2, 2003));
}

#[test]
fn scheduler_fires_on_local_time_without_replaying_startup_or_clock_jumps() {
    let mut c = controller();
    // The first tick only records the clock.
    assert!(c.tick(at(Weekday::Tue, 22, 15, 0)).is_empty());
    assert!(!c.running(Kind::Temple));
    let mut c = controller();
    c.tick(at(Weekday::Tue, 22, 9, 0));
    let actions = run(
        &mut c,
        at(Weekday::Tue, 22, 9, 0),
        at(Weekday::Tue, 22, 15, 0),
    );
    let notices = texts(&actions);
    assert_eq!(
        notices
            .iter()
            .filter(|t| t.contains("level 60~80 will start at 22:15"))
            .count(),
        2
    );
    assert!(c.running(Kind::Temple));
    assert_eq!(c.gates().temple.unwrap().level_min, 60);
    // A clock jump past the next start does not replay it.
    let mut c = controller();
    c.tick(at(Weekday::Tue, 23, 0, 0));
    assert!(c.tick(at(Weekday::Tue, 23, 40, 0)).is_empty());
    assert!(!c.running(Kind::Temple));
}

#[test]
fn weekly_wars_and_siege_start_only_on_their_days() {
    let mut c = controller();
    c.tick(at(Weekday::Wed, 19, 59, 0));
    run(
        &mut c,
        at(Weekday::Wed, 19, 59, 0),
        at(Weekday::Wed, 20, 1, 0),
    );
    assert!(c.gates().war.is_none() && c.gates().siege.is_none());
    let mut c = controller();
    c.tick(at(Weekday::Thu, 19, 54, 0));
    let actions = run(
        &mut c,
        at(Weekday::Thu, 19, 54, 0),
        at(Weekday::Thu, 20, 0, 0),
    );
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t.contains("5 minutes remaining"))
    );
    assert!(c.running(Kind::DarkLunar));
    assert_eq!(c.gates().war.unwrap().zone, 63);
    assert!(actions.contains(&Action::ArmWar { zone: 63 }));
    let mut c = controller();
    c.tick(at(Weekday::Fri, 19, 59, 59));
    c.tick(at(Weekday::Fri, 20, 0, 0));
    assert!(c.running(Kind::Lunar));
    let zone = c.gates().war.unwrap().zone;
    assert!([61, 62, 64].contains(&zone));
    let mut c = controller();
    c.tick(at(Weekday::Sun, 19, 59, 59));
    c.tick(at(Weekday::Sun, 20, 0, 0));
    assert_eq!(
        c.gates().siege,
        Some(SiegeGate {
            battle: false,
            owner: 0
        })
    );
}

#[test]
fn temple_signup_waves_clear_and_chest_follow_1453_timing() {
    let mut c = controller();
    let start = at(Weekday::Mon, 22, 15, 0);
    c.tick(start - Duration::seconds(1));
    c.tick(start);
    let gates = c.gates();
    assert!(gates.temple.as_ref().unwrap().entry_open);
    assert_eq!(gates.exit(TEMPLE_ZONE, 1, false), None);
    let actions = run(&mut c, start, start + Duration::seconds(300));
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t.contains("Monsters are appearing"))
    );
    assert!(
        !c.gates().temple.unwrap().entry_open,
        "sign-up closes with the first notice"
    );
    c.set_temple_alive(5);
    let actions = run(
        &mut c,
        start + Duration::seconds(300),
        start + Duration::seconds(374),
    );
    assert!(!actions.iter().any(|a| matches!(a, Action::SpawnWave(_))));
    let actions = run(
        &mut c,
        start + Duration::seconds(374),
        start + Duration::seconds(375 + 45 * 55),
    );
    let waves: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            Action::SpawnWave(w) => Some(w.stage),
            _ => None,
        })
        .collect();
    assert_eq!(waves, (1..=56).collect::<Vec<_>>());
    // Monsters still alive: no victory yet; the last stage needs 45 more seconds.
    let last = start + Duration::seconds(375 + 45 * 55);
    let actions = run(&mut c, last, last + Duration::seconds(60));
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::RewardZone { .. }))
    );
    c.set_temple_alive(0);
    let actions = run(
        &mut c,
        last + Duration::seconds(60),
        last + Duration::seconds(61),
    );
    assert!(actions.contains(&Action::RewardZone {
        zone: TEMPLE_ZONE,
        items: vec![(379156000, 1)],
        source: "forgotten-temple",
    }));
    let end = last + Duration::seconds(61);
    let actions = run(&mut c, end, end + Duration::seconds(30));
    assert!(actions.contains(&Action::Despawn(EventTag::Temple)));
    assert_eq!(c.gates().exit(TEMPLE_ZONE, 2, false), Some(Exit::Moradon));
    assert_eq!(c.gates().exit(TEMPLE_ZONE, 2, true), None, "admins stay");
}

#[test]
fn uncleared_temple_fails_without_chests_and_clears_the_zone() {
    let mut c = controller();
    let start = at(Weekday::Mon, 23, 30, 0);
    c.tick(start - Duration::seconds(1));
    c.tick(start);
    assert_eq!(c.gates().temple.unwrap().level_min, 45);
    c.set_temple_alive(3);
    let actions = run(&mut c, start, start + Duration::seconds(3000));
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::RewardZone { .. }))
    );
    assert!(c.running(Kind::Temple));
    let actions = run(
        &mut c,
        start + Duration::seconds(3000),
        start + Duration::seconds(3030),
    );
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t.contains("Forgotten Temple is now over"))
    );
    let actions = run(
        &mut c,
        start + Duration::seconds(3030),
        start + Duration::seconds(3230),
    );
    assert!(actions.contains(&Action::Despawn(EventTag::Temple)));
    assert!(!c.running(Kind::Temple));
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::RewardZone { .. }))
    );
}

#[test]
fn three_keeper_kills_win_the_war_open_invasion_and_banish_losers() {
    let mut c = controller();
    let start = at(Weekday::Fri, 20, 0, 0);
    let rule = match &c.schedule[2].rule {
        Rule::War(r) => r.clone(),
        _ => unreachable!(),
    };
    c.start_war(Kind::Lunar, rule.clone(), 61, start).unwrap();
    assert!(
        c.start_war(Kind::Lunar, rule, 62, start).is_err(),
        "one war at a time"
    );
    c.take_actions();
    let gates = c.gates();
    assert_eq!(gates.exit(61, 1, false), None);
    assert_eq!(gates.exit(62, 1, false), Some(Exit::Home));
    assert_eq!(
        gates.exit(72, 1, false),
        Some(Exit::Home),
        "Ardream closes in war"
    );
    assert_eq!(gates.exit(1, 2, false), Some(Exit::Home), "no invasion yet");
    assert_eq!(gates.war_np(61), Some((64, 50)));
    // Two El Morad NPCs and one Karus NPC are not decisive.
    c.war_npc_killed(2, 11042, "Kar", "Clan", start);
    c.war_npc_killed(1, 21042, "Elmo", "Other", start);
    c.war_npc_killed(2, 11047, "Kar", "Clan", start);
    assert!(c.gates().war.unwrap().loser.is_none());
    c.war_npc_killed(2, 11041, "Kar", "Clan", start);
    let actions = c.take_actions();
    let notices = texts(&actions);
    assert!(
        notices
            .iter()
            .any(|t| t.contains("gatekeeper is [Clan][Kar]"))
    );
    assert!(notices.iter().any(|t| t.starts_with("Karus is victorious")));
    assert!(actions.contains(&Action::Invasion {
        zone: 2,
        open: true
    }));
    assert!(actions.contains(&Action::RewardNation {
        nation: 1,
        zones: vec![1, 61],
        items: vec![(NOAH, 100000), (EXPERIENCE, 5000)],
        source: "war-victory",
    }));
    let gates = c.gates();
    assert_eq!(gates.exit(61, 2, false), Some(Exit::Home), "losers leave");
    assert_eq!(gates.exit(61, 1, false), None);
    assert_eq!(gates.exit(2, 1, false), None, "winners may invade");
    assert_eq!(gates.invasion, Some(2));
    assert!(gates.war_entry(61, 2, 60, 0).is_err());
    // The war still closes on time, then clears all zones after the banish delay.
    let end = start + Duration::seconds(7200);
    let actions = run(&mut c, start, end);
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::Invasion { open: false, .. }))
    );
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t == "Lunar Gate is closing.")
    );
    assert_eq!(
        c.gates().exit(2, 1, false),
        None,
        "invaders leave after the banish delay"
    );
    assert_eq!(c.gates().exit(61, 1, false), None, "banish delay");
    assert!(
        c.gates().war_entry(61, 1, 60, 0).is_err(),
        "no entry while closing"
    );
    let actions = run(&mut c, end, end + Duration::seconds(60));
    assert!(actions.contains(&Action::Invasion {
        zone: 2,
        open: false
    }));
    assert!(actions.contains(&Action::Despawn(EventTag::War)));
    assert_eq!(c.gates(), Gates::default());
    assert_eq!(Gates::default().exit(61, 1, false), Some(Exit::Home));
}

#[test]
fn half_time_decides_by_kills_or_draws_and_dark_lunar_closes_on_result() {
    let mut c = controller();
    let start = at(Weekday::Thu, 20, 0, 0);
    let rule = match &c.schedule[3].rule {
        Rule::War(r) => r.clone(),
        _ => unreachable!(),
    };
    c.start_war(Kind::DarkLunar, rule.clone(), 63, start)
        .unwrap();
    let gates = c.gates();
    assert_eq!(gates.war.as_ref().unwrap().lockout, vec![72]);
    assert_eq!(
        gates.war_entry(63, 1, 60, 0),
        Err("Your level does not qualify for this war.")
    );
    assert!(gates.war_entry(63, 1, 50, 149).is_ok());
    assert_eq!(
        gates.war_entry(63, 1, 50, 150),
        Err("Your nation's war zone is full.")
    );
    c.add_deaths(3, 5);
    let actions = run(&mut c, start, start + Duration::seconds(3600));
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t.starts_with("Karus is victorious"))
    );
    assert!(
        !actions.iter().any(|a| matches!(a, Action::Invasion { .. })),
        "no invasion"
    );
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t == "Lunar Gate is closing.")
    );
    assert_eq!(c.gates().invasion, None);
    // A tie on keepers and deaths is a draw that closes the war.
    let mut c = controller();
    c.start_war(Kind::DarkLunar, rule, 63, start).unwrap();
    let actions = run(&mut c, start, start + Duration::seconds(3600));
    assert!(!texts(&actions).iter().any(|t| t.contains("victorious")));
    assert!(!c.gates().war.unwrap().entry_open);
}

#[test]
fn nereid_monuments_score_and_win_early_with_seven() {
    let mut c = controller();
    let start = at(Weekday::Fri, 20, 0, 0);
    let rule = match &c.schedule[2].rule {
        Rule::War(r) => r.clone(),
        _ => unreachable!(),
    };
    c.start_war(Kind::Lunar, rule, 64, start).unwrap();
    let monuments: Vec<(u16, u8)> = (0..7)
        .map(|i| (14156 + i, [2, 2, 2, 4, 1, 1, 1][i as usize]))
        .collect();
    c.set_monuments(&monuments);
    for id in [14156, 14157, 14158, 14159] {
        c.monument_captured(id, 1, "a Nereid Monument");
    }
    let actions = c.take_actions();
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t == "You have occupied a Nereid Monument.")
    );
    let actions = run(&mut c, start, start + Duration::seconds(900));
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t.starts_with("Karus is victorious"))
    );
    assert_eq!(c.gates().invasion, Some(2));
}

#[test]
fn siege_countdown_battle_artifact_owner_and_end() {
    let mut c = controller();
    c.castle_owner = Some((7, "Old".into()));
    let start = at(Weekday::Sun, 20, 0, 0);
    c.start_siege(
        SiegeRule {
            countdown_s: 600,
            duration_s: 3600,
        },
        start,
    )
    .unwrap();
    assert_eq!(c.gates().exit(DELOS, 1, false), Some(Exit::Moradon));
    let actions = run(&mut c, start, start + Duration::seconds(599));
    assert_eq!(
        texts(&actions)
            .iter()
            .filter(|t| t.contains("minutes remaining until the Castle Siege"))
            .count(),
        4,
        "10, 3, 2 and 1 minutes"
    );
    c.artifact_destroyed(9, "Early");
    assert!(c.take_actions().is_empty(), "no capture before the battle");
    let actions = run(
        &mut c,
        start + Duration::seconds(599),
        start + Duration::seconds(600),
    );
    assert!(actions.contains(&Action::SpawnArtifact { owner: 7 }));
    assert!(c.gates().siege_battle());
    assert_eq!(c.gates().exit(DELOS, 1, false), None);
    c.artifact_destroyed(9, "Winners");
    let actions = c.take_actions();
    assert!(actions.contains(&Action::CastleOwner { clan: 9 }));
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t == "Winners clan has destroyed the Center Artifact.")
    );
    let actions = run(
        &mut c,
        start + Duration::seconds(600),
        start + Duration::seconds(4200),
    );
    assert!(
        texts(&actions)
            .iter()
            .any(|t| t == "The Knights Winners is victorious")
    );
    assert!(actions.contains(&Action::Despawn(EventTag::Siege)));
    assert_eq!(c.castle_owner, Some((9, "Winners".into())));
}

#[test]
fn operator_commands_start_stop_advance_and_report() {
    let mut c = controller();
    let now = at(Weekday::Mon, 10, 0, 0);
    assert!(c.command("start temple 50", now)[0].contains("levels 45-59"));
    assert!(c.command("start temple", now)[0].contains("already running"));
    assert!(c.command("advance temple", now)[0].contains("Sign-up closed"));
    c.tick(now);
    c.tick(now + Duration::seconds(1));
    assert!(!c.gates().temple.unwrap().entry_open);
    assert!(c.command("status", now)[0].starts_with("Temple 45-59"));
    assert!(c.command("stop temple", now)[0].contains("stopped"));
    assert!(
        c.take_actions()
            .contains(&Action::Despawn(EventTag::Temple))
    );
    assert!(c.command("start lunar 62", now)[0].contains("zone 62"));
    assert!(c.command("start darklunar", now)[0].contains("already running"));
    assert_eq!(c.command("result karus", now)[0], "War result applied.");
    assert!(c.take_actions().contains(&Action::Invasion {
        zone: 2,
        open: true
    }));
    assert!(c.command("stop lunar", now)[0].contains("stopped"));
    assert!(c.take_actions().contains(&Action::Invasion {
        zone: 2,
        open: false
    }));
    assert!(c.command("start siege", now)[0].contains("countdown"));
    c.command("advance siege", now);
    c.tick(now + Duration::seconds(1));
    assert!(c.gates().siege_battle());
    assert!(c.command("start bdw", now)[0].contains("pending"));
    assert_eq!(c.command("list", now).len(), 5);
    assert!(c.command("dance", now)[0].starts_with("Use op event"));
}

#[test]
fn temple_entry_checks_window_capacity_and_fee() {
    let gate = TempleGate {
        level_min: 45,
        level_max: 59,
        fee: 100000,
        capacity: 33,
        entry_open: true,
    };
    use crate::event_runtime::temple_entry;
    assert_eq!(temple_entry(None, 0, 1_000_000), (1, 0));
    assert_eq!(temple_entry(Some(&gate), 33, 1_000_000), (2, 0));
    assert_eq!(temple_entry(Some(&gate), 32, 99_999), (3, 0));
    assert_eq!(temple_entry(Some(&gate), 32, 100_000), (0, 100_000));
    let lobby = crate::social::tests::lobby();
    let s = crate::social::tests::player(&lobby, "templelow", 1, 44);
    lobby.social.lock().unwrap().events.gates.temple = Some(gate.clone());
    assert!(
        crate::event_runtime::temple_window(&lobby, &s).0.is_none(),
        "level 44 is outside 45-59"
    );
    let mut s = crate::social::tests::player(&lobby, "templeok", 2, 45);
    assert_eq!(
        crate::event_runtime::temple_window(&lobby, &s).0,
        Some(gate)
    );
    s.position.zone = TEMPLE_ZONE;
    crate::social::synchronize(&lobby, "templeok", &mut s);
    let other = crate::social::tests::player(&lobby, "templetwo", 2, 50);
    assert_eq!(crate::event_runtime::temple_window(&lobby, &other).1, 1);
}

#[test]
fn operator_event_commands_require_an_admin_character() {
    let lobby = crate::social::tests::lobby();
    *lobby.events.lock().unwrap() = controller();
    let s = crate::social::tests::player(&lobby, "eventop", 1, 80);
    let reply =
        crate::operator_chat::command(&lobby, "eventop", &s, b"op event start temple 70").unwrap();
    assert!(String::from_utf8_lossy(&reply).contains("administrator"));
    assert!(lobby.social.lock().unwrap().events.gates.temple.is_none());
    lobby
        .db
        .lock()
        .unwrap()
        .execute("INSERT INTO character_privileges VALUES('eventop',0,1)", [])
        .unwrap();
    let reply =
        crate::operator_chat::command(&lobby, "eventop", &s, b"op event start temple 70").unwrap();
    assert!(String::from_utf8_lossy(&reply).contains("levels 60-80"));
    assert_eq!(
        lobby
            .social
            .lock()
            .unwrap()
            .events
            .gates
            .temple
            .as_ref()
            .unwrap()
            .level_min,
        60
    );
}

#[test]
fn sessions_leave_closed_event_zones_and_war_invasion_changes_hostility() {
    let lobby = crate::social::tests::lobby();
    lobby
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "INSERT INTO world_town_starts VALUES(21,306,352,310,354,0,0,'test');
             INSERT INTO world_town_starts VALUES(1,441,1625,441,1625,0,0,'test');
             INSERT INTO world_town_starts VALUES(2,219,1859,219,1859,0,0,'test');",
        )
        .unwrap();
    // Logging in inside the temple without a running event moves to Moradon.
    let mut s = crate::social::tests::player(&lobby, "templeleft", 1, 60);
    s.skills.hp = 100;
    s.position = crate::movement::Position {
        zone: TEMPLE_ZONE,
        x: 1500,
        z: 1500,
        y: 0,
    };
    let packets = crate::event_runtime::session_tick(&lobby, "templeleft", &mut s).unwrap();
    assert_eq!(packets[0][..3], [0x27, 3, 21]);
    assert_eq!(
        crate::movement::load(&lobby.db.lock().unwrap(), "templeleft", 0)
            .unwrap()
            .zone,
        21
    );
    // Invasion: El Morad may fight Karus inside Luferson only while it is open.
    let mut karus = crate::social::tests::player(&lobby, "defender", 1, 60);
    let mut elmo = crate::social::tests::player(&lobby, "invader", 2, 60);
    for (name, p) in [("defender", &mut karus), ("invader", &mut elmo)] {
        p.skills.hp = 100;
        p.position = crate::movement::Position {
            zone: 1,
            x: 5000,
            z: 5000,
            y: 0,
        };
        crate::social::synchronize(&lobby, name, p);
    }
    let hub = || lobby.social.lock().unwrap();
    assert!(!crate::pvp::is_hostile(&hub(), &elmo, &karus));
    assert_eq!(
        crate::event_runtime::session_tick(&lobby, "invader", &mut elmo.clone()).unwrap()[0][..3],
        [0x27, 3, 2],
        "uninvited invaders go home"
    );
    hub().events.gates.invasion = Some(1);
    assert!(crate::pvp::is_hostile(&hub(), &elmo, &karus));
    assert!(
        crate::event_runtime::session_tick(&lobby, "invader", &mut elmo)
            .unwrap()
            .is_empty()
    );
    // Castle Siege battle: different clans fight in Delos.
    for p in [&mut karus, &mut elmo] {
        p.position = crate::movement::Position {
            zone: DELOS,
            x: 5000,
            z: 6000,
            y: 0,
        };
    }
    karus.clan.id = 3;
    elmo.clan.id = 4;
    assert!(!crate::pvp::is_hostile(&hub(), &elmo, &karus));
    hub().events.gates.siege = Some(SiegeGate {
        battle: true,
        owner: 0,
    });
    assert!(crate::pvp::is_hostile(&hub(), &elmo, &karus));
    elmo.clan.id = 3;
    assert!(!crate::pvp::is_hostile(&hub(), &elmo, &karus));
}

#[test]
fn full_bag_chest_waits_in_the_save_and_arrives_after_relog_when_space_frees() {
    let Some((lobby, mut s)) = crate::world_service_tests::setup() else {
        return;
    };
    let filler = 110110001u32;
    let blue = 379156000u32;
    let catalog = lobby.combat.as_ref().unwrap();
    if catalog.item(blue).is_none() || catalog.item(filler).is_none() {
        return;
    }
    {
        let db = lobby.db.lock().unwrap();
        for slot in 0..28 {
            db.execute(
                "INSERT INTO character_inventory(account,slot,inventory_slot,item_id,durability,count) VALUES('test',0,?1,?2,1,1)",
                rusqlite::params![slot, filler],
            )
            .unwrap();
        }
        s.inventory = crate::starter::inventory(&db, "test", 0).unwrap();
        crate::event_runtime::queue_rewards(
            &db,
            &[("test".into(), 0, s.character.name.clone())],
            &[(blue, 1), (NOAH, 5000)],
            "forgotten-temple",
        )
        .unwrap();
    }
    let coins = s.coins;
    let (packets, remaining, blocked) =
        crate::event_runtime::deliver(&lobby, "test", &mut s).unwrap();
    assert!(remaining && blocked);
    assert!(String::from_utf8_lossy(packets.last().unwrap()).contains("stays reserved"));
    assert!(!s.inventory.iter().any(|i| i.id == blue));
    assert_eq!(
        s.coins, coins,
        "later rewards wait behind the blocked chest"
    );
    // Relog: the pending reward is still in the save.
    let pending = crate::event_runtime::pending_names(&lobby.db.lock().unwrap()).unwrap();
    assert!(pending.contains(&s.character.name.to_ascii_lowercase()));
    {
        let db = lobby.db.lock().unwrap();
        db.execute(
            "DELETE FROM character_inventory WHERE account='test' AND slot=0 AND inventory_slot=5",
            [],
        )
        .unwrap();
        s.inventory = crate::starter::inventory(&db, "test", 0).unwrap();
    }
    let (packets, remaining, blocked) =
        crate::event_runtime::deliver(&lobby, "test", &mut s).unwrap();
    assert!(!remaining && !blocked);
    assert_eq!(s.inventory[5].id, blue);
    assert_eq!(s.coins, coins + 5000);
    assert!(
        packets
            .iter()
            .any(|p| String::from_utf8_lossy(p).contains("treasure chest was delivered"))
    );
    let db = lobby.db.lock().unwrap();
    assert_eq!(
        crate::starter::inventory(&db, "test", 0).unwrap()[5].id,
        blue
    );
    assert_eq!(crate::starter::coins(&db, "test", 0).unwrap(), coins + 5000);
    assert!(crate::event_runtime::pending_names(&db).unwrap().is_empty());
}

fn world_fixture(lobby: &crate::lobby::Lobby) -> Option<crate::monsters::World> {
    let game = reko_common::game_dir();
    if !game.join("Data/NPC_Looks.tbl").exists() {
        return None;
    }
    let mut world = crate::monsters::World::load(&game).unwrap();
    world.social = Some(lobby.social.clone());
    Some(world)
}

fn spawn(id: u16, nation: u8, zone: u8) -> crate::population::Spawn {
    let position = crate::movement::Position {
        zone,
        x: 3000,
        z: 3000,
        y: 0,
    };
    crate::population::Spawn {
        id,
        monster: true,
        position,
        prototype: 100,
        appearance: 100,
        name: "Event Target".into(),
        kind: 11,
        group: 0,
        nation,
        level: 80,
        size: 100,
        weapons: [0; 2],
        direction: 0,
        bounds: [3000, 3000, 3000, 3000],
        hp: 100,
        armor: 0,
        resistance: [0; 6],
        respawn_ms: 1000,
        regen_type: 1,
        move_type: 0,
    }
}

fn strike() -> crate::monsters::Attack {
    crate::monsters::Attack {
        position: crate::movement::Position {
            zone: 64,
            x: 3000,
            z: 3000,
            y: 0,
        },
        range: 5.0,
        attack: 0,
        percent: 100,
        extra: 1000,
        skill: true,
        drain_percent: 0,
        arrow: false,
        penetrate: false,
        magic: None,
    }
}

#[test]
fn temporary_ids_are_reused_after_death_and_event_despawn() {
    let lobby = crate::social::tests::lobby();
    let Some(world) = world_fixture(&lobby) else {
        return;
    };
    let reserved = Default::default();
    let first = world
        .summon_with(
            spawn(0, 0, 64),
            &reserved,
            Some(EventTag::Temple),
            None,
            true,
        )
        .unwrap();
    assert_eq!(world.event_alive(EventTag::Temple), 1);
    assert!(
        world
            .hit_owned(first, strike(), Some("Nobody"), || Ok(()))
            .unwrap()
            .dead
    );
    assert_eq!(world.event_alive(EventTag::Temple), 0);
    let second = world
        .summon_with(
            spawn(0, 0, 64),
            &reserved,
            Some(EventTag::Temple),
            None,
            true,
        )
        .unwrap();
    assert_ne!(first, second, "a fresh corpse keeps its ID");
    world.tick(crate::skills::now_ms() + 11_000);
    let third = world
        .summon_with(
            spawn(0, 0, 64),
            &reserved,
            Some(EventTag::Temple),
            None,
            true,
        )
        .unwrap();
    assert_eq!(third, first, "the ID returns to the pool after the delay");
    assert_eq!(world.despawn_event(EventTag::Temple), 2);
    assert_eq!(world.event_alive(EventTag::Temple), 0);
    let mut events = world.events.subscribe();
    let fourth = world
        .summon_with(spawn(0, 0, 64), &reserved, None, None, true)
        .unwrap();
    assert_eq!(fourth, first);
    assert_eq!(events.try_recv().unwrap().packet[..2], [0x0a, 1]);
}

#[test]
fn armed_war_npcs_respect_nation_and_monuments_change_hands() {
    let lobby = crate::social::tests::lobby();
    let Some(world) = world_fixture(&lobby) else {
        return;
    };
    let mut karus = crate::social::tests::player(&lobby, "keeperk", 1, 80);
    let mut elmo = crate::social::tests::player(&lobby, "keepere", 2, 80);
    for (name, p) in [("keeperk", &mut karus), ("keepere", &mut elmo)] {
        p.position.zone = 64;
        crate::social::synchronize(&lobby, name, p);
    }
    let armed = world.arm(vec![spawn(12500, 2, 64)], EventTag::War, true);
    assert_eq!(armed, vec![(12500, 2)]);
    assert!(world.armed_ids().contains(&12500));
    assert!(
        world
            .hit_owned(12500, strike(), Some("keepere"), || Ok(()))
            .is_err(),
        "own monument"
    );
    assert!(world.hit_owned(12500, strike(), None, || Ok(())).is_err());
    world
        .hit_owned(12500, strike(), Some("keeperk"), || Ok(()))
        .unwrap();
    let deaths = world.take_event_deaths();
    assert_eq!(deaths[0].captured_by, Some(1));
    assert_eq!(deaths[0].nation, 2);
    assert_eq!(world.event_alive(EventTag::War), 1, "captured, not dead");
    assert!(
        world
            .hit_owned(12500, strike(), Some("keeperk"), || Ok(()))
            .is_err()
    );
    assert!(
        world
            .hit_owned(12500, strike(), Some("keepere"), || Ok(()))
            .is_ok(),
        "recapture"
    );
    // A keeper dies once and reports its killer.
    world.arm(vec![spawn(12501, 1, 64)], EventTag::War, false);
    world.take_event_deaths();
    assert!(
        world
            .hit_owned(12501, strike(), Some("keeperk"), || Ok(()))
            .is_err()
    );
    assert!(
        world
            .hit_owned(12501, strike(), Some("keepere"), || Ok(()))
            .unwrap()
            .dead
    );
    let deaths = world.take_event_deaths();
    assert_eq!((deaths[0].id, deaths[0].nation), (12501, 1));
    assert_eq!(deaths[0].killer.as_deref(), Some("keepere"));
    assert_eq!(world.despawn_event(EventTag::War), 2);
    assert!(world.armed_ids().is_empty());
    // The siege artifact only takes damage from clan members outside the owner.
    let id = world
        .summon_with(
            spawn(0, 3, 64),
            &Default::default(),
            Some(EventTag::Siege),
            Some(crate::monsters::Restriction::Clan { owner: 5 }),
            false,
        )
        .unwrap();
    assert!(
        world
            .hit_owned(id, strike(), Some("keeperk"), || Ok(()))
            .is_err(),
        "clanless"
    );
    karus.clan.id = 5;
    lobby.social.lock().unwrap().synchronize("keeperk", &karus);
    assert!(
        world
            .hit_owned(id, strike(), Some("keeperk"), || Ok(()))
            .is_err(),
        "owner clan"
    );
    elmo.clan.id = 6;
    lobby.social.lock().unwrap().synchronize("keepere", &elmo);
    assert!(
        world
            .hit_owned(id, strike(), Some("keepere"), || Ok(()))
            .unwrap()
            .dead
    );
    assert_eq!(world.take_event_deaths()[0].tag, EventTag::Siege);
}

#[test]
fn controller_loads_migrated_schedule_waves_and_castle_owner() {
    let lobby = crate::social::tests::lobby();
    let db = lobby.db.lock().unwrap();
    for (id, row) in [
        (
            1,
            r#"{"capacity": 33, "chest": 379156000, "days": "daily", "enabled": true, "event": "temple", "fee": 100000, "level_max": 80, "level_min": 60, "time": "22:15", "waves": 3}"#,
        ),
        (
            3,
            r#"{"days": ["fri"], "enabled": true, "event": "lunar", "invasion": true, "level_max": 83, "level_min": 35, "minutes": 120, "nation_cap": 150, "np_death": 50, "np_kill": 64, "time": "20:00", "victory_exp": 5000, "victory_noah": 100000, "zones": [61, 62, 64]}"#,
        ),
        (
            5,
            r#"{"countdown_minutes": 10, "days": ["sun"], "enabled": true, "event": "siege", "minutes": 60, "time": "20:00"}"#,
        ),
        (6, r#"{"event": "nonsense", "time": "20:00"}"#),
    ] {
        db.execute(
            "INSERT INTO world_reference_rules VALUES('EVENT_SCHEDULE',?1,?2)",
            rusqlite::params![id, row],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO world_reference_rules VALUES('MONSTER_CHALLENGE_SUMMON_LIST',0,?1)",
        [r#"{"bLevel": 3, "bRange": 45, "bStage": 1, "bStageLevel": 1, "sCount": 30, "sIndex": 0, "sPosX": 126, "sPosZ": 126, "sSid": 2903, "sTime": 310}"#],
    )
    .unwrap();
    db.execute_batch(
        "INSERT INTO clans VALUES(4,'Keepers',1); INSERT INTO castle_siege_owner VALUES(1,4,'test');",
    )
    .unwrap();
    let c = crate::event_runtime::load(&db).unwrap();
    assert_eq!(c.schedule.len(), 3, "invalid rows are skipped");
    assert_eq!(c.waves[&3][0].sid, 2903);
    assert_eq!(c.castle_owner, Some((4, "Keepers".into())));
}

#[test]
fn monuments_never_die_when_the_top_damager_went_offline() {
    let lobby = crate::social::tests::lobby();
    let Some(world) = world_fixture(&lobby) else {
        return;
    };
    let mut karus = crate::social::tests::player(&lobby, "leaverk", 1, 80);
    let mut elmo = crate::social::tests::player(&lobby, "finishe", 2, 80);
    for (name, p) in [("leaverk", &mut karus), ("finishe", &mut elmo)] {
        p.position.zone = 64;
        lobby.social.lock().unwrap().synchronize(name, p);
    }
    // Neutral monument (nation 4): anyone may attack it.
    world.arm(vec![spawn(12600, 4, 64)], EventTag::War, true);
    let partial = |extra| crate::monsters::Attack { extra, ..strike() };
    world
        .hit_owned(12600, partial(90), Some("leaverk"), || Ok(()))
        .unwrap();
    lobby.social.lock().unwrap().disconnect(karus.player_id);
    world
        .hit_owned(12600, partial(50), Some("finishe"), || Ok(()))
        .unwrap();
    let deaths = world.take_event_deaths();
    assert_eq!(
        deaths[0].captured_by,
        Some(2),
        "falls back to the last attacker"
    );
    assert_eq!(world.event_alive(EventTag::War), 1);
    assert!(
        world
            .hit_owned(12600, partial(50), Some("finishe"), || Ok(()))
            .is_err(),
        "now El Morad's"
    );
}
