//! Data-driven quest walkthroughs: native list entry, acceptance, objectives,
//! turn-in, rewards and replay, without levelling a character in-game.
use super::*;
use tokio::io::AsyncReadExt;

const ACCEPT: i32 = 2147483001;
const TURN_IN: i32 = 2147483002;

struct Harness {
    lobby: Lobby,
    s: Session,
    socket: TcpStream,
    peer: SocketAddr,
    all: Vec<Helper>,
}

/// Copies the imported 1453 quest tables, NPC population and town starts.
async fn harness() -> Option<Harness> {
    let path = std::path::Path::new("../../reko.db");
    if !path.exists() {
        return None;
    }
    let (mut lobby, s) = crate::world_service_tests::setup()?;
    let source =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    {
        let db = lobby.db.lock().unwrap();
        let mut q = source
            .prepare("SELECT table_name,rule_id,data FROM world_reference_rules WHERE table_name IN ('QUEST_HELPER','QUEST_MONSTER','ITEM_EXCHANGE')")
            .unwrap();
        for row in q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
        {
            let (t, id, data) = row.unwrap();
            db.execute(
                "INSERT INTO world_reference_rules VALUES(?1,?2,?3)",
                params![t, id, data],
            )
            .unwrap();
        }
    }
    lobby.merchants = crate::shops::load_population(&source, &reko_common::game_dir()).unwrap();
    let all = helpers(&lobby.db.lock().unwrap()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (socket, peer) = listener.accept().await.unwrap();
    // Drain native replies so long walkthroughs never block on socket buffers.
    tokio::spawn(async move {
        let mut buffer = [0u8; 8192];
        while client.read(&mut buffer).await.is_ok_and(|n| n > 0) {}
    });
    Some(Harness {
        lobby,
        s,
        socket,
        peer,
        all,
    })
}

/// Exchange kind, inputs and rewards as (item, count) pairs.
type ExchangeRow = (i64, Vec<(u32, u32)>, Vec<(u32, u32)>);
fn exchange_row(db: &Connection, id: u32) -> ExchangeRow {
    let row = exchange_rule(db, id).unwrap();
    let pairs = |item: &str, count: &str, n: usize| {
        (1..=n)
            .map(|i| {
                (
                    number(&row, &format!("{item}{i}")) as u32,
                    number(&row, &format!("{count}{i}")) as u32,
                )
            })
            .filter(|(id, count)| *id != 0 && *count != 0)
            .collect::<Vec<_>>()
    };
    (
        number(&row, "bRandomFlag"),
        pairs("nOriginItemNum", "nOriginItemCount", 11),
        pairs("nExchangeItemNum", "nExchangeItemCount", 5),
    )
}

impl Harness {
    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.lobby.db.lock().unwrap()
    }

    /// A fresh character of the given full class code and level.
    fn prepare(&mut self, class: u16, level: u8) {
        let nation = class / 100;
        self.s.character.class = class;
        self.s.character.race = if nation == 1 { 2 } else { 12 };
        self.s.progress.level = level;
        self.s.progress.points = [0; 9];
        self.s.progress.points[0] = 40;
        self.s.rewards = crate::quest_rewards::Totals {
            loyalty: 300,
            ..Default::default()
        };
        self.s.inventory = [starter::Item::default(); 28];
        self.s.coins = 0;
        self.s.npc_dialog = None;
        self.s.skills.hp = self.s.skills.max_hp.max(1);
        let db = self.lobby.db.lock().unwrap();
        db.execute_batch(
            "DELETE FROM character_quests; DELETE FROM character_quest_kills;
             DELETE FROM character_inventory; DELETE FROM character_quest_rewards;
             UPDATE character_wallets SET coins=0;",
        )
        .unwrap();
        db.execute(
            "UPDATE lobby_characters SET class=?1,race=?2 WHERE account='test' AND slot=0",
            params![class, self.s.character.race],
        )
        .unwrap();
        crate::progress::save(&db, "test", 0, &self.s.progress).unwrap();
    }

    fn wallet(&self) -> ([starter::Item; 28], u32, crate::quest_rewards::Totals) {
        let bag = starter::inventory(&self.db(), "test", 0).unwrap();
        (bag, self.count(900000000), self.s.rewards)
    }

    fn state(&self, flag: u16) -> u8 {
        self.db()
            .query_row(
                "SELECT COALESCE((SELECT state FROM character_quests WHERE account='test' AND slot=0 AND quest=?1),0)",
                [flag],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn count(&self, item: u32) -> u32 {
        if item == 900000000 {
            return starter::coins(&self.db(), "test", 0).unwrap();
        }
        starter::inventory(&self.db(), "test", 0)
            .unwrap()
            .iter()
            .filter(|i| i.id == item)
            .map(|i| u32::from(i.count))
            .sum()
    }

    /// Places drop items (and Noah) directly; quest products are earned instead.
    fn seed(&mut self, item: u32, count: u32) -> anyhow::Result<()> {
        let db = self.lobby.db.lock().unwrap();
        if item == 900000000 {
            db.execute(
                "UPDATE character_wallets SET coins=coins+?1 WHERE account='test' AND slot=0",
                [count],
            )?;
            self.s.coins = starter::coins(&db, "test", 0)?;
            return Ok(());
        }
        let c = self.lobby.combat.as_ref().unwrap();
        let mut bag = Changes {
            travel: None,
            rewards: crate::quest_rewards::State::new(&db, "test", &self.s)?,
            level_state: None,
            class: None,
            bag: starter::inventory(&db, "test", 0)?,
            coins: 0,
            quests: HashMap::new(),
            packets: Vec::new(),
            context: Context::default(),
        };
        bag.give(c, item, count)?;
        db.execute(
            "DELETE FROM character_inventory WHERE account='test' AND slot=0",
            [],
        )?;
        for (i, entry) in bag.bag.iter().enumerate() {
            if entry.id != 0 {
                db.execute(
                    "INSERT INTO character_inventory VALUES ('test',0,?1,?2,?3,?4)",
                    params![i, entry.id, entry.durability, entry.count],
                )?;
            }
        }
        self.s.inventory = bag.bag;
        Ok(())
    }

    async fn send(&mut self, packet: &[u8]) {
        self.s.combat.max_weight = u16::MAX;
        handle(
            &mut self.socket,
            self.peer,
            &self.lobby,
            "test",
            &mut self.s,
            packet,
        )
        .await;
    }

    async fn quest(&mut self, op: u8, helper: u32) {
        let mut p = vec![0x64, op];
        p.extend(helper.to_le_bytes());
        self.send(&p).await;
    }

    /// Replies to the open dialog with the menu slot bound to `event`.
    async fn choose(&mut self, event: i32) -> anyhow::Result<()> {
        let context = self
            .s
            .npc_dialog
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no dialog"))?;
        let index = context
            .choices
            .iter()
            .position(|c| *c == event)
            .ok_or_else(|| anyhow::anyhow!("menu lacks event {event}: {:?}", context.choices))?;
        let mut p = vec![0x55, index as u8, context.file.len() as u8];
        p.extend(context.file.as_bytes());
        self.send(&p).await;
        Ok(())
    }

    async fn click(&mut self, prototype: u16) -> anyhow::Result<()> {
        let merchant = self
            .lobby
            .merchants
            .iter()
            .find(|m| m.prototype == prototype)
            .ok_or_else(|| anyhow::anyhow!("NPC {prototype} is not populated"))?;
        let (id, position) = (merchant.id, merchant.position);
        self.s.position = position;
        let mut p = vec![0x20];
        p.extend(id.to_le_bytes());
        self.send(&p).await;
        Ok(())
    }

    fn go_to(&mut self, prototype: u16) -> anyhow::Result<()> {
        let merchant = self
            .lobby
            .merchants
            .iter()
            .find(|m| m.prototype == prototype)
            .ok_or_else(|| anyhow::anyhow!("NPC {prototype} is not populated"))?;
        self.s.position = merchant.position;
        Ok(())
    }

    /// The offer helper and the native list entry the client would send. Some
    /// quests (66, 67, 925) only define their offer as the status-4 helper.
    fn entry(&self, flag: u16) -> Option<(Helper, Helper)> {
        let offer = [0, 4].iter().find_map(|status| self.offer(flag, *status))?;
        let entry = self
            .all
            .iter()
            .find(|h| {
                h.flag == flag
                    && h.npc == offer.npc
                    && h.status == 255
                    && h.level <= self.s.progress.level
                    && eligible(h, &self.s)
            })
            .unwrap_or(offer);
        Some((offer.clone(), entry.clone()))
    }

    fn offer(&self, flag: u16, status: u8) -> Option<&Helper> {
        self.all.iter().find(|h| {
            h.flag == flag
                && h.status == status
                && h.level <= self.s.progress.level
                && offerable(h, &self.s)
                && self.lobby.merchants.iter().any(|m| m.prototype == h.npc)
                && database_quest_script(
                    &self.all,
                    &Context {
                        helper: h.id,
                        ..Context::default()
                    },
                )
                .is_some()
        })
    }

    /// Producer quests for an item, restricted to this character's variants.
    fn producer(&self, item: u32) -> Option<u16> {
        let db = self.db();
        [112, 113, 212, 213]
            .iter()
            .chain(REPEATABLE_QUESTS)
            .copied()
            .find(|flag| {
                self.entry(*flag).is_some_and(|(offer, _)| {
                    exchange_row(&db, offer.exchange)
                        .2
                        .iter()
                        .any(|(id, _)| *id == item)
                })
            })
    }

    /// Provides `count` of an item: crafted through its quest, otherwise seeded.
    fn provide<'a>(
        &'a mut self,
        item: u32,
        count: u32,
        depth: u8,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
            if depth > 0
                && item != 900000000
                && let Some(flag) = self.producer(item)
            {
                for _ in 0..count {
                    if self.count(item) >= count {
                        break;
                    }
                    self.walk(flag, depth - 1).await?;
                }
                anyhow::ensure!(
                    self.count(item) >= count,
                    "quest {flag} did not craft {item}"
                );
                return Ok(());
            }
            self.seed(item, count)
        })
    }

    /// Offer -> accept -> objectives -> turn-in for one quest flag.
    async fn walk(&mut self, flag: u16, depth: u8) -> anyhow::Result<u8> {
        let (offer, entry) = self
            .entry(flag)
            .ok_or_else(|| anyhow::anyhow!("no synthesized offer for this character"))?;
        let before = self.state(flag);
        anyhow::ensure!(matches!(before, 0 | 4), "quest already in state {before}");
        self.go_to(offer.npc)?;
        self.quest(3, entry.id).await;
        anyhow::ensure!(
            self.s
                .npc_dialog
                .as_ref()
                .is_some_and(|c| c.choices.contains(&ACCEPT)),
            "native list entry {} did not open the offer",
            entry.id
        );
        self.quest(6, entry.id).await;
        anyhow::ensure!(self.state(flag) == 1, "accept failed");
        if let Some(&(_, item)) = ACCEPT_ITEMS.iter().find(|(f, _)| *f == flag) {
            anyhow::ensure!(self.count(item) >= 1, "accept item {item} missing");
        }
        let (kind, origins, rewards) = exchange_row(&self.db(), offer.exchange);
        for &(item, count) in &origins {
            if matches!(item, 900005000 | 900008000) {
                let row: String = self.db().query_row(
                    "SELECT data FROM world_reference_rules WHERE table_name='QUEST_MONSTER' AND rule_id=?1",
                    [count],
                    |r| r.get(0),
                )?;
                let row: serde_json::Value = serde_json::from_str(&row)?;
                for group in 1..=4 {
                    let target = number(&row, &format!("sNum{group}a")) as u16;
                    for _ in 0..number(&row, &format!("sCount{group}")) {
                        let db = self.db();
                        if item == 900008000 {
                            let tx = db.unchecked_transaction()?;
                            crate::quests::player_kill(&tx, "test", 0, target as u8)?;
                            tx.commit()?;
                        } else {
                            crate::quests::kill(&db, "test", 0, target)?;
                        }
                    }
                }
            } else if !(item == 900000000 && self.count(item) >= count) {
                let have = self.count(item);
                if have < count {
                    self.provide(item, count, depth).await?;
                }
            }
        }
        let held: Vec<u32> = rewards.iter().map(|(id, _)| self.count(*id)).collect();
        let (xp, np) = (self.s.rewards.experience, self.s.rewards.loyalty);
        let level = self.s.progress.level;
        self.go_to(offer.npc)?;
        self.quest(3, entry.id).await;
        self.choose(TURN_IN).await?;
        let after = self.state(flag);
        let expected = if REPEATABLE_QUESTS.contains(&flag) {
            4
        } else {
            2
        };
        anyhow::ensure!(after == expected, "turn-in left state {after}");
        if matches!(kind, 0 | 10 | 11 | 12) {
            let paid = if kind == 12 {
                &rewards[..1]
            } else {
                &rewards[..]
            };
            for (i, &(item, count)) in paid.iter().enumerate() {
                match item {
                    900001000 => anyhow::ensure!(
                        self.s.rewards.experience != xp || self.s.progress.level != level,
                        "experience reward missing"
                    ),
                    900002000 | 900003000 => anyhow::ensure!(
                        self.s.rewards.loyalty == np.saturating_add(count).min(2100000000),
                        "national point reward mismatch"
                    ),
                    _ if MARKER_ITEMS.contains(&item) => {
                        anyhow::ensure!(self.count(item) == 0, "marker {item} entered the bag")
                    }
                    _ => {
                        let consumed = origins
                            .iter()
                            .filter(|(id, _)| *id == item)
                            .map(|(_, n)| *n)
                            .sum::<u32>();
                        let item = if let Some(c) = self.lobby.combat.as_ref()
                            && let Some(compatible) = skeleton_priest_reward(c, offer.exchange)
                            && item == 285002292
                        {
                            compatible
                        } else {
                            item
                        };
                        anyhow::ensure!(
                            self.count(item) + consumed >= held[i] + count,
                            "item reward {item} missing"
                        );
                    }
                }
            }
        }
        // Replaying the confirmation must not pay again.
        let snapshot = self.wallet();
        let _ = self.choose(TURN_IN).await;
        anyhow::ensure!(snapshot == self.wallet(), "replayed turn-in paid again");
        anyhow::ensure!(self.state(flag) == expected, "replay changed the state");
        Ok(after)
    }
}

/// Class code for a job group (1..4); novices for master promotion.
fn class_for(nation: u16, job: u16, novice: bool) -> u16 {
    nation * 100 + job * 2 + if novice { 3 } else { 4 }
}

#[tokio::test]
async fn every_synthesized_quest_walks_from_native_list_to_single_reward() {
    let Some(mut h) = harness().await else {
        return;
    };
    let mut variants = std::collections::BTreeSet::new();
    for helper in h
        .all
        .iter()
        .filter(|x| matches!(x.status, 0 | 4) && x.flag > 0)
    {
        // 406 and 603/605 have dedicated tests; unpopulated NPCs are listed gaps.
        if matches!(helper.flag, 406 | 603 | 605)
            || !h.lobby.merchants.iter().any(|m| m.prototype == helper.npc)
            || database_quest_script(
                &h.all,
                &Context {
                    helper: helper.id,
                    ..Context::default()
                },
            )
            .is_none()
        {
            continue;
        }
        let jobs = if (1..=4).contains(&helper.class) {
            vec![u16::from(helper.class)]
        } else {
            vec![1]
        };
        let nations = match helper.nation {
            1 | 2 => vec![u16::from(helper.nation)],
            _ => vec![1, 2],
        };
        for nation in nations {
            for &job in &jobs {
                variants.insert((helper.flag, nation, job, helper.promotion, helper.level));
            }
        }
    }
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for (flag, nation, job, novice, level) in variants {
        // One level above the helper avoids its same-level experience gate.
        h.prepare(
            class_for(nation, job, novice),
            level.saturating_add(1).clamp(1, 80),
        );
        let result = h.walk(flag, 0).await;
        if let Err(error) = &result {
            failures.push(format!("{flag}/{nation}/{job}: {error}"));
        }
        report.push(serde_json::json!({"flag":flag,"nation":nation,"job":job,"result":result.map_or_else(|e|e.to_string(),|s|format!("state {s}"))}));
    }
    std::fs::write(
        "../../logs/quest-walkthrough-audit.json",
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!("Quest walkthroughs: {} cases", report.len());
    assert!(report.len() > 150, "expected broad reference-data coverage");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn every_class_masters_and_unlocks_all_skill_quests_from_raw_drops() {
    let Some(mut h) = harness().await else {
        return;
    };
    let catalog = crate::skills::Catalog::load(&reko_common::game_dir()).unwrap();
    let skills: [&[u16]; 4] = [
        &[51, 510, 511],
        &[52, 512, 513, 514],
        &[53, 515, 516, 517],
        &[54, 518, 519, 520, 521, 522, 523],
    ];
    for nation in 1..=2u16 {
        for job in 1..=4u16 {
            let novice = class_for(nation, job, true);
            h.prepare(novice, 80);
            // A novice sees no skill-unlock offer and a master no promotion.
            assert!(h.entry(skills[usize::from(job - 1)][0]).is_none());
            h.walk(408, 3)
                .await
                .unwrap_or_else(|e| panic!("408 {novice}: {e}"));
            let master = novice + 1;
            assert_eq!(h.s.character.class, master);
            {
                let db = h.db();
                let saved: u16 = db
                    .query_row(
                        "SELECT class FROM lobby_characters WHERE account='test' AND slot=0",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(saved, master);
                assert!(crate::quests::completed(&db, "test", 0, 408).unwrap());
                assert!(crate::quests::completed(&db, "test", 0, job).unwrap());
                assert!(
                    crate::progress::allocate(&db, "test", 0, master, 8)
                        .unwrap()
                        .1,
                    "master points open after promotion"
                );
            }
            assert!(h.entry(408).is_none(), "masters cannot repeat promotion");
            for &flag in skills[usize::from(job - 1)] {
                h.walk(flag, 3)
                    .await
                    .unwrap_or_else(|e| panic!("{flag} class {master}: {e}"));
                assert!(crate::quests::completed(&h.db(), "test", 0, flag).unwrap());
                assert!(
                    catalog.spells.values().any(|s| s.required_quest == flag),
                    "quest {flag} gates native skills"
                );
            }
        }
    }
}

#[tokio::test]
async fn repeatable_ingredients_reoffer_and_pay_each_time() {
    let Some(mut h) = harness().await else {
        return;
    };
    h.prepare(class_for(2, 3, false), 70);
    for round in 1..=3 {
        assert_eq!(h.walk(118, 0).await.unwrap(), 4);
        assert_eq!(h.count(379236000), round);
    }
}

/// Events of a reference script whose block saves a quest state.
fn saving_events(file: &str) -> Vec<i32> {
    let path = reko_common::game_dir()
        .parent()
        .unwrap()
        .join("kn_2086_patch_private_server/lCAFILES/Quests")
        .join(file);
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.split("if (EVENT == ")
        .skip(1)
        .filter(|block| block.contains("SaveEvent"))
        .filter_map(|block| block.split(')').next()?.trim().parse().ok())
        .collect()
}

/// Clicks through a reference-script dialog. Riddle NPCs (the key sentinels)
/// offer a wrong answer that destroys the item, so a choice leading to a saved
/// state wins; otherwise the first (accept) choice is followed.
async fn click_through(h: &mut Harness, npc: u16, flag: u16, state: u8) -> anyhow::Result<()> {
    h.click(npc).await?;
    for _ in 0..4 {
        if h.state(flag) == state {
            return Ok(());
        }
        let context =
            h.s.npc_dialog
                .clone()
                .ok_or_else(|| anyhow::anyhow!("NPC {npc} has no dialog"))?;
        let saving = saving_events(&context.file);
        let next = context
            .choices
            .iter()
            .copied()
            .find(|c| saving.contains(c))
            .or_else(|| context.choices.first().copied())
            .filter(|c| *c >= 0)
            .ok_or_else(|| anyhow::anyhow!("NPC {npc} dialog ended early"))?;
        h.choose(next).await?;
    }
    anyhow::ensure!(
        h.state(flag) == state,
        "NPC {npc} did not save state {state}"
    );
    Ok(())
}

#[tokio::test]
async fn seven_key_suffering_and_reconnaissance_chains_complete_through_reference_npcs() {
    let Some(mut h) = harness().await else {
        return;
    };
    let scripts = reko_common::game_dir()
        .parent()
        .unwrap()
        .join("kn_2086_patch_private_server/lCAFILES/Quests");
    if !scripts.join("26051_key01.lua").exists() {
        return;
    }
    for nation in 1..=2u16 {
        let base = if nation == 1 { 26051 } else { 16051 };
        // Level-50 seven keys: quest 9 hands over the order, seven sentinels swap keys.
        h.prepare(class_for(nation, 1, false), 60);
        let (offer, entry) = h.entry(9).unwrap();
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.quest(6, entry.id).await;
        assert_eq!(h.state(9), 1);
        assert_eq!(h.count(910050000), 1);
        for step in 0..7u16 {
            click_through(&mut h, base + step, 416 + step, 4)
                .await
                .unwrap_or_else(|e| panic!("key {step}: {e}"));
        }
        assert_eq!(h.count(910057000), 1);
        assert_eq!(h.count(910050000), 0);
        let xp = h.s.rewards.experience;
        let level = h.s.progress.level;
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.choose(TURN_IN).await.unwrap();
        assert_eq!(h.state(9), 2);
        assert_eq!(h.count(910057000), 0);
        assert!(h.s.rewards.experience != xp || h.s.progress.level != level);

        // Quest 430: the Book of Suffering through seven certificates.
        h.prepare(class_for(nation, 2, false), 70);
        let (offer, entry) = h.entry(430).unwrap();
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.quest(6, entry.id).await;
        assert_eq!(h.count(910127000), 1);
        let watch = if nation == 1 { 24417 } else { 14417 };
        for step in 0..7u16 {
            click_through(&mut h, watch + step, 431 + step, 4)
                .await
                .unwrap_or_else(|e| panic!("certificate {step}: {e}"));
        }
        assert_eq!(h.count(910134000), 1);
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.choose(TURN_IN).await.unwrap();
        assert_eq!(h.state(430), 2);
        for item in [910139000, 910140000, 910141000, 910142000] {
            assert_eq!(h.count(item), 1);
        }

        // Quest 8: the order is exchanged by the Recon for its report.
        h.prepare(class_for(nation, 3, false), 30);
        let (offer, entry) = h.entry(8).unwrap();
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.quest(6, entry.id).await;
        assert_eq!(h.count(910044000), 1);
        let recon = if nation == 1 { 21051 } else { 11051 };
        let recon_flag = if nation == 1 { 237 } else { 137 };
        click_through(&mut h, recon, recon_flag, 2).await.unwrap();
        assert_eq!(h.count(910041000), 1);
        h.seed(910040000, 5).unwrap();
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.choose(TURN_IN).await.unwrap();
        assert_eq!(h.state(8), 2);
    }
}

#[tokio::test]
async fn opposing_nation_kills_complete_pvp_quests_and_own_nation_kills_do_not() {
    let Some(mut h) = harness().await else {
        return;
    };
    for (flag, nation) in [(438u16, 1u16), (439, 1), (442, 2), (443, 2)] {
        h.prepare(class_for(nation, 1, false), 70);
        let (offer, entry) = h.entry(flag).unwrap();
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.quest(6, entry.id).await;
        assert_eq!(h.state(flag), 1);
        {
            let db = h.db();
            let tx = db.unchecked_transaction().unwrap();
            let own = crate::quests::player_kill(&tx, "test", 0, nation as u8).unwrap();
            assert!(own.is_empty(), "own-nation kills never count");
            tx.commit().unwrap();
        }
        h.walk_kills(flag, 3 - nation as u8);
        let xp = h.s.rewards.experience;
        let level = h.s.progress.level;
        h.go_to(offer.npc).unwrap();
        h.quest(3, entry.id).await;
        h.choose(TURN_IN).await.unwrap();
        assert_eq!(h.state(flag), 2);
        assert!(h.s.rewards.experience != xp || h.s.progress.level != level);
    }
}

impl Harness {
    fn walk_kills(&self, flag: u16, victim: u8) {
        let required: u8 = self
            .db()
            .query_row(
                "SELECT CAST(json_extract(data,'$.sCount1') AS INTEGER) FROM world_reference_rules WHERE table_name='QUEST_MONSTER' AND rule_id=?1",
                [flag],
                |r| r.get(0),
            )
            .unwrap();
        for _ in 0..required {
            let db = self.db();
            let tx = db.unchecked_transaction().unwrap();
            assert!(
                !crate::quests::player_kill(&tx, "test", 0, victim)
                    .unwrap()
                    .is_empty()
            );
            tx.commit().unwrap();
        }
        assert_eq!(
            crate::quests::counts(&self.db(), "test", 0, flag).unwrap()[0],
            required
        );
    }
}

#[test]
fn every_quest_monster_target_is_spawned_or_a_player_nation() {
    let path = std::path::Path::new("../../reko.db");
    if !path.exists() {
        return;
    }
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut q = db
        .prepare("SELECT rule_id,data FROM world_reference_rules WHERE table_name='QUEST_MONSTER' AND rule_id IN (SELECT CAST(json_extract(data,'$.sEventDataIndex') AS INTEGER) FROM world_reference_rules WHERE table_name='QUEST_HELPER')")
        .unwrap();
    let mut missing = Vec::new();
    for row in q
        .query_map([], |r| Ok((r.get::<_, u16>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
    {
        let (quest, data) = row.unwrap();
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        for group in 1..=4 {
            if number(&v, &format!("sCount{group}")) == 0 {
                continue;
            }
            let targets: Vec<i64> = ['a', 'b', 'c', 'd']
                .iter()
                .map(|part| number(&v, &format!("sNum{group}{part}")))
                .filter(|n| *n != 0)
                .collect();
            let reachable = targets.iter().any(|&t| {
                matches!(t, 1 | 2)
                    || db
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM world_spawns WHERE prototype=?1)",
                            [t],
                            |r| r.get::<_, bool>(0),
                        )
                        .unwrap()
            });
            if !reachable {
                missing.push((quest, group, targets));
            }
        }
    }
    assert!(missing.is_empty(), "unspawned quest targets: {missing:?}");
}
